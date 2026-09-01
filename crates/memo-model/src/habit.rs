//! What a procedure claims about itself, and where it applies.
//!
//! A habit is the most dangerous kind of memory, because it is the one an agent imitates. A
//! wrong fact makes a model say something untrue; a wrong habit makes it *do* something. So the
//! rules here are stricter than anywhere else in the model.
//!
//! **One success is advisory.** A procedure derived from a single repair is a hypothesis about
//! what works, not a rule. It is offered as a suggestion and labelled as one until something
//! independent corroborates it.
//!
//! **A failure narrows, it does not prohibit.** "Do not do X" as a global rule is almost always
//! wrong — X failed *under conditions*, and a negative habit that cannot name those conditions
//! is a superstition. Every avoidance here has to say when.
//!
//! **The environment is part of the claim.** A procedure that worked on one machine is not
//! known to work on another, and pretending otherwise is how a memory layer starts confidently
//! breaking things.

use std::fmt;
use std::str::FromStr;

/// Whether a procedure says to do something or to avoid it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    /// Do this.
    #[default]
    Apply,
    /// Do not do this — under the conditions the habit names.
    Avoid,
}

impl Polarity {
    /// The word this is spelled with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Avoid => "avoid",
        }
    }
}

impl FromStr for Polarity {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "apply" => Ok(Self::Apply),
            "avoid" => Ok(Self::Avoid),
            _ => Err(()),
        }
    }
}

impl fmt::Display for Polarity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How strongly a procedure may be offered.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// Learned once, never corroborated. Offered as a suggestion.
    Advisory,
    /// Repeatedly verified, and nothing outstanding against it.
    Established,
    /// It worked before and the conditions have changed. Kept, not offered.
    Suspended,
}

impl Standing {
    /// Whether a procedure at this standing may be stated rather than suggested.
    #[must_use]
    pub fn may_assert(self) -> bool {
        matches!(self, Self::Established)
    }

    /// Whether it belongs in an ordinary context at all.
    #[must_use]
    pub fn may_offer(self) -> bool {
        !matches!(self, Self::Suspended)
    }

    /// The word this is spelled with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Advisory => "advisory",
            Self::Established => "established",
            Self::Suspended => "suspended",
        }
    }
}

impl fmt::Display for Standing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The conditions a procedure was learned under.
///
/// Cheap and explicit on purpose. Capturing the whole environment would destroy both privacy
/// and matching — every run would differ in something irrelevant, and nothing would ever match
/// anything. These are the parts that actually decide whether a command works.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Environment {
    /// Which project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Operating system, when the caller says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    /// Architecture, when the caller says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    /// Branch, when it matters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The tool and its version, when observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Whatever the caller wanted to label the run with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

impl Environment {
    /// How well this matches the conditions a procedure was learned under.
    ///
    /// Only fields both sides state are compared. An unstated field is unknown, not different —
    /// a caller that reports nothing should get a procedure offered, with its conditions shown,
    /// rather than silently excluded for failing to answer a question nobody asked.
    ///
    /// Returns `None` when there is nothing in common to compare, which is a real answer and
    /// not a score of zero.
    #[must_use]
    pub fn agreement(&self, other: &Self) -> Option<f64> {
        let pairs: [(&Option<String>, &Option<String>); 5] = [
            (&self.scope, &other.scope),
            (&self.os, &other.os),
            (&self.arch, &other.arch),
            (&self.branch, &other.branch),
            (&self.tool, &other.tool),
        ];
        let mut compared = 0;
        let mut agreed = 0;
        for (mine, theirs) in pairs {
            if let (Some(a), Some(b)) = (mine, theirs) {
                compared += 1;
                if a == b {
                    agreed += 1;
                }
            }
        }
        (compared > 0).then(|| agreed as f64 / compared as f64)
    }

    /// Whether the conditions have changed enough that a procedure should stop being offered.
    ///
    /// Deliberately conservative: only a *stated disagreement* suspends. Missing information
    /// never does, because the alternative is a memory layer that goes quiet whenever a harness
    /// stops reporting its architecture.
    #[must_use]
    pub fn has_moved_from(&self, learned: &Self) -> bool {
        self.agreement(learned).is_some_and(|share| share < 0.5)
    }

    /// The sentence a person reads.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        for (name, held) in [
            ("os", &self.os),
            ("arch", &self.arch),
            ("branch", &self.branch),
            ("tool", &self.tool),
        ] {
            if let Some(value) = held {
                parts.push(format!("{name} {value}"));
            }
        }
        parts.extend(self.labels.iter().cloned());
        if parts.is_empty() {
            "no conditions recorded".to_owned()
        } else {
            parts.join(" · ")
        }
    }
}

/// What a procedure has been observed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Record {
    /// How many times it was attempted.
    pub tried: u32,
    /// How many of those worked.
    pub worked: u32,
}

impl Record {
    /// Where this procedure stands, given its record and its conditions.
    ///
    /// A single success is advisory however good it looked. Establishment needs the thing a
    /// single run cannot provide: independent repetition, and nothing outstanding against it.
    #[must_use]
    pub fn standing(&self, moved: bool, unresolved_harm: bool) -> Standing {
        if moved {
            return Standing::Suspended;
        }
        if unresolved_harm || self.worked < 2 {
            return Standing::Advisory;
        }
        Standing::Established
    }

    /// The share of attempts that worked, when there have been any.
    #[must_use]
    pub fn success(&self) -> Option<f64> {
        (self.tried > 0).then(|| f64::from(self.worked) / f64::from(self.tried))
    }

    /// Count an attempt that worked.
    pub fn succeeded(&mut self) {
        self.tried = self.tried.saturating_add(1);
        self.worked = self.worked.saturating_add(1);
    }

