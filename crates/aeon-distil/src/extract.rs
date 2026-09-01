//! Turning turns into candidates, without a model.
//!
//! These are the rules that make commitment 2 affordable. A distiller does this better; these
//! do it always. Three paths across the gate are reachable from a transcript alone:
//!
//! * **SAID** — an imperative in a user turn. Crosses alone.
//! * **FIX** — a turn that corrects what just happened. Crosses alone.
//! * **SCAR** — something that cost work to learn: a call that failed and then succeeded, or a
//!   file read again and again. This is the signal a coding agent gets for free and that no
//!   general memory framework looks for.

use crate::{Candidate, Observation, Role, instruction};
use aeon_model::{Body, Importance, NoteKind, Tier, WitnessKind};
use std::collections::HashMap;

/// What a pass over one session produced.
#[derive(Debug, Default)]
pub struct Extracted {
    /// Everything proposed, in the order it was seen.
    pub candidates: Vec<Candidate>,
}

/// How many preceding turns a repair rule may look back over.
const LOOKBACK: usize = 6;

/// How many times a file must be read before it is worth remembering that it matters.
const RE_READS: u32 = 3;

/// How long a command must take before it is worth warning the next session about.
const SLOW_MS: u64 = 30_000;

/// Run every extractive rule over one session's turns.
///
/// `imperatives` is the configuration's list of words that mark a turn as an instruction.
#[must_use]
pub fn extract(turns: &[Observation], imperatives: &[String]) -> Extracted {
    let mut out = Extracted::default();
    let mut reads: HashMap<String, u32> = HashMap::new();

    for (index, turn) in turns.iter().enumerate() {
        match turn.role {
            Role::User => {
                if let Some(said) = instruction::read(&turn.text, imperatives) {
                    out.candidates
                        .extend(asked_for(&said, turn, &turns[..index]));
                } else if let Some(claim) = correction(&turn.text) {
                    out.candidates.push(
                        Candidate::new(
                            Body::note(claim, NoteKind::Claim),
                            Tier::Fact,
                            WitnessKind::Correction,
                            "correction",
                        )
                        .at(turn.cursor),
                    );
                }
            }
            Role::Tool => {
                let before = &turns[index.saturating_sub(LOOKBACK)..index];
                if let Some(candidate) = repair(turn, before) {
                    out.candidates.push(candidate);
                }
                if let Some(candidate) = slow(turn) {
                    out.candidates.push(candidate);
                }
                if let Some(candidate) = re_read(turn, &mut reads) {
                    out.candidates.push(candidate);
                }
            }
            Role::Assistant => {}
        }
    }
    out
}

/// What an instruction asks for.
///
/// Two shapes, and the second is the one people actually type. **"remember: we use make"**
/// carries its claim, and one candidate comes out of it. **"REMEMBER THIS, I have told you
/// four times"** carries none — what it means is whatever the session was just doing, so the
/// preceding turns become the candidates.
///
/// Insisting raises what comes out rather than lowering it. Somebody who has been asked the
/// same question repeatedly is telling you the agent has already failed at this, which is a
/// better reason to keep something than a calm first mention.
fn asked_for(
    said: &instruction::Instruction,
    turn: &Observation,
    before: &[Observation],
) -> Vec<Candidate> {
    let make = |text: String, cursor: Option<u64>| {
        let candidate = Candidate::new(
            Body::note(text, NoteKind::Claim),
            Tier::Fact,
            WitnessKind::Imperative,
            if said.insistent {
                "insisted"
            } else {
                "imperative"
            },
        )
        .at(cursor)
        .fading(Importance::High);
        if said.insistent {
            candidate.pinned()
        } else {
            candidate
        }
    };

    if let Some(claim) = &said.claim {
        return vec![make(claim.clone(), turn.cursor)];
    }

    // Referential. "Remember THIS" names nothing, so what it means has to be found — and it is
    // right behind it. Taking the recent substantive turns is what a person means by "this",
    // and it is the difference between storing the word "this" and storing the thing.
    recent_substance(before)
        .into_iter()
        .map(|(text, cursor)| make(text, cursor))
        .collect()
}

