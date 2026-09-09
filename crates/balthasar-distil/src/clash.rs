//! CLASH: a turn that disagrees with what this project already believes.
//!
//! Every other extractive rule reads a turn in isolation and asks *what words are in it*. This
//! one asks what the project already thinks, and treats a person contradicting it as a
//! correction — whatever words they used.
//!
//! ```text
//!   we moved off Heroku last month, it's all fly.io now
//! ```
//!
//! No marker, no imperative, and a correction — the project is holding *"we deploy with heroku"*.
//!
//! A false clash would promote a claim nobody made and supersede one somebody did. What keeps it
//! narrow:
//!
//! * **Only the person's turns.** An agent cannot talk itself into a belief by restating one.
//! * **Only assertions.** A question that names a value is asking, not correcting.
//! * **Only a real revision.** [`same_claim_different_value`](balthasar_model::same_claim_different_value)
//!   wants a shared opening run of two words covering half the shorter claim, and differing tails.

use crate::{Candidate, Observation, Role};
use balthasar_model::{Body, NoteKind, ScopeId, Tier, WitnessKind};
use balthasar_store::Store;

/// The shortest sentence that may contradict something, and the longest.
const CLAIM: std::ops::RangeInclusive<usize> = 12..=200;

/// Every claim in these turns that disagrees with something the project holds.
///
/// Read-only against the store: this proposes, and the gate decides.
#[must_use]
pub fn clashes(store: &Store, scope: &ScopeId, turns: &[Observation]) -> Vec<Candidate> {
    let mut out = Vec::new();

    for turn in turns
        .iter()
        .filter(|t| t.role == Role::User && t.kind.can_instruct())
    {
        for said in assertions(&turn.text) {
            let Ok(Some((_, held))) = store.what_this_revises(scope, &said) else {
                continue;
            };
            // What it contradicts goes into the witness note, so `balthasar why` can print it.
            out.push(
                Candidate::new(
                    Body::note(&said, NoteKind::Claim),
                    Tier::Fact,
                    WitnessKind::Correction,
                    format!("clash with \"{}\"", balthasar_model::normalised(&held)),
                )
                .at(turn.cursor),
            );
            // One per turn: a person restating a decision several ways has corrected one thing.
            break;
        }
    }
    out
}

/// The sentences in a turn that assert something.
///
/// Questions are dropped, and so is anything too short or too long to be a claim.
fn assertions(text: &str) -> Vec<String> {
    sentences(text)
        .into_iter()
        .filter(|line| !line.contains('?'))
        .filter(|line| CLAIM.contains(&line.len()))
        .collect()
}

/// A turn's sentences.
///
/// A full stop ends one only at the end of the text or before whitespace, so a hostname, a
/// version or a filename stays whole.
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
    use balthasar_model::{Memory, SessionId, Witness, WitnessId};
    use balthasar_store::mint;

    const NOW: balthasar_model::Timestamp = 1_756_000_000;

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