    /// Count an attempt that did not.
    ///
    /// `tried` moves and `worked` does not, which is the whole arithmetic: a procedure that
    /// fails becomes less certain without being erased, and its record still says how often it
    /// used to work.
    pub fn failed(&mut self) {
        self.tried = self.tried.saturating_add(1);
    }
}

/// What a negative procedure has to name before it is worth keeping.
///
/// A vague failure must not become a global prohibition. "That did not work once" is not
/// knowledge; "that fails on this branch, and this is what to do instead" is.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Avoidance {
    /// What not to do.
    pub rejected: String,
    /// When it fails.
    pub when: String,
    /// What was observed.
    pub observed: String,
    /// What to do instead, when something verified is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instead: Option<String>,
}

impl Avoidance {
    /// Whether this is narrow enough to be worth keeping.
    ///
    /// All three of what, when and what-happened. A negative habit missing any of them is a
    /// superstition, and storing it would teach an agent to refuse things for reasons nobody
    /// can inspect.
    #[must_use]
    pub fn is_narrow(&self) -> bool {
        !self.rejected.trim().is_empty()
            && !self.when.trim().is_empty()
            && !self.observed.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(scope: &str, os: &str) -> Environment {
        Environment {
            scope: Some(scope.to_owned()),
            os: Some(os.to_owned()),
            ..Environment::default()
        }
    }

    #[test]
    fn one_success_is_never_more_than_advisory() {
        // The rule that stops a single lucky repair becoming a rule the agent follows.
        let held = Record {
            tried: 1,
            worked: 1,
        };
        assert_eq!(held.standing(false, false), Standing::Advisory);
        assert!(!Standing::Advisory.may_assert());
        assert!(Standing::Advisory.may_offer());
    }

    #[test]
    fn repetition_is_what_establishes_a_procedure() {
        let held = Record {
            tried: 2,
            worked: 2,
        };
        assert_eq!(held.standing(false, false), Standing::Established);
        assert!(Standing::Established.may_assert());
    }

    #[test]
    fn an_unresolved_harm_holds_a_procedure_back() {
        // Six successes and one unexplained failure is not an established procedure. It is a
        // procedure with something outstanding against it.
        let held = Record {
            tried: 7,
            worked: 6,
        };
        assert_eq!(held.standing(false, true), Standing::Advisory);
    }

    #[test]
    fn changed_conditions_suspend_rather_than_archive() {
        // The machine changed; what was learned did not stop being true of the old one.
        let held = Record {
            tried: 9,
            worked: 9,
        };
        assert_eq!(held.standing(true, false), Standing::Suspended);
        assert!(!Standing::Suspended.may_offer());
        assert!(!Standing::Suspended.may_assert());
    }

    #[test]
    fn failing_lowers_the_ratio_without_erasing_the_history() {
        let mut held = Record::default();
        held.succeeded();
        held.succeeded();
        held.failed();
        assert_eq!(held.tried, 3);
        assert_eq!(held.worked, 2);
        assert!((held.success().expect("some") - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn an_unstated_condition_is_unknown_and_not_different() {
        // A caller that reports nothing should get the procedure with its conditions shown, not
        // silently excluded for failing to answer a question nobody asked it.
        let learned = env("/w/p", "linux");
        let quiet = Environment::default();
        assert_eq!(quiet.agreement(&learned), None);
        assert!(!quiet.has_moved_from(&learned), "silence suspended it");
    }

    #[test]
    fn a_stated_disagreement_is_what_suspends() {
        let learned = env("/w/p", "linux");
        let elsewhere = env("/w/p", "windows");
        assert_eq!(elsewhere.agreement(&learned), Some(0.5));
        assert!(
            !elsewhere.has_moved_from(&learned),
            "half agreeing is not moved"
        );

        let entirely = env("/w/other", "windows");
        assert!(entirely.has_moved_from(&learned));
    }

    #[test]
    fn identical_conditions_agree_completely() {
        let learned = env("/w/p", "linux");
        assert_eq!(learned.agreement(&learned), Some(1.0));
    }

    #[test]
    fn a_negative_habit_must_say_what_when_and_what_happened() {
        // "That did not work once" is a superstition. Storing it would teach an agent to refuse
        // things for reasons nobody can inspect.
        let vague = Avoidance {
            rejected: "cargo test".to_owned(),
            ..Avoidance::default()
        };
        assert!(!vague.is_narrow());

        let narrow = Avoidance {
            rejected: "cargo test".to_owned(),
            when: "in this workspace, where tests are wired through make".to_owned(),
            observed: "no such command".to_owned(),
            instead: Some("make test".to_owned()),
        };
        assert!(narrow.is_narrow());
    }

    #[test]
    fn a_negative_habit_may_not_know_the_replacement_yet() {
        // Knowing what fails is useful before knowing what works. Requiring a fix would throw
        // away the more common half of what a failure teaches.
        let held = Avoidance {
            rejected: "cargo test".to_owned(),
            when: "in this workspace".to_owned(),
            observed: "no such command".to_owned(),
            instead: None,
        };
        assert!(held.is_narrow());
    }

    #[test]
    fn conditions_are_shown_rather_than_implied() {
        let held = Environment {
            os: Some("linux".to_owned()),
            tool: Some("make 4.4".to_owned()),
            labels: vec!["ci".to_owned()],
            ..Environment::default()
        };
        let said = held.describe();
        assert!(said.contains("linux") && said.contains("make 4.4") && said.contains("ci"));
        assert_eq!(Environment::default().describe(), "no conditions recorded");
    }

    #[test]
    fn a_procedure_nobody_has_tried_claims_nothing() {
        assert_eq!(Record::default().success(), None);
    }
}