/// How far back "this" reaches.
///
/// Enough to catch the exchange somebody is pointing at, short enough not to sweep in whatever
/// they were doing before it.
const POINTS_BACK: usize = 4;

/// The recent turns worth treating as what "this" meant.
///
/// Assistant prose and successful tool calls: what the agent said, and what worked. Not the
/// person's own turns — somebody pointing at the conversation means what the *agent* did, not
/// a replay of what they themselves asked for.
fn recent_substance(before: &[Observation]) -> Vec<(String, Option<u64>)> {
    let mut out = Vec::new();
    for turn in before.iter().rev().take(POINTS_BACK * 2) {
        let worth = match turn.role {
            Role::Assistant => !turn.text.trim().is_empty(),
            Role::Tool => turn.worked() && turn.command().is_some(),
            Role::User => false,
        };
        if !worth {
            continue;
        }
        let text = match turn.role {
            Role::Tool => format!(
                "`{}` is what works here",
                turn.command().unwrap_or_default()
            ),
            _ => first_sentence(&turn.text),
        };
        if text.len() >= 8 {
            out.push((text, turn.cursor));
        }
        if out.len() >= POINTS_BACK {
            break;
        }
    }
    out.reverse();
    out
}

/// A claim that replaces what just happened.
///
/// The most information-dense turn in a coding session is the one that starts "no, ". It
/// carries both a refutation and a replacement, and the replacement is what is worth keeping.
#[must_use]
pub fn correction(text: &str) -> Option<String> {
    const OPENERS: &[&str] = &["no, ", "no — ", "not ", "actually, ", "wrong, ", "don't "];
    let trimmed = text.trim_start();
    let lower = trimmed.to_lowercase();
    if !OPENERS.iter().any(|o| lower.starts_with(o)) {
        return None;
    }
    let claim = first_sentence(trimmed);
    (claim.len() >= 8).then_some(claim)
}

/// A call that failed and then succeeded is a habit worth keeping.
///
/// The canonical case, and the one to demo: `cargo test` fails because this repository needs
/// `make test`, and `make test` works. One habit, learned once, worth more than a hundred
/// sentences of prose — and today every harness throws it away when the session ends.
fn repair(worked: &Observation, before: &[Observation]) -> Option<Candidate> {
    if !worked.worked() {
        return None;
    }
    let tool = worked.tool.as_deref()?;
    let succeeded = worked.command()?;

    let failed = before
        .iter()
        .rev()
        .find(|earlier| earlier.failed() && earlier.tool.as_deref() == Some(tool))?;
    let attempted = failed.command()?;
    if attempted == succeeded {
        return None;
    }

    Some(
        Candidate::new(
            Body::habit(
                format!("when `{attempted}` is what you would reach for"),
                vec![succeeded.to_owned()],
            ),
            Tier::Habit,
            WitnessKind::Cost,
            "repair",
        )
        .at(worked.cursor)
        .fading(Importance::High),
    )
}

/// A command nobody wants to rediscover the hard way.
fn slow(turn: &Observation) -> Option<Candidate> {
    let ms = turn.ms?;
    if !turn.worked() || ms < SLOW_MS {
        return None;
    }
    let command = turn.command()?;
    Some(
        Candidate::new(
            Body::fact(
                "project",
                "slow_command",
                format!("{command} (~{}s)", ms / 1000),
            ),
            Tier::Fact,
            WitnessKind::Cost,
            "slow-command",
        )
        .at(turn.cursor),
    )
}

