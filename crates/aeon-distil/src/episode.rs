//! What an episode was about, pulled out of the turns it spans.
//!
//! Segmentation says where an episode starts and stops; this says what happened inside it. Both
//! are rules, and this one is deliberately extractive — it quotes the transcript rather than
//! describing it, because a summary that paraphrases can be wrong in ways nobody notices, and a
//! quotation can only be irrelevant.
//!
//! A model does this better and is welcome to, through the existing distiller boundary. What is
//! here is the floor: with no model at all, an episode still says what it was for, what was
//! tried, where it turned, and what was left undone.

use crate::observation::{Observation, Role};

/// What one episode was.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Told {
    /// What was being attempted, in the person's own words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// What was already true when it began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting_state: Option<String>,
    /// What it is about.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<String>,
    /// What was tried, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<String>,
    /// The moment it changed direction — a failure followed by something that worked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turning_point: Option<String>,
    /// How it ended.
    pub outcome: aeon_model::Outcome,
    /// What was still open when it stopped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<String>,
    /// The cursors it covers.
    pub span: (u64, u64),
    /// Which rules produced this.
    pub method: &'static str,
}

/// Read an episode out of the turns it spans.
///
/// Everything is quoted or counted; nothing is invented. A field with no evidence is `None`,
/// which is a better answer than a plausible sentence nobody can check.
#[must_use]
pub fn tell(turns: &[Observation]) -> Told {
    let span = (
        turns.first().and_then(|t| t.cursor).unwrap_or(0),
        turns.last().and_then(|t| t.cursor).unwrap_or(0),
    );
    if turns.is_empty() {
        return Told {
            method: crate::METHOD,
            ..Told::default()
        };
    }

    // The first substantive thing a person said. What an episode is *for* is almost always the
    // request that opened it, and asking a model to infer it is asking it to re-read the turn.
    let goal = turns
        .iter()
        .find(|t| t.role == Role::User && substantive(&t.text))
        .map(|t| clip(&t.text));

    // What was already true: the first tool result before anybody tried to change anything.
    let starting_state = turns
        .iter()
        .take_while(|t| !t.failed())
        .find(|t| t.worked() && substantive(&t.text))
        .map(|t| clip(&t.text));

    let attempts: Vec<String> = turns
        .iter()
        .filter(|t| t.role == Role::Tool)
        .filter_map(command)
        .collect();

    // Where it turned: the first failure with a different success after it. That pair is the
    // most instructive thing in most episodes and the thing a fixed window most often splits.
    let turning_point = turns.iter().enumerate().find_map(|(at, turn)| {
        if !turn.failed() {
            return None;
        }
        let broke = command(turn)?;
        let fixed = turns
            .iter()
            .skip(at + 1)
            .find(|later| later.worked() && command(later).is_some_and(|c| c != broke))
            .and_then(command)?;
        Some(format!("{broke} failed; {fixed} worked"))
    });

    // How it ended, from the last tool that said either way. A run that ended mid-flight is
    // open, which is different from having failed.
    let outcome = match turns.iter().rev().find(|t| t.ok.is_some()) {
        Some(last) if last.worked() => aeon_model::Outcome::Done,
        Some(_) => aeon_model::Outcome::Failed,
        None => aeon_model::Outcome::Open,
    };

    // Left open: anything that failed and was never followed by something that worked.
    let unresolved: Vec<String> = turns
        .iter()
        .enumerate()
        .filter(|(_, t)| t.failed())
        .filter(|(at, _)| !turns.iter().skip(at + 1).any(Observation::worked))
        .filter_map(|(_, t)| command(t))
        .collect();

    let mut entities: Vec<String> = turns
        .iter()
        .flat_map(|t| aeon_store::entities_in(&t.text))
        .map(|e| e.display)
        .chain(attempts.iter().cloned())
        .collect();
    entities.sort();
    entities.dedup();
    entities.truncate(12);

    Told {
        goal,
        starting_state,
        entities,
        attempts,
        turning_point,
        outcome,
        unresolved,
        span,
        method: crate::METHOD,
    }
}

