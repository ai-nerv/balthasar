//! Forgetting, as arithmetic.
//!
//! Ebbinghaus decay with a floor: strength falls exponentially with elapsed time, and the rate
//! is damped by how often the memory has actually been needed. Without decay a store grows
//! without bound and retrieval drowns in stale noise; without the floor, the thing the agent
//! reaches for every day fades at the same rate as a passing remark.
//!
//! Confidence (`crate::confidence`) is a different quantity and they are easy to confuse.
//! Strength says *how faded* — a function of time and use. Confidence says *how sure* — a
//! function of evidence. A well-witnessed fact nobody has needed in a year is confident and
//! faint; a guess repeated this morning is doubtful and bright.

use crate::Timestamp;
use std::fmt;
use std::str::FromStr;

/// How fast a memory is allowed to fade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    /// Identity, safety constraints, what must never be got wrong. ~139-day half-life.
    Critical,
    /// This project's build, deploy and test commands. ~69 days.
    High,
    /// Ordinary facts. ~14 days.
    #[default]
    Normal,
    /// Passing observations. ~7 days.
    Low,
}

impl Importance {
    /// Base decay constant per day.
    ///
    /// LivingBrain's numbers, taken unchanged: they are the Ebbinghaus curve, and there is
    /// nothing here to improve on.
    #[must_use]
    pub fn rate(self) -> f64 {
        match self {
            Self::Critical => 0.005,
            Self::High => 0.01,
            Self::Normal => 0.05,
            Self::Low => 0.1,
        }
    }

    /// The wire and column spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Normal => "normal",
            Self::Low => "low",
        }
    }
}

/// How faded a memory is, and what resists the fading.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Strength {
    /// Current strength, `0.0..=1.0`.
    pub value: f64,
    /// Which decay class this belongs to.
    pub importance: Importance,
    /// When it was last recalled or reinforced.
    pub last_accessed: Timestamp,
    /// How many times it has been recalled. Drives the inertia below.
    pub access_count: u32,
    /// Pinned memories do not decay at all.
    pub pinned: bool,
}

impl Strength {
    /// A memory at full strength, learned now.
    #[must_use]
    pub fn fresh(importance: Importance, now: Timestamp) -> Self {
        Self {
            value: 1.0,
            importance,
            last_accessed: now,
            access_count: 0,
            pinned: false,
        }
    }

    /// The decay constant actually applied, after inertia.
    ///
    /// Logarithmic damping by access count: 1 access leaves the rate alone, ~50 quarter it,
    /// ~1000 reduce it roughly sevenfold. A memory the agent keeps needing stops fading, which
    /// is the floor the literature asks for and forty lines of arithmetic supply.
    #[must_use]
    pub fn effective_rate(&self) -> f64 {
        if self.pinned {
            return 0.0;
        }
        let inertia = 1.0 / (1.0 + f64::from(self.access_count).ln_1p());
        self.importance.rate() * inertia
    }

    /// Strength as it would be at `now`, without writing it back.
    ///
    /// Separate from [`Self::decay`] so `memo decay --preview` can show what a pass would do
    /// before it does it. Forgetting is the most alarming thing this system performs and it
    /// should never be a surprise.
    #[must_use]
    pub fn at(&self, now: Timestamp) -> f64 {
        if self.pinned {
            return 1.0;
        }
        let days = ((now - self.last_accessed).max(0)) as f64 / 86_400.0;
        (self.value * (-self.effective_rate() * days).exp()).clamp(0.0, 1.0)
    }

    /// What this will be worth at `now`, for a memory of this tier.
    ///
    /// The tier-aware form. A fact and an afternoon's episode should not fade at the same
    /// speed: disuse is not evidence against a claim about the world, and it is exactly what an
    /// episode's worth is made of. See [`crate::tempo`] for the multipliers and the case for
    /// each one.
    #[must_use]
    pub fn at_tier(&self, tier: crate::Tier, now: Timestamp) -> f64 {
        if self.pinned {
            return 1.0;
        }
        let days = ((now - self.last_accessed).max(0)) as f64 / 86_400.0;
        let rate = self.effective_rate() * crate::tempo(tier);
        (self.value * (-rate * days).exp()).clamp(0.0, 1.0)
    }
    /// Apply the fade, up to `now`.
    pub fn decay(&mut self, now: Timestamp) {
        if self.pinned {
            return;
        }
        self.value = self.at(now);
        self.last_accessed = now;
    }

