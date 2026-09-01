//! Identities.
//!
//! [`MemoryId`] is a ULID: sortable by creation time, needing no coordination, and doubling as
//! a timeline. A UUID would have cost an index on `observed_at` to answer "what did this
//! session produce, in order".
//!
//! Written here rather than taken from a crate because it is forty lines and the alternative
//! is a dependency in the crate every other crate depends on.

use std::fmt;

/// Crockford's base32, which is what a ULID's text form is spelled in.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

macro_rules! text_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Take a string as this identity, as it arrived from a store or a peer.
            #[must_use]
            pub fn new(text: impl Into<String>) -> Self {
                Self(text.into())
            }

            /// The identity as text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(text: String) -> Self {
                Self(text)
            }
        }
    };
}

text_id!(MemoryId, "One memory, anywhere in the store.");
text_id!(WitnessId, "One piece of evidence for one memory.");
text_id!(SessionId, "One run of a harness, as that harness names it.");
text_id!(
    ScopeId,
    "Which store a memory lives in. `global`, or a stable name for a project."
);

impl ScopeId {
    /// The store that holds what is true everywhere.
    #[must_use]
    pub fn global() -> Self {
        Self::new("global")
    }

    /// Whether this is the global scope rather than a project's.
    #[must_use]
    pub fn is_global(&self) -> bool {
        self.as_str() == "global"
    }
}

impl MemoryId {
    /// A fresh identity for something observed at `millis` since the epoch.
    ///
    /// The timestamp occupies the leading ten characters, so ids sort by creation without a
    /// clock being consulted twice. `entropy` is the caller's — this crate holds no RNG,
    /// because a pure-data crate that reaches for the operating system stops being one.
    #[must_use]
    pub fn minted(millis: u64, entropy: u128) -> Self {
        let mut out = String::with_capacity(26);
        for shift in (0..10).rev() {
            let index = (millis >> (shift * 5)) & 0x1f;
            out.push(char::from(ALPHABET[index as usize]));
        }
        for shift in (0..16).rev() {
            let index = (entropy >> (shift * 5)) & 0x1f;
            out.push(char::from(ALPHABET[index as usize]));
        }
        Self(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minted_id_is_twenty_six_characters() {
        assert_eq!(MemoryId::minted(1_700_000_000_000, 7).as_str().len(), 26);
    }

    #[test]
    fn later_ids_sort_after_earlier_ones() {
        // The whole reason for a ULID: "what did this session produce, in order" is a sort,
        // not an index on another column.
        let early = MemoryId::minted(1_700_000_000_000, u128::MAX);
        let late = MemoryId::minted(1_700_000_000_001, 0);
        assert!(early < late, "{early} should sort before {late}");
    }

    #[test]
    fn ids_minted_in_one_millisecond_differ_by_entropy() {
        let a = MemoryId::minted(1_700_000_000_000, 1);
        let b = MemoryId::minted(1_700_000_000_000, 2);
        assert_ne!(a, b);
    }

    #[test]
    fn global_is_the_scope_that_answers_everywhere() {
        assert!(ScopeId::global().is_global());
        assert!(!ScopeId::new("/home/you/work/thing").is_global());
    }
}
