//! Minting an identity: a ULID, sortable by creation and needing no coordination.

use balthasar_model::{MemoryId, Timestamp};
use std::sync::atomic::{AtomicU64, Ordering};

/// Distinguishes two ids minted in the same millisecond. A counter, not a random number.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A fresh identity, for something observed at `now`.
///
/// The timestamp comes from the real clock in milliseconds; `now` is seconds, and scaling it
/// leaves every id minted in one second sharing its first ten characters. The entropy is mixed
/// rather than packed: the handle a person types is the trailing characters, and shifting bits
/// into place leaves those at zero.
#[must_use]
pub fn mint(now: Timestamp) -> MemoryId {
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok();
    let millis = clock.map_or_else(
        || (now.max(0) as u64).saturating_mul(1000),
        |d| d.as_millis() as u64,
    );

    // A genuine collision is refused by the primary key rather than silently overwriting.
    let seed = u64::from(clock.map_or(0, |d| d.subsec_nanos()))
        ^ SEQUENCE
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (u64::from(std::process::id()) << 32);

    let entropy = (u128::from(mix(seed)) << 16) | u128::from(mix(seed ^ 0xA5A5_A5A5) & 0xFFFF);
    MemoryId::minted(millis, entropy)
}

/// SplitMix64's finaliser, so neighbouring seeds do not produce neighbouring ids.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[cfg(test)]
mod mint_tests {
    use super::*;

    /// The trailing eight characters, which is what `balthasar why` and `balthasar forget` are given.
    fn handle(id: &MemoryId) -> String {
        let text = id.as_str();
        text.chars().skip(text.chars().count() - 8).collect()
    }

    #[test]
    fn two_ids_minted_together_have_different_handles() {
        let ids: Vec<MemoryId> = (0..64).map(|_| mint(1_756_000_000)).collect();
        let mut handles: Vec<String> = ids.iter().map(handle).collect();
        handles.sort();
        handles.dedup();
        assert_eq!(handles.len(), 64, "handles must tell memories apart");
    }

    #[test]
    fn a_handle_is_not_all_one_character() {
        let id = mint(1_756_000_000);
        let text = handle(&id);
        assert!(
            text.chars()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                > 1,
            "{text} carries no entropy"
        );
    }

    #[test]
    fn ids_still_sort_by_when_they_were_made() {
        let early = mint(1_756_000_000);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let late = mint(1_756_000_000);
        assert!(early < late, "{early} should sort before {late}");
    }
}