    /// Recall reinforces: back to full, and one more reason to resist fading next time.
    pub fn touch(&mut self, now: Timestamp) {
        self.value = 1.0;
        self.last_accessed = now;
        self.access_count = self.access_count.saturating_add(1);
    }

    /// Whether this has faded past the point of being worth keeping live.
    #[must_use]
    pub fn is_spent(&self, now: Timestamp, floor: f64) -> bool {
        !self.pinned && self.at(now) < floor
    }
}

/// What a parse of an unknown importance says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownImportance(pub String);

impl fmt::Display for UnknownImportance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}' is not an importance memo knows", self.0)
    }
}

impl std::error::Error for UnknownImportance {}

impl FromStr for Importance {
    type Err = UnknownImportance;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "critical" => Ok(Self::Critical),
            "high" => Ok(Self::High),
            "normal" => Ok(Self::Normal),
            "low" => Ok(Self::Low),
            other => Err(UnknownImportance(other.to_owned())),
        }
    }
}

impl fmt::Display for Importance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Timestamp = 86_400;
    const NOW: Timestamp = 1_756_000_000;

    fn about(got: f64, want: f64) {
        assert!((got - want).abs() < 0.02, "got {got}, wanted about {want}");
    }

    #[test]
    fn each_class_halves_when_it_says_it_does() {
        for (importance, half_life) in [
            (Importance::Critical, 139.0),
            (Importance::High, 69.0),
            (Importance::Normal, 14.0),
            (Importance::Low, 7.0),
        ] {
            let s = Strength::fresh(importance, NOW);
            let then = NOW + (half_life * 86_400.0) as Timestamp;
            about(s.at(then), 0.5);
        }
    }

    #[test]
    fn a_pinned_memory_never_fades() {
        let mut s = Strength::fresh(Importance::Low, NOW);
        s.pinned = true;
        assert_eq!(s.at(NOW + 10_000 * DAY), 1.0);
        assert_eq!(s.effective_rate(), 0.0);
    }

    #[test]
    fn what_is_needed_often_resists_fading() {
        // The floor the literature asks for: a memory the agent keeps reaching for should not
        // fade at the same rate as a passing remark.
        let once = Strength::fresh(Importance::Normal, NOW);
        let mut often = Strength::fresh(Importance::Normal, NOW);
        often.access_count = 50;
        assert!(often.effective_rate() < once.effective_rate() * 0.3);
        assert!(often.at(NOW + 30 * DAY) > once.at(NOW + 30 * DAY));
    }

    #[test]
    fn recall_restores_and_counts() {
        let mut s = Strength::fresh(Importance::Normal, NOW);
        s.decay(NOW + 60 * DAY);
        assert!(s.value < 0.1);
        s.touch(NOW + 60 * DAY);
        assert_eq!(s.value, 1.0);
        assert_eq!(s.access_count, 1);
    }

    #[test]
    fn previewing_does_not_change_anything() {
        // `memo decay --preview` has to be able to show the future without causing it.
        let s = Strength::fresh(Importance::Normal, NOW);
        let before = s.value;
        let _ = s.at(NOW + 100 * DAY);
        assert_eq!(s.value, before);
    }

    #[test]
    fn a_pinned_memory_is_never_spent() {
        let mut s = Strength::fresh(Importance::Low, NOW);
        s.pinned = true;
        assert!(!s.is_spent(NOW + 10_000 * DAY, 0.10));
    }

    #[test]
    fn decay_does_not_run_backwards() {
        let s = Strength::fresh(Importance::Normal, NOW);
        assert_eq!(s.at(NOW - 100 * DAY), 1.0);
    }
}
