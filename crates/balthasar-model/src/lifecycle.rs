//! How each kind of memory ages.
//!
//! One decay intuition applied to everything is wrong in both directions. A fact does not
//! become false because nobody asked about it, so treating disuse as evidence against it would
//! quietly discard true things. An ordinary episode is a record of one afternoon, and keeping it
//! at full strength forever fills the live set with afternoons.
//!
//! So the multiplier below is a function of what the memory *is*, and the reasoning for each
//! number is written beside it rather than tuned into a constant nobody can defend.

use crate::{Tier, Timestamp};

/// How fast a tier fades relative to the configured rate.
///
/// Multiplies the decay constant. Below one is slower.
#[must_use]
pub fn tempo(tier: Tier) -> f64 {
    match tier {
        // A fact is a claim about the world with a validity interval. Disuse is not evidence
        // against it, so it barely fades at all — what removes a fact is supersession or a
        // contradiction, both of which are handled elsewhere and neither of which is a clock.
        Tier::Fact => 0.25,

        // A procedure fades slowly for the same reason, and there is a second one: a habit that
        // faded out of the live set would be rediscovered the hard way, which is precisely the
        // failure the whole system is measured on.
        Tier::Habit => 0.35,

        // One afternoon's work. Most of them are worth keeping as evidence and not worth
        // injecting a month later, and this is the number that makes that happen.
        Tier::Episode => 1.4,

        // A session's own notes, which the ladder is already deciding about. Fading fast is
        // what stops the scratch of a thousand runs competing with the project's memory.
        Tier::Scratch => 2.0,

        // Already out of the live set. Nothing here is asserted, so the rate is a formality.
        Tier::Archive => 1.0,
    }
}

/// Whether an episode has earned more than the ordinary rate.
///
/// The exceptions are the episodes that turn out to matter later: the expensive ones, the ones
/// that corrected something, and the ones other memories were derived from. Each of these is an
/// observable property rather than a judgment, which is what keeps this a rule.
#[must_use]
pub fn episode_holds(cost_seconds: i64, corrected: bool, derived_from: usize) -> bool {
    // Half an hour of work is a real investment, and its record is worth more than a
    // two-minute lookup's.
    cost_seconds >= 30 * 60 || corrected || derived_from > 0
}

/// Whether a fact should be shown with a caveat rather than stated plainly.
///
/// Staleness is about validity and observation age, never about how often something was
/// recalled. A fact nobody has needed for a year is not stale; a fact observed a year ago whose
/// subject changes monthly is.
#[must_use]
pub fn is_stale(observed_at: Timestamp, valid_to: Option<Timestamp>, now: Timestamp) -> bool {
    if let Some(until) = valid_to {
        return now > until;
    }
    // A year without re-observation. Not a floor on truth — a threshold for saying "as of".
    now - observed_at > 365 * 86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = 1_756_000_000;
    const DAY: Timestamp = 86_400;

    #[test]
    fn a_fact_outlasts_an_episode() {
        // The whole point of tier-aware decay: a claim about the world and a record of one
        // afternoon should not fade at the same speed.
        assert!(tempo(Tier::Fact) < tempo(Tier::Episode));
        assert!(
            tempo(Tier::Fact) < 1.0,
            "facts fade slower than the base rate"
        );
    }

    #[test]
    fn a_procedure_does_not_fade_out_of_reach() {
        // A habit that decayed away would be relearned the hard way, which is exactly the
        // failure the benchmark measures.
        assert!(tempo(Tier::Habit) < 1.0);
    }

    #[test]
    fn a_sessions_own_notes_fade_fastest() {
        // Otherwise the scratch of a thousand runs competes with the project's memory.
        assert!(tempo(Tier::Scratch) > tempo(Tier::Episode));
        assert!(tempo(Tier::Scratch) > tempo(Tier::Fact));
    }

    #[test]
    fn an_expensive_episode_is_worth_keeping() {
        assert!(episode_holds(45 * 60, false, 0), "an hour of work");
        assert!(episode_holds(60, true, 0), "it corrected something");
        assert!(episode_holds(60, false, 3), "things were derived from it");
        assert!(!episode_holds(60, false, 0), "an ordinary two minutes");
    }

    #[test]
    fn a_fact_nobody_asked_about_is_not_stale() {
        // Disuse is not evidence. A system that marked unread facts stale would caveat
        // everything it had not been asked about lately.
        assert!(!is_stale(NOW - 30 * DAY, None, NOW));
    }

    #[test]
    fn a_fact_whose_interval_closed_is_stale() {
        assert!(is_stale(NOW - 10 * DAY, Some(NOW - DAY), NOW));
        assert!(!is_stale(NOW - 10 * DAY, Some(NOW + DAY), NOW));
    }

    #[test]
    fn a_very_old_observation_earns_an_as_of() {
        assert!(is_stale(NOW - 400 * DAY, None, NOW));
        assert!(!is_stale(NOW - 300 * DAY, None, NOW));
    }
}
