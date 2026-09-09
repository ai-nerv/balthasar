//! Whether using a memory helped.
//!
//! Truth and utility are independent. Only an attributed outcome is evidence of utility, and an
//! action whose outcome nobody reported is unknown rather than failed.

use std::fmt;
use std::str::FromStr;

/// How an action that used a memory turned out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    /// The action did what it set out to do.
    Succeeded,
    /// It did not.
    Failed,
    /// It was followed, then fixed — evidence the memory was close but wrong.
    Corrected,
    /// It was undone.
    Reverted,
    /// The agent declined to act, which is sometimes the right answer.
    Abstained,
    /// The memory was injected and visibly not used.
    Ignored,
    /// Nobody said. The default, and not a failure.
    Unknown,
}

impl OutcomeKind {
    /// Whether this is evidence the memory helped.
    #[must_use]
    pub fn is_helpful(self) -> bool {
        matches!(self, Self::Succeeded)
    }

    /// Whether this is evidence the memory hurt.
    ///
    /// `Ignored` and `Abstained` are neither.
    #[must_use]
    pub fn is_harmful(self) -> bool {
        matches!(self, Self::Failed | Self::Corrected | Self::Reverted)
    }

    /// Whether anybody actually said how it went.
    #[must_use]
    pub fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// The word this is spelled with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Corrected => "corrected",
            Self::Reverted => "reverted",
            Self::Abstained => "abstained",
            Self::Ignored => "ignored",
            Self::Unknown => "unknown",
        }
    }
}

impl FromStr for OutcomeKind {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "corrected" => Ok(Self::Corrected),
            "reverted" => Ok(Self::Reverted),
            "abstained" => Ok(Self::Abstained),
            "ignored" => Ok(Self::Ignored),
            "unknown" => Ok(Self::Unknown),
            _ => Err(()),
        }
    }
}

impl fmt::Display for OutcomeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How sure we are that this memory had anything to do with that outcome.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    /// The memory was in the context shortly before the action. Weakest, and analysis only.
    Proximal,
    /// The action matched a step or entity the memory names.
    Structural,
    /// The caller said which memory it followed.
    Explicit,
}

impl Attribution {
    /// Whether this is strong enough to move a habit's counters on its own.
    ///
    /// Proximal is not.
    #[must_use]
    pub fn is_countable(self) -> bool {
        matches!(self, Self::Explicit | Self::Structural)
    }

    /// The word this is spelled with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proximal => "proximal",
            Self::Structural => "structural",
            Self::Explicit => "explicit",
        }
    }
}

impl FromStr for Attribution {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "proximal" => Ok(Self::Proximal),
            "structural" => Ok(Self::Structural),
            "explicit" => Ok(Self::Explicit),
            _ => Err(()),
        }
    }
}

impl fmt::Display for Attribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a memory was placed in a context.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Presentation {
    /// A current witnessed fact, stated as true.
    #[default]
    Asserted,
    /// A qualified suggestion — a habit that has worked, not a rule.
    Advisory,
    /// Historical or uncertain material, offered for reasoning rather than belief.
    Evidence,
    /// Kept and findable, never placed in an ordinary context.
    Quarantined,
}

impl Presentation {
    /// Whether a memory in this mode may be placed in an ordinary context.
    #[must_use]
    pub fn may_inject(self) -> bool {
        !matches!(self, Self::Quarantined)
    }

    /// Whether a memory in this mode may be stated as true.
    #[must_use]
    pub fn may_assert(self) -> bool {
        matches!(self, Self::Asserted)
    }

    /// The weaker of two modes.
    ///
    /// Combining is always downward: nothing promotes a quarantined memory back.
    #[must_use]
    pub fn and(self, other: Self) -> Self {
        self.max(other)
    }

    /// The word this is spelled with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asserted => "asserted",
            Self::Advisory => "advisory",
            Self::Evidence => "evidence",
            Self::Quarantined => "quarantined",
        }
    }
}

