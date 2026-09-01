//! Which tier a memory is in, and who may see it.

use std::fmt;
use std::str::FromStr;

/// The five tiers, and the one that is not a tier so much as a destination.
///
/// Fixed. Lua configures thresholds, weights, sections and gates; it does not add a sixth.
/// A configurable tier count means retrieval can never be reasoned about, and every reference
/// implementation that grew one regretted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// This session only. Dies with it unless something on the ladder promotes it.
    Scratch,
    /// What happened. Durable, and decays.
    Episode,
    /// What is true. The tier that gets asserted.
    Fact,
    /// How things are done here.
    Habit,
    /// Below the floor, superseded, or forgotten on purpose. Never deleted.
    Archive,
}

impl Tier {
    /// The wire and column spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scratch => "scratch",
            Self::Episode => "episode",
            Self::Fact => "fact",
            Self::Habit => "habit",
            Self::Archive => "archive",
        }
    }

    /// Whether a memory in this tier outlives the session that made it.
    #[must_use]
    pub fn is_durable(self) -> bool {
        matches!(self, Self::Episode | Self::Fact | Self::Habit)
    }

    /// Whether a memory in this tier must carry at least one witness.
    ///
    /// Facts and habits are *asserted* to a model, so they answer for themselves. An episode
    /// is a record of something that happened and its witness is the happening.
    #[must_use]
    pub fn must_be_witnessed(self) -> bool {
        matches!(self, Self::Fact | Self::Habit)
    }
}

/// How far a memory may travel.
///
/// Enforced where memory *leaves* rather than where it is stored: the store answers faithfully,
/// and the injection boundary decides. A store that lied to its owner would be harder to debug
/// and no safer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Privacy {
    /// May be injected anywhere, including into a request to a remote model.
    #[default]
    Open,
    /// Never leaves this machine. Injected for a local model, withheld from a remote one.
    Local,
    /// Never injected at all. Found only by an explicit search by the person who owns it.
    Secret,
}

impl Privacy {
    /// The wire and column spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Local => "local",
            Self::Secret => "secret",
        }
    }

    /// Whether this may be injected into a context bound for `remote`.
    #[must_use]
    pub fn may_reach(self, remote: bool) -> bool {
        match self {
            Self::Open => true,
            Self::Local => !remote,
            Self::Secret => false,
        }
    }
}

/// What a parse of an unknown tier or privacy says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unknown(pub String);

impl fmt::Display for Unknown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}' is not one aeon knows", self.0)
    }
}

impl std::error::Error for Unknown {}

impl FromStr for Tier {
    type Err = Unknown;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "scratch" => Ok(Self::Scratch),
            "episode" => Ok(Self::Episode),
            "fact" => Ok(Self::Fact),
            "habit" => Ok(Self::Habit),
            "archive" => Ok(Self::Archive),
            other => Err(Unknown(other.to_owned())),
        }
    }
}

impl FromStr for Privacy {
    type Err = Unknown;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "open" => Ok(Self::Open),
            "local" => Ok(Self::Local),
            "secret" => Ok(Self::Secret),
            other => Err(Unknown(other.to_owned())),
        }
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for Privacy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tier_round_trips_through_its_column_spelling() {
        for tier in [
            Tier::Scratch,
            Tier::Episode,
            Tier::Fact,
            Tier::Habit,
            Tier::Archive,
        ] {
            assert_eq!(tier.as_str().parse(), Ok(tier));
        }
    }

    #[test]
    fn scratch_and_archive_are_not_durable() {
        assert!(!Tier::Scratch.is_durable());
        assert!(!Tier::Archive.is_durable());
        assert!(Tier::Fact.is_durable());
    }

    #[test]
    fn only_asserted_tiers_must_answer_for_themselves() {
        // An episode's witness is the happening. A fact is presented as true, so it owes
        // an answer to "how do you know".
        assert!(Tier::Fact.must_be_witnessed());
        assert!(Tier::Habit.must_be_witnessed());
        assert!(!Tier::Episode.must_be_witnessed());
    }

    #[test]
    fn a_local_memory_is_withheld_from_a_remote_model_only() {
        assert!(Privacy::Local.may_reach(false));
        assert!(!Privacy::Local.may_reach(true));
        assert!(Privacy::Open.may_reach(true));
        assert!(!Privacy::Secret.may_reach(false));
    }

    #[test]
    fn an_unknown_tier_says_which_one() {
        assert_eq!("semantic".parse::<Tier>(), Err(Unknown("semantic".into())));
    }
}
