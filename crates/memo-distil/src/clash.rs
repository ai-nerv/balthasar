//! CLASH: a turn that disagrees with what this project already believes.
//!
//! The seventh path across the gate, and the first one that needs the store to notice it. Every
//! other extractive rule reads a turn in isolation and asks *what words are in it*. This one
//! asks what the project already thinks, and treats a person contradicting it as a correction —
//! whatever words they used.
//!
//! # Why this and not a longer word list
//!
//! FIX matches six openings and SAID matches eighteen phrases, and the sentence that started
//! all of this used none of them:
//!
//! ```text
//!   we moved off Heroku last month, it's all fly.io now
//! ```
//!
//! No marker, no imperative, and unmistakably a correction — because the project is holding
//! *"we deploy with heroku"* and this disagrees with it. The disagreement is the signal. Adding
//! phrases to a list would never have caught it; the list is not too short, it is the wrong
//! kind of thing.
//!
//! This is what the 2026 memory literature calls contradiction detection, and it consistently
//! comes out as one of the few storage triggers worth having: what is worth remembering is what
//! *changes* something, not what happens to be phrased memorably.
//!
//! # What keeps it safe
//!
//! A false clash is expensive — it would promote a claim nobody made and supersede one somebody
//! did. Three things keep it narrow:
//!
//! * **Only the person's turns.** An agent cannot talk itself into a belief by restating one.
//! * **Only assertions.** A question that names a value is asking, not correcting.
//! * **Only a real revision.** [`same_claim_different_value`](memo_model::same_claim_different_value)
//!   wants a shared opening run of at least two words covering half the shorter claim, with
//!   differing tails. Passing that is close to quoting the belief back with one part changed.

use crate::{Candidate, Observation, Role};
use memo_model::{Body, NoteKind, ScopeId, Tier, WitnessKind};
use memo_store::Store;

/// The shortest sentence that may contradict something, and the longest.
///
/// A three-word fragment matching a stored claim's opening is an accident; a paragraph is not a
/// claim. Between them is the range a person states a fact in.
const CLAIM: std::ops::RangeInclusive<usize> = 12..=200;

/// Every claim in these turns that disagrees with something the project holds.
///
/// Read-only against the store: this proposes, and the same gate that weighs every other
/// candidate decides.
#[must_use]
pub fn clashes(store: &Store, scope: &ScopeId, turns: &[Observation]) -> Vec<Candidate> {
    let mut out = Vec::new();

    for turn in turns.iter().filter(|t| t.role == Role::User) {
        for said in assertions(&turn.text) {
            let Ok(Some((_, held))) = store.what_this_revises(scope, &said) else {
                continue;
            };
            // What it contradicts goes into the witness note, so `memo why` can print the
            // argument — "you said this, and the project was holding that" — rather than
            // announcing a correction the person has to go and reconstruct.
            out.push(
                Candidate::new(
                    Body::note(&said, NoteKind::Claim),
                    Tier::Fact,
                    WitnessKind::Correction,
                    format!("clash with \"{}\"", memo_model::normalised(&held)),
                )
                .at(turn.cursor),
            );
            // One per turn. A person restating a decision several ways in one breath has
            // corrected one thing, and taking each phrasing would count their emphasis as
            // evidence — which is the whole failure the ladder's weights exist to avoid.
            break;
        }
    }
    out
}

/// The sentences in a turn that assert something.
///
/// Questions are dropped, and so is anything too short or too long to be a claim. A person
/// asking *"are we still on heroku?"* names the value without disagreeing with it, and taking
/// that as a correction would let curiosity rewrite a project's memory.
fn assertions(text: &str) -> Vec<String> {
    sentences(text)
        .into_iter()
        .filter(|line| !line.contains('?'))
        .filter(|line| CLAIM.contains(&line.len()))
        .collect()
}