/// The same file, read again and again, is the session saying what it is about.
fn re_read(turn: &Observation, seen: &mut HashMap<String, u32>) -> Option<Candidate> {
    if !turn.worked() {
        return None;
    }
    let path = turn.path()?;
    let count = seen.entry(path.to_owned()).or_default();
    *count += 1;
    // Once, exactly, at the threshold. Proposing it again on every subsequent read would put
    // one session's emphasis in as several witnesses, which is what diversity exists to stop.
    if *count != RE_READS {
        return None;
    }
    Some(
        Candidate::new(
            Body::fact("project", "central_file", path),
            Tier::Fact,
            WitnessKind::Cost,
            "re-read",
        )
        .at(turn.cursor),
    )
}

/// The first sentence of a claim, so a paragraph does not become one fact.
///
/// A full stop only ends a sentence when something follows it that is not more of the same
/// token. `10.0.0.7`, `1.2` and `fly.io` are one word each, and splitting on every dot stored
/// the staging box as being "at 10".
fn first_sentence(text: &str) -> String {
    let bytes: Vec<char> = text.chars().collect();
    for (i, c) in bytes.iter().enumerate() {
        let ends = match c {
            '\n' | '!' | '?' => true,
            // A dot ends a sentence only at the end of the text or before whitespace. Inside
            // a run of non-space characters it is part of a number, a host or a filename.
            '.' => bytes.get(i + 1).is_none_or(|next| next.is_whitespace()),
            _ => false,
        };
        if ends {
            return bytes[..i].iter().collect::<String>().trim().to_owned();
        }
    }
    text.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markers() -> Vec<String> {
        ["remember", "always", "never", "from now on", "note that"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    }

    fn tool(name: &str, command: &str, ok: bool) -> Observation {
        Observation {
            role: Role::Tool,
            tool: Some(name.to_owned()),
            args: Some(serde_json::json!({ "command": command })),
            ok: Some(ok),
            ..Observation::default()
        }
    }

    fn user(text: &str) -> Observation {
        Observation {
            role: Role::User,
            text: text.to_owned(),
            ..Observation::default()
        }
    }

    #[test]
    fn an_instruction_that_carries_its_claim_keeps_it() {
        let out = extract(&[user("remember: we deploy with fly")], &markers());
        assert_eq!(out.candidates.len(), 1);
        assert_eq!(out.candidates[0].text(), "we deploy with fly");
        assert!(!out.candidates[0].pinned, "nobody was insisting");
    }

    #[test]
    fn somebody_insisting_gets_a_memory_that_will_not_fade() {
        // A person who has been asked the same thing repeatedly is not asking for a memory
        // that decays. Their annoyance is part of the evidence.
        let out = extract(&[user("DUUUDE REMEBER we use make test!!")], &markers());
        let kept = out.candidates.first().expect("an instruction");
        assert!(kept.pinned, "it was shouted");
        assert_eq!(kept.importance, Importance::Critical);
        assert_eq!(kept.from, "insisted");
    }

    #[test]
    fn pointing_at_the_conversation_keeps_what_the_conversation_did() {
        // The shape people actually type. "REMEMBER THIS" names nothing, so what it means is
        // right behind it — and storing the word "this" would be worse than storing nothing.
        let turns = vec![
            user("how do I run the tests"),
            tool("shell", "cargo test", false),
            tool("shell", "make test", true),
            user("DUUUUDE FUKING REMEBER THIS SHIT COS YOU ARE ASKIG ME 100 TIMES"),
        ];
        let out = extract(&turns, &markers());
        let texts: Vec<String> = out.candidates.iter().map(Candidate::text).collect();
        assert!(
            texts.iter().any(|t| t.contains("make test")),
            "what actually worked was not kept: {texts:?}"
        );
        assert!(
            out.candidates.iter().any(|c| c.pinned),
            "somebody was plainly insisting"
        );
    }

    #[test]
    fn pointing_backwards_does_not_replay_the_persons_own_turns() {
        // Somebody pointing at the conversation means what the *agent* did, not a repeat of
        // what they themselves just asked for.
        let turns = vec![user("what is the port"), user("REMEMBER THAT")];
        let out = extract(&turns, &markers());
        assert!(
            !out.candidates
                .iter()
                .any(|c| c.text().contains("what is the port")),
            "{:?}",
            out.candidates
                .iter()
                .map(Candidate::text)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_aside_about_forgetting_is_not_an_instruction() {
        for aside in [
            "I can never remember which flag it is",
            "does it always fail like that?",
        ] {
            assert!(
                extract(&[user(aside)], &markers()).candidates.is_empty(),
                "{aside}"
            );
        }
    }

    #[test]
    fn a_correction_is_recognised_by_how_it_opens() {
        assert_eq!(
            correction("no, this project uses make").as_deref(),
            Some("no, this project uses make")
        );
        assert_eq!(correction("that looks right to me"), None);
    }

    #[test]
    fn a_failure_then_a_success_becomes_a_habit() {
        // The canonical case, and the one to demo.
        let turns = vec![
            user("run the tests"),
            tool("shell", "cargo test", false),
            tool("shell", "make test", true),
        ];
        let out = extract(&turns, &markers());
        let habit = out
            .candidates
            .iter()
            .find(|c| c.tier == Tier::Habit)
            .expect("a habit was learned");
        assert_eq!(habit.witness, WitnessKind::Cost);
        assert!(habit.text().contains("make test"), "{}", habit.text());
        assert!(habit.text().contains("cargo test"), "{}", habit.text());
    }

    #[test]
    fn a_command_that_worked_first_time_teaches_nothing() {
        let turns = vec![user("run the tests"), tool("shell", "make test", true)];
        assert!(extract(&turns, &markers()).candidates.is_empty());
    }

    #[test]
    fn retrying_the_identical_command_teaches_nothing() {
        // A flake is not a lesson.
        let turns = vec![
            tool("shell", "make test", false),
            tool("shell", "make test", true),
        ];
        assert!(extract(&turns, &markers()).candidates.is_empty());
    }

    #[test]
    fn a_failure_in_a_different_tool_is_not_the_repair_of_this_one() {
        let turns = vec![
            tool("read", "cargo test", false),
            tool("shell", "make test", true),
        ];
        assert!(extract(&turns, &markers()).candidates.is_empty());
    }

    #[test]
    fn a_slow_command_is_worth_warning_the_next_session_about() {
        let mut slow_one = tool("shell", "make dist", true);
        slow_one.ms = Some(45_000);
        let out = extract(&[slow_one], &markers());
        assert_eq!(out.candidates.len(), 1);
        assert!(
            out.candidates[0].text().contains("45s"),
            "{}",
            out.candidates[0].text()
        );
    }

    #[test]
    fn a_quick_command_is_not() {
        let mut quick = tool("shell", "ls", true);
        quick.ms = Some(12);
        assert!(extract(&[quick], &markers()).candidates.is_empty());
    }

    #[test]
    fn a_file_read_three_times_is_proposed_once() {
        // One session's emphasis must arrive as one witness, not as several. That is what
        // diversity exists to protect.
        let read = Observation {
            role: Role::Tool,
            tool: Some("read".into()),
            args: Some(serde_json::json!({ "path": "src/lib.rs" })),
            ok: Some(true),
            ..Observation::default()
        };
        let turns = vec![read.clone(), read.clone(), read.clone(), read.clone(), read];
        let out = extract(&turns, &markers());
        assert_eq!(out.candidates.len(), 1);
        assert!(out.candidates[0].text().contains("src/lib.rs"));
    }

    #[test]
    fn nothing_worth_keeping_produces_nothing() {
        let turns = vec![
            user("what does this do?"),
            Observation {
                role: Role::Assistant,
                text: "it does a thing".into(),
                ..Observation::default()
            },
        ];
        assert!(extract(&turns, &markers()).candidates.is_empty());
    }
}