/// The command a tool turn ran, when it names one.
fn command(turn: &Observation) -> Option<String> {
    turn.args
        .as_ref()
        .and_then(|a| a.get("command"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// Whether a turn says anything worth quoting.
fn substantive(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.len() > 3 && trimmed.split_whitespace().count() > 1
}

/// A quotation, bounded.
fn clip(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= 120 {
        return trimmed.to_owned();
    }
    trimmed.chars().take(117).collect::<String>() + "…"
}

/// A narrow avoidance, when an episode ends with something that failed and stayed failed.
///
/// The most dangerous memory to get wrong. "Do not do X" as a general rule is almost always
/// false — X failed *under conditions*, and a prohibition that cannot name them is a
/// superstition an agent will obey forever.
///
/// So this refuses more than it produces. It needs a command that failed, an observed failure,
/// and a condition to attach it to; without all three there is nothing worth keeping, and
/// `None` is the answer rather than a vague warning.
#[must_use]
pub fn avoidance(told: &Told, condition: &str) -> Option<aeon_model::Avoidance> {
    // Only from an episode that actually ended badly. A failure that was repaired is a repair,
    // and the positive habit is the better memory.
    if told.outcome != aeon_model::Outcome::Failed {
        return None;
    }
    let rejected = told.unresolved.first()?.clone();
    if condition.trim().is_empty() {
        return None;
    }

    let held = aeon_model::Avoidance {
        rejected,
        when: condition.to_owned(),
        observed: told
            .goal
            .clone()
            .unwrap_or_else(|| "it did not work".to_owned()),
        // Only if something verified is actually known. Inventing a replacement would be the
        // worst of both: a prohibition and a guess.
        instead: told
            .attempts
            .iter()
            .find(|c| !told.unresolved.contains(c))
            .cloned(),
    };
    held.is_narrow().then_some(held)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::Kind;

    const NOW: aeon_model::Timestamp = 1_756_000_000;

    fn user(cursor: u64, text: &str) -> Observation {
        Observation {
            cursor: Some(cursor),
            role: Role::User,
            text: text.to_owned(),
            at: Some(NOW),
            ..Observation::default()
        }
    }

    fn tool(cursor: u64, command: &str, ok: bool) -> Observation {
        Observation {
            cursor: Some(cursor),
            role: Role::Tool,
            kind: Kind::ToolResult,
            tool: Some("shell".to_owned()),
            args: Some(serde_json::json!({ "command": command })),
            ok: Some(ok),
            text: if ok { "ok" } else { "no such command" }.to_owned(),
            at: Some(NOW),
            ..Observation::default()
        }
    }

    fn a_repair() -> Vec<Observation> {
        vec![
            user(0, "get the tests passing"),
            tool(1, "cargo test", false),
            tool(2, "make test", true),
        ]
    }

    #[test]
    fn the_goal_is_what_was_asked_rather_than_a_paraphrase() {
        // Quoted, not described. A paraphrase can be wrong in ways nobody notices; a quotation
        // can only be irrelevant.
        let held = tell(&a_repair());
        assert_eq!(held.goal.as_deref(), Some("get the tests passing"));
    }

    #[test]
    fn the_turning_point_names_both_halves() {
        // The most instructive thing in most episodes, and the pair a fixed window splits.
        let held = tell(&a_repair());
        let said = held.turning_point.expect("a turn");
        assert!(
            said.contains("cargo test") && said.contains("make test"),
            "{said}"
        );
    }

    #[test]
    fn an_episode_that_never_turned_has_no_turning_point() {
        let held = tell(&[user(0, "build it"), tool(1, "make build", true)]);
        assert_eq!(held.turning_point, None);
    }

    #[test]
    fn rerunning_the_same_command_is_not_a_turn() {
        // A flaky test passing on the second attempt taught nobody anything.
        let held = tell(&[
            user(0, "run them"),
            tool(1, "cargo test", false),
            tool(2, "cargo test", true),
        ]);
        assert_eq!(held.turning_point, None);
    }

    #[test]
    fn what_failed_and_was_never_fixed_is_left_open() {
        let held = tell(&[
            user(0, "deploy it"),
            tool(1, "make build", true),
            tool(2, "flyctl deploy", false),
        ]);
        assert_eq!(held.unresolved, vec!["flyctl deploy"]);
        assert_eq!(held.outcome, aeon_model::Outcome::Failed);
    }

    #[test]
    fn a_run_that_stopped_mid_flight_is_open_not_failed() {
        // Different states. An episode that was abandoned is not one that went wrong.
        let held = tell(&[user(0, "look into it"), user(1, "actually never mind")]);
        assert_eq!(held.outcome, aeon_model::Outcome::Open);
        assert!(held.unresolved.is_empty());
    }

    #[test]
    fn a_fixed_failure_is_not_left_open() {
        let held = tell(&a_repair());
        assert!(held.unresolved.is_empty());
        assert_eq!(held.outcome, aeon_model::Outcome::Done);
    }

    #[test]
    fn every_attempt_is_recorded_in_order() {
        let held = tell(&a_repair());
        assert_eq!(held.attempts, vec!["cargo test", "make test"]);
    }

    #[test]
    fn an_empty_span_says_nothing_rather_than_guessing() {
        // A field with no evidence is `None`, which is better than a plausible sentence nobody
        // can check.
        let held = tell(&[]);
        assert_eq!(held.goal, None);
        assert_eq!(held.turning_point, None);
        assert!(held.attempts.is_empty());
    }

    #[test]
    fn the_span_says_which_turns_it_covers() {
        // So a memory distilled from it can point back at the transcript rather than at a
        // summary of the transcript.
        let held = tell(&a_repair());
        assert_eq!(held.span, (0, 2));
    }

    #[test]
    fn nothing_here_needs_a_model() {
        // The floor. With no distiller at all an episode still says what it was for, what was
        // tried, where it turned, and what was left undone.
        let held = tell(&a_repair());
        assert_eq!(held.method, "rules");
        assert!(held.goal.is_some() && held.turning_point.is_some());
    }
}

#[cfg(test)]
mod avoiding {
    use super::*;
    use crate::observation::{Kind, Role};

    fn tool(cursor: u64, command: &str, ok: bool) -> Observation {
        Observation {
            cursor: Some(cursor),
            role: Role::Tool,
            kind: Kind::ToolResult,
            tool: Some("shell".to_owned()),
            args: Some(serde_json::json!({ "command": command })),
            ok: Some(ok),
            text: if ok { "ok" } else { "failed" }.to_owned(),
            at: Some(1_756_000_000),
            ..Observation::default()
        }
    }

    fn user(cursor: u64, text: &str) -> Observation {
        Observation {
            cursor: Some(cursor),
            role: Role::User,
            text: text.to_owned(),
            at: Some(1_756_000_000),
            ..Observation::default()
        }
    }

    #[test]
    fn a_failure_that_stayed_failed_becomes_a_narrow_avoidance() {
        let told = tell(&[
            user(0, "deploy it"),
            tool(1, "make build", true),
            tool(2, "flyctl deploy", false),
        ]);
        let held = avoidance(&told, "on this branch, where the lockfile is stale")
            .expect("something to avoid");

        assert_eq!(held.rejected, "flyctl deploy");
        assert!(held.when.contains("lockfile"));
        assert!(held.is_narrow());
    }

    #[test]
    fn a_failure_that_was_repaired_produces_no_prohibition() {
        // The repair is the better memory. A prohibition here would tell an agent to avoid the
        // thing that turned out to work.
        let told = tell(&[
            user(0, "run the tests"),
            tool(1, "cargo test", false),
            tool(2, "make test", true),
        ]);
        assert!(avoidance(&told, "in this workspace").is_none());
    }

    #[test]
    fn a_prohibition_with_no_condition_is_refused() {
        // "Do not do X" as a general rule is almost always false. X failed under conditions,
        // and one that cannot name them is a superstition an agent obeys forever.
        let told = tell(&[user(0, "deploy it"), tool(1, "flyctl deploy", false)]);
        assert!(avoidance(&told, "").is_none());
        assert!(avoidance(&told, "   ").is_none());
    }

    #[test]
    fn a_replacement_is_named_only_when_one_was_observed() {
        // Inventing one would be the worst of both: a prohibition and a guess.
        let bare = tell(&[user(0, "deploy it"), tool(1, "flyctl deploy", false)]);
        assert_eq!(
            avoidance(&bare, "on this branch").expect("held").instead,
            None
        );

        let with = tell(&[
            user(0, "deploy it"),
            tool(1, "make build", true),
            tool(2, "flyctl deploy", false),
        ]);
        assert_eq!(
            avoidance(&with, "on this branch")
                .expect("held")
                .instead
                .as_deref(),
            Some("make build")
        );
    }

    #[test]
    fn an_episode_that_simply_stopped_produces_nothing() {
        let told = tell(&[user(0, "look into it"), user(1, "never mind")]);
        assert!(avoidance(&told, "anywhere").is_none());
    }
}