/// A turn's sentences.
///
/// A full stop ends one only at the end of the text or before whitespace. Inside a run of
/// non-space characters it is part of a hostname, a version or a filename — and splitting there
/// turns `fly.io` into `fly`, which is how a claim about a deploy target became a claim about a
/// different one and a turn agreeing with the project read as contradicting it.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut held = String::new();
    let letters: Vec<char> = text.chars().collect();

    for (at, c) in letters.iter().enumerate() {
        let ends = match c {
            '\n' | '!' | ';' => true,
            '.' => letters.get(at + 1).is_none_or(|next| next.is_whitespace()),
            _ => false,
        };
        if ends {
            out.push(std::mem::take(&mut held));
        } else {
            held.push(*c);
        }
    }
    out.push(held);
    out.into_iter()
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use memo_model::{Memory, SessionId, Witness, WitnessId};
    use memo_store::mint;

    const NOW: memo_model::Timestamp = 1_756_000_000;

    fn scope() -> ScopeId {
        ScopeId::new("/w/thing")
    }

    /// A project that already believes something.
    fn believing(text: &str) -> Store {
        let mut store = Store::ephemeral().expect("store");
        let memory = Memory::new(
            mint(NOW),
            Tier::Fact,
            scope(),
            Body::note(text, NoteKind::Claim),
            NOW,
        );
        store
            .remember(
                memory,
                Witness::new(
                    WitnessId::new("w"),
                    WitnessKind::Imperative,
                    SessionId::new("01OLD"),
                    scope(),
                    NOW,
                ),
                NOW,
            )
            .expect("believe");
        store
    }

    fn said(text: &str) -> Observation {
        Observation {
            cursor: Some(7),
            role: Role::User,
            text: text.to_owned(),
            ..Observation::default()
        }
    }

    #[test]
    fn a_correction_with_no_marker_in_it_is_still_a_correction() {
        // The sentence this whole path exists for. No imperative, no opening "no," — just a
        // person stating what is true now, which happens to disagree with what is held.
        let store = believing("we deploy with heroku");
        let found = clashes(&store, &scope(), &[said("we deploy with fly.io now")]);

        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].witness, WitnessKind::Correction);
        assert!(
            found[0].from.contains("heroku"),
            "the receipt names what it contradicts: {}",
            found[0].from
        );
        assert_eq!(found[0].cursor, Some(7));
        assert!(found[0].text().contains("fly.io"));
    }

    #[test]
    fn agreeing_with_what_is_held_is_not_a_clash() {
        let store = believing("we deploy with fly.io");
        assert!(clashes(&store, &scope(), &[said("we deploy with fly.io")]).is_empty());
    }

    #[test]
    fn a_question_naming_the_value_is_asking_not_correcting() {
        // Curiosity must not rewrite a project's memory. Somebody checking what the target is
        // has said nothing about what it should be.
        let store = believing("we deploy with heroku");
        for asked in [
            "we deploy with fly.io now?",
            "do we deploy with fly.io now?",
            "we deploy with fly.io, right?",
        ] {
            assert!(
                clashes(&store, &scope(), &[said(asked)]).is_empty(),
                "{asked:?}"
            );
        }
    }

    #[test]
    fn only_the_person_can_contradict_the_project() {
        // An agent restating a belief back must not be able to talk itself into a new one, and
        // a tool printing a config value must not either.
        let store = believing("we deploy with heroku");
        for role in [Role::Assistant, Role::Tool] {
            let turn = Observation {
                role,
                text: "we deploy with fly.io now".to_owned(),
                ..Observation::default()
            };
            assert!(clashes(&store, &scope(), &[turn]).is_empty(), "{role:?}");
        }
    }

    #[test]
    fn something_the_project_has_no_opinion_about_is_not_a_clash() {
        let store = believing("we deploy with heroku");
        assert!(
            clashes(&store, &scope(), &[said("the database is postgres")]).is_empty(),
            "a new claim is not a correction — the other rules decide it"
        );
    }

    #[test]
    fn one_turn_correcting_one_thing_is_one_candidate() {
        // Emphasis is not evidence. Saying it three ways in one breath is one correction.
        let store = believing("we deploy with heroku");
        let found = clashes(
            &store,
            &scope(),
            &[said(
                "we deploy with fly.io now. we deploy with fly.io. really, we deploy with fly.io",
            )],
        );
        assert_eq!(found.len(), 1, "{found:?}");
    }
}
