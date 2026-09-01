//! What a memory is.
//!
//! Pure data: no SQL, no Lua, no sockets, no clock beyond the one a caller hands in. Every
//! other crate is a projection of what is declared here, so this is the file to read first and
//! the one to change last.
//!
//! The shape that carries the design is [`Witness`]. A durable memory does not have a
//! confidence somebody assigned to it; it has the evidence that promoted it, and its
//! confidence is computed from that evidence every time the evidence changes. That is what
//! makes `aeon why` possible, and `aeon why` is the reason to trust anything else here.

mod body;
mod cardinality;
mod claim;
mod confidence;
pub mod floor;
mod id;
mod memory;
mod strength;
mod temporal;
mod tier;
mod witness;

pub use body::{Body, NoteKind, Outcome, Span};
pub use cardinality::is_single_valued;
pub use claim::{lead, same_claim_different_value};
pub use confidence::{Contradiction, of as confidence_of};
pub use id::{MemoryId, ScopeId, SessionId, WitnessId};
pub use memory::{Link, LinkRelation, Memory, Provenance, Through};
pub use strength::{Importance, Strength};
pub use temporal::{Temporal, Timestamp};
pub use tier::{Privacy, Tier};
pub use witness::{Witness, WitnessKind};

/// Digest of a memory's content, for deduplication and the cheap half of clustering.
///
/// Over the rendered text rather than the struct: two memories that say the same thing in
/// different tiers are the same claim, and a hash that included the tier would not say so.
#[must_use]
pub fn content_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.trim().to_lowercase().as_bytes());
    format!("{:x}", hasher.finalize())
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
}