impl FromStr for Presentation {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "asserted" => Ok(Self::Asserted),
            "advisory" => Ok(Self::Advisory),
            "evidence" => Ok(Self::Evidence),
            "quarantined" => Ok(Self::Quarantined),
            _ => Err(()),
        }
    }
}

impl fmt::Display for Presentation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the ledger adds up to for one memory.
///
/// Counts rather than a score; a policy that wants a score derives one and says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Utility {
    /// Countably attributed outcomes that went well.
    pub verified_helpful: usize,
    /// Countably attributed outcomes that went badly.
    pub verified_harmful: usize,
    /// Injected and visibly not used.
    pub ignored: usize,
    /// Used, with nobody reporting how it went.
    pub unknown: usize,
    /// Proximal-only evidence, kept apart because it is not countable.
    pub proximal: usize,
    /// When the last countable outcome landed.
    pub last_verified_at: Option<crate::Timestamp>,
}

impl Utility {
    /// Whether anything countable has ever been observed.
    #[must_use]
    pub fn is_verified(&self) -> bool {
        self.verified_helpful + self.verified_harmful > 0
    }

    /// Helpful share of countable outcomes, when there are any.
    ///
    /// `None` rather than a default: "no evidence" and "evidence that it is useless" differ.
    #[must_use]
    pub fn helpfulness(&self) -> Option<f64> {
        let known = self.verified_helpful + self.verified_harmful;
        (known > 0).then(|| self.verified_helpful as f64 / known as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn being_ignored_is_not_being_harmful() {
        assert!(!OutcomeKind::Ignored.is_harmful());
        assert!(!OutcomeKind::Ignored.is_helpful());
    }

    #[test]
    fn abstaining_is_not_a_failure() {
        assert!(!OutcomeKind::Abstained.is_harmful());
    }

    #[test]
    fn unknown_is_a_real_state_and_not_a_failure() {
        assert!(!OutcomeKind::Unknown.is_known());
        assert!(!OutcomeKind::Unknown.is_harmful());
        assert!(!OutcomeKind::Unknown.is_helpful());
    }

    #[test]
    fn being_corrected_is_evidence_against() {
        assert!(OutcomeKind::Corrected.is_harmful());
        assert!(OutcomeKind::Reverted.is_harmful());
    }

    #[test]
    fn proximity_alone_never_moves_a_counter() {
        assert!(!Attribution::Proximal.is_countable());
        assert!(Attribution::Structural.is_countable());
        assert!(Attribution::Explicit.is_countable());
    }

    #[test]
    fn attribution_strengths_are_ordered() {
        assert!(Attribution::Explicit > Attribution::Structural);
        assert!(Attribution::Structural > Attribution::Proximal);
    }

    #[test]
    fn no_evidence_is_not_the_same_as_useless() {
        assert_eq!(Utility::default().helpfulness(), None);
        let harmful = Utility {
            verified_harmful: 3,
            ..Utility::default()
        };
        assert_eq!(harmful.helpfulness(), Some(0.0));
    }

    #[test]
    fn helpfulness_ignores_what_was_never_attributed() {
        let held = Utility {
            verified_harmful: 1,
            proximal: 10,
            unknown: 5,
            ..Utility::default()
        };
        assert_eq!(held.helpfulness(), Some(0.0));
    }

    #[test]
    fn every_word_survives_a_round_trip() {
        for kind in [
            OutcomeKind::Succeeded,
            OutcomeKind::Failed,
            OutcomeKind::Corrected,
            OutcomeKind::Reverted,
            OutcomeKind::Abstained,
            OutcomeKind::Ignored,
            OutcomeKind::Unknown,
        ] {
            assert_eq!(kind.as_str().parse::<OutcomeKind>(), Ok(kind));
        }
        for how in [
            Attribution::Proximal,
            Attribution::Structural,
            Attribution::Explicit,
        ] {
            assert_eq!(how.as_str().parse::<Attribution>(), Ok(how));
        }
        for mode in [
            Presentation::Asserted,
            Presentation::Advisory,
            Presentation::Evidence,
        ] {
            assert_eq!(mode.as_str().parse::<Presentation>(), Ok(mode));
        }
    }
}
