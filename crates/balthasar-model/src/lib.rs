//! What a memory is.
//!
//! Pure data: no SQL, no Lua, no sockets, no clock beyond the one a caller hands in. Every
//! other crate is a projection of what is declared here, so this is the file to read first and
//! the one to change last.
//!
//! The shape that carries the design is [`Witness`]. A durable memory does not have a
//! confidence somebody assigned to it; it has the evidence that promoted it, and its
//! confidence is computed from that evidence every time the evidence changes. That is what
//! makes `balthasar why` possible, and `balthasar why` is the reason to trust anything else here.

mod body;
mod cardinality;
mod channel;
mod claim;
mod confidence;
pub mod floor;
mod guard;
mod habit;
mod id;
mod lifecycle;
mod memory;
pub mod noted;
mod relation;
pub mod scratch;
mod skill;
mod strength;
mod temporal;
mod tier;
mod utility;
mod witness;

pub use body::{Body, NoteKind, Outcome, Span};
pub use cardinality::is_single_valued;
pub use channel::{Channel, Domain};
pub use claim::{claim_overlap, lead, same_claim, same_claim_different_value};
pub use confidence::{Contradiction, of as confidence_of};
pub use guard::{looks_like_injection, presentation_for, witness_for};
pub use habit::{Avoidance, Environment, Polarity, Record, Standing};
pub use id::{MemoryId, ScopeId, SessionId, WitnessId};
pub use lifecycle::{episode_holds, is_stale, tempo};
pub use memory::{Link, LinkRelation, Memory, Provenance, Through};
pub use relation::{Derivation, Family, Relation, View};
pub use skill::{Skill, Step as SkillStep, Verification};
pub use strength::{Importance, Strength};
pub use temporal::{Temporal, Timestamp};
pub use tier::{Privacy, Tier};
pub use utility::{Attribution, OutcomeKind, Presentation, Utility};
pub use witness::{Witness, WitnessKind};

/// Digest of a memory's content, for deduplication and the cheap half of clustering.
///
/// Over the rendered text rather than the struct: two memories that say the same thing in
/// different tiers are the same claim, and a hash that included the tier would not say so.
///
/// Normalised first, by [`normalised`]. The difference between ``use `make test``` and
/// `use make test.` is formatting, and two people who typed those have said one thing.
#[must_use]
pub fn content_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(normalised(text).as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Punctuation that is formatting when it sits at the edge of a word.
///
/// Trimmed only at a token's edges, never from inside it. `fly.io`, `make-test` and `-v` are
/// claims about the world; the comma in `fly.io,` is somebody typing a sentence. A blanket
/// strip would merge `use -v` with `use v`, which are different instructions.
const EDGE: &[char] = &[
    '.', ',', '!', '?', ';', ':', '"', '\'', '`', '(', ')', '[', ']', '{', '}', '\u{2018}',
    '\u{2019}', '\u{201c}', '\u{201d}',
];

/// One claim's text, with the ways of typing it that do not change what it says removed.
///
/// Case, runs of whitespace, and edge punctuation. Nothing cleverer: this is the exact half of
/// clustering, and everything it merges becomes corroboration — so it may only merge what is
/// the same claim beyond argument. Anything needing judgement belongs above it, where there is
/// a threshold to tune and a witness note to record what was done.
#[must_use]
pub fn normalised(text: &str) -> String {
    text.split_whitespace()
        .map(|word| word.trim_matches(EDGE))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_ignores_case_and_surrounding_space() {
        // Two people stating one fact should not produce two facts. Normalising here is
        // cheaper and more predictable than asking an embedding whether they match.
        assert_eq!(content_hash("  Uses Make  "), content_hash("uses make"));
    }

    #[test]
    fn a_hash_distinguishes_different_claims() {
        assert_ne!(content_hash("uses make"), content_hash("uses cargo"));
    }

    #[test]
    fn a_hash_ignores_how_a_claim_was_typed() {
        // The commonest way one claim became two: a person writing prose, an extractor quoting
        // a command. None of these differences change what was said.
        let same = [
            "we deploy with `make ship`",
            "We deploy with make ship.",
            "we  deploy   with make ship",
            "\"we deploy with make ship\"",
        ];
        for text in &same[1..] {
            assert_eq!(
                content_hash(same[0]),
                content_hash(text),
                "{text:?} says the same thing as {:?}",
                same[0]
            );
        }
    }

    #[test]
    fn punctuation_inside_a_word_is_part_of_the_claim() {
        // The line this normalisation must not cross. A blanket strip would make each of these
        // pairs identical, and each pair is two different instructions.
        assert_ne!(
            content_hash("deploy to fly.io"),
            content_hash("deploy to flyio")
        );
        assert_ne!(content_hash("run make-test"), content_hash("run make test"));
        assert_ne!(content_hash("pass -v"), content_hash("pass v"));
    }

    #[test]
    fn normalising_nothing_is_not_a_crash() {
        assert_eq!(normalised("   "), "");
        assert_eq!(normalised("..."), "");
        assert_eq!(content_hash(""), content_hash("  ,  "));
    }
}
