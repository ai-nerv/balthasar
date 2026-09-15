//! How each kind of memory ages.

use crate::{Tier, Timestamp};

/// How fast a tier fades relative to the configured rate. Multiplies the decay constant;
/// below one is slower.
#[must_use]
pub fn tempo(tier: Tier) -> f64 {
    match tier {
        // Disuse is not evidence against a claim about the world.
        Tier::Fact => 0.25,

        // A habit that faded out of the live set would be rediscovered the hard way.
        Tier::Habit => 0.35,

        // One afternoon's work: worth keeping as evidence, not worth injecting a month later.
        Tier::Episode => 1.4,

        // Fading fast is what stops the scratch of a thousand runs competing with the project.
        Tier::Scratch => 2.0,

        Tier::Archive => 1.0,
    }
}

/// Whether an episode has earned more than the ordinary rate.
#[must_use]
pub fn episode_holds(cost_seconds: i64, corrected: bool, derived_from: usize) -> bool {
    cost_seconds >= 30 * 60 || corrected || derived_from > 0
}

/// Whether a fact should be shown with a caveat rather than stated plainly.
///
/// Staleness is about validity and observation age, never about how often something was recalled.
#[must_use]
pub fn is_stale(observed_at: Timestamp, valid_to: Option<Timestamp>, now: Timestamp) -> bool {
    if let Some(until) = valid_to {
        return now > until;
    }
    now - observed_at > 365 * 86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = 1_756_000_000;
    const DAY: Timestamp = 86_400;

    #[test]
    fn a_fact_outlasts_an_episode() {
        assert!(tempo(Tier::Fact) < tempo(Tier::Episode));
        assert!(
            tempo(Tier::Fact) < 1.0,
            "facts fade slower than the base rate"
        );
    }

    #[test]
    fn a_procedure_does_not_fade_out_of_reach() {
        assert!(tempo(Tier::Habit) < 1.0);
    }

    #[test]
    fn a_sessions_own_notes_fade_fastest() {
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
