//! How sure, derived from what is known.
//!
//! Confidence is never assigned. It is computed from the witnesses, every time the witnesses
//! change, which is what makes `aeon why` an argument rather than a restatement. A number
//! somebody typed cannot be explained, cannot be audited, and cannot be wrong in a way anyone
//! notices.
//!
//! ```text
//!   confidence = saturate( Σ witness.value(now) )    how much evidence
//!              × diversity(distinct sessions)         from how many directions
//!              × (1 − contradiction_pressure)         against how much dissent
//!              × currency                             and is it still claimed to hold
//! ```

use crate::{Timestamp, Witness};
use std::collections::BTreeSet;

/// Evidence at which the saturating curve reaches half.
///
/// Tuned so one imperative (weight 1.0) lands near 0.67 before diversity, and a single
/// distillation (0.3) lands near 0.38 — above the retrieval floor, below where anything is
/// asserted with a straight face.
const HALF: f64 = 0.5;

/// How much a single-session claim is discounted.
///
/// One session repeating something is a person being emphatic; the same thing surfacing in
/// unrelated runs is a property of the world. The gap between them is this constant.
const LONE_SESSION: f64 = 0.75;

/// What a superseded claim keeps.
///
/// Not zero: "this was true until March" is a real answer to a question about March, and a
/// claim whose confidence collapsed to nothing could not be told apart from one nobody ever
/// had reason to believe.
const SUPERSEDED: f64 = 0.3;

/// A claim pulling against this one, and how sure *it* is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contradiction {
    /// The confidence of the memory doing the contradicting.
    pub confidence: f64,
}

/// Confidence in a memory, from its evidence.
///
/// `superseded` is whether the claim's validity interval has been closed — a fact that stopped
/// being true is not a fact nobody witnessed, and the two must not score the same.
#[must_use]
pub fn of(
    witnesses: &[Witness],
    contradictions: &[Contradiction],
    superseded: bool,
    pinned: bool,
    now: Timestamp,
) -> f64 {
    // A pinned memory is one somebody chose to keep. Deriving a number for it and then arguing
    // with them about it would be worse than useless.
    //
    // Superseding still applies. Pinning says "do not let this fade"; it does not say "this is
    // true forever", and a pinned fact that has been corrected must stop being asserted like
    // any other. Returning 1.0 here unconditionally had `aeon recall` showing a replaced
    // answer at full confidence beside the answer that replaced it.
    if pinned {
        return if superseded { SUPERSEDED } else { 1.0 };
    }
    if witnesses.is_empty() {
        return 0.0;
    }

    let evidence: f64 = witnesses.iter().map(|w| w.value(now)).sum();
    let saturated = evidence / (evidence + HALF);

    let sessions: BTreeSet<&str> = witnesses.iter().map(|w| w.session.as_str()).collect();
    let diversity = if sessions.len() <= 1 {
        LONE_SESSION
    } else {
        // Approaches 1.0 quickly: two independent runs is most of the signal, and the fifth
        // adds very little that the second did not.
        1.0 - (1.0 - LONE_SESSION) * (-((sessions.len() - 1) as f64)).exp()
    };

    let dissent: f64 = contradictions.iter().map(|c| c.confidence).sum();
    let pressure = dissent / (dissent + 1.0);

    let currency = if superseded { SUPERSEDED } else { 1.0 };

    (saturated * diversity * (1.0 - pressure) * currency).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ScopeId, SessionId, WitnessId, WitnessKind};

    const NOW: Timestamp = 1_756_000_000;

    fn w(kind: WitnessKind, session: &str) -> Witness {
        Witness::new(
            WitnessId::new(format!("w-{session}-{kind}")),
            kind,
            SessionId::new(session),
            ScopeId::global(),
            NOW,
        )
    }

    fn plain(witnesses: &[Witness]) -> f64 {
        of(witnesses, &[], false, false, NOW)
    }

    #[test]
    fn nothing_witnessed_is_nothing_believed() {
        assert_eq!(plain(&[]), 0.0);
    }

    #[test]
    fn a_pinned_memory_is_certain_without_argument() {
        assert_eq!(of(&[], &[], false, true, NOW), 1.0);
    }

    #[test]
    fn pinning_does_not_make_a_corrected_fact_true() {
        // Pinning says "do not let this fade". It does not say "this is true forever", and a
        // pinned answer that has been replaced must stop being asserted like any other.
        let pinned_and_replaced = of(&[], &[], true, true, NOW);
        assert!(pinned_and_replaced < 0.35, "got {pinned_and_replaced}");
        assert!(
            pinned_and_replaced > 0.0,
            "it was believed, and for good reason"
        );
    }

    #[test]
    fn many_sessions_beat_one_loud_session() {
        // The cheapest defence there is against a single poisoned run.
        let loud = [
            w(WitnessKind::Repetition, "s1"),
            w(WitnessKind::Repetition, "s1"),
            w(WitnessKind::Repetition, "s1"),
        ];
        let spread = [
            w(WitnessKind::Repetition, "s1"),
            w(WitnessKind::Repetition, "s2"),
            w(WitnessKind::Repetition, "s3"),
        ];
        assert!(plain(&spread) > plain(&loud));
    }

    #[test]
    fn one_distillation_is_findable_but_not_assertable() {
        // Above the retrieval floor (0.10), below the injection floor (0.35). This is the
        // whole point of §5.1's weight being 0.3.
        let c = plain(&[w(WitnessKind::Distillation, "s1")]);
        assert!(c > 0.10, "should still be findable, got {c}");
        assert!(c < 0.35, "should not be asserted, got {c}");
    }

    #[test]
    fn one_imperative_is_asserted() {
        let c = plain(&[w(WitnessKind::Imperative, "s1")]);
        assert!(c > 0.35, "what the user asked for is asserted, got {c}");
    }

    #[test]
    fn dissent_pushes_a_claim_below_assertion() {
        let evidence = [w(WitnessKind::Cost, "s1"), w(WitnessKind::Cost, "s2")];
        let quiet = of(&evidence, &[], false, false, NOW);
        let disputed = of(
            &evidence,
            &[Contradiction { confidence: 0.9 }],
            false,
            false,
            NOW,
        );
        assert!(disputed < quiet);
        assert!(disputed < 0.35, "a disputed claim stops being asserted");
    }

    #[test]
    fn a_superseded_claim_is_kept_but_no_longer_asserted() {
        // "This was true until March" is a real answer. It must not score as if nobody ever
        // had reason to believe it.
        let evidence = [w(WitnessKind::Imperative, "s1"), w(WitnessKind::Cost, "s2")];
        let live = of(&evidence, &[], false, false, NOW);
        let past = of(&evidence, &[], true, false, NOW);
        assert!(past < 0.35, "not asserted any more, got {past}");
        assert!(past > 0.0, "still known to have been believed, got {past}");
        assert!(past < live);
    }

    #[test]
    fn confidence_never_leaves_its_range() {
        let piles: Vec<Witness> = (0..200)
            .map(|i| w(WitnessKind::Imperative, &format!("s{i}")))
            .collect();
        let c = plain(&piles);
        assert!((0.0..=1.0).contains(&c), "got {c}");
    }
}
