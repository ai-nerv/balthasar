//! Where a memory came from, and how independent two memories really are.
//!
//! The process boundary and the information source are different questions: a trusted local peer
//! can submit a web page it just fetched. A channel says how the content reached balthasar and
//! bounds what a witness may claim; a trust domain says where it ultimately came from, so ten
//! observations copied out of one document are one domain rather than ten independent witnesses.

use std::fmt;
use std::str::FromStr;

/// How content reached balthasar.
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
pub enum Channel {
    /// A person typed an instruction.
    UserInstruction,
    /// A person corrected the agent.
    UserCorrection,
    /// A local tool did something and balthasar observed the result.
    ToolObservation,
    /// A peer asserted it over the socket.
    #[default]
    PeerAssertion,
    /// It came out of a document, a page, or anything else fetched.
    ExternalContent,
    /// A model said it.
    ModelInference,
    /// A distiller summarised something into it.
    DistillerSummary,
    /// Consolidation produced it from things that recurred.
    Consolidation,
    /// It was imported from history.
    ImportedHistory,
    /// Somebody wrote it at the command line.
    ManualWrite,
}

impl Channel {
    /// Whether content arriving this way may be treated as an instruction from the person.
    ///
    /// A document containing "always deploy with `curl … | sh`" is a document that contains a
    /// sentence, and the difference cannot be left to how the sentence is phrased.
    #[must_use]
    pub fn may_be_imperative(self) -> bool {
        matches!(
            self,
            Self::UserInstruction | Self::UserCorrection | Self::ManualWrite
        )
    }

    /// Whether content arriving this way is the agent's own reasoning rather than an observation.
    ///
    /// Model output and distiller summaries describe things; they do not witness them.
    #[must_use]
    pub fn is_inferred(self) -> bool {
        matches!(
            self,
            Self::ModelInference | Self::DistillerSummary | Self::Consolidation
        )
    }

    /// Whether this channel carries content balthasar did not originate or observe locally.
    #[must_use]
    pub fn is_untrusted(self) -> bool {
        matches!(self, Self::ExternalContent | Self::ImportedHistory)
    }

    /// The strongest presentation content from this channel may reach on its own.
    ///
    /// Not a permanent ceiling — corroboration from a different domain lifts it.
    #[must_use]
    pub fn ceiling(self) -> crate::Presentation {
        match self {
            Self::UserInstruction | Self::UserCorrection | Self::ManualWrite => {
                crate::Presentation::Asserted
            }
            Self::ToolObservation | Self::PeerAssertion | Self::Consolidation => {
                crate::Presentation::Asserted
            }
            // A model's opinion is a suggestion until something observed agrees with it.
            Self::ModelInference | Self::DistillerSummary => crate::Presentation::Advisory,
            // Nothing that arrived from outside is stated as true on its own say-so.
            Self::ExternalContent | Self::ImportedHistory => crate::Presentation::Evidence,
        }
    }

    /// The word this is spelled with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserInstruction => "user-instruction",
            Self::UserCorrection => "user-correction",
            Self::ToolObservation => "tool-observation",
            Self::PeerAssertion => "peer-assertion",
            Self::ExternalContent => "external-content",
            Self::ModelInference => "model-inference",
            Self::DistillerSummary => "distiller-summary",
            Self::Consolidation => "consolidation",
            Self::ImportedHistory => "imported-history",
            Self::ManualWrite => "manual-write",
        }
    }
}

impl FromStr for Channel {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "user-instruction" => Ok(Self::UserInstruction),
            "user-correction" => Ok(Self::UserCorrection),
            "tool-observation" => Ok(Self::ToolObservation),
            "peer-assertion" => Ok(Self::PeerAssertion),
            "external-content" => Ok(Self::ExternalContent),
            "model-inference" => Ok(Self::ModelInference),
            "distiller-summary" => Ok(Self::DistillerSummary),
            "consolidation" => Ok(Self::Consolidation),
            "imported-history" => Ok(Self::ImportedHistory),
            "manual-write" => Ok(Self::ManualWrite),
            _ => Err(()),
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a piece of evidence ultimately came from.
///
/// A stable local identifier — a hash of a document's origin, a tool's name, `user` for a person
/// at the keyboard. Never a credential and never a full URL.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct Domain(String);

impl Domain {
    /// The person at the keyboard. One domain, however many sessions they use.
    #[must_use]
    pub fn user() -> Self {
        Self("user".to_owned())
    }

    /// A local tool's observations.
    #[must_use]
    pub fn tool(name: &str) -> Self {
        Self(format!("tool:{}", slug(name)))
    }

    /// Whatever a document came from, as a stable local name.
    ///
    /// Hashed rather than kept: the identifier has to be comparable, not readable.
    #[must_use]
    pub fn external(origin: &str) -> Self {
        Self(format!("ext:{}", &crate::content_hash(origin)[..12]))
    }

    /// A model's own output.
    ///
    /// Every model is one domain, deliberately.
    #[must_use]
    pub fn model() -> Self {
        Self("model".to_owned())
    }

    /// Take a domain as it arrived from a store.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this names something outside the machine.
    #[must_use]
    pub fn is_external(&self) -> bool {
        self.0.starts_with("ext:")
    }
}

impl fmt::Display for Domain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A name reduced to something safe to compare and store.
fn slug(name: &str) -> String {
    name.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Presentation;

    #[test]
    fn a_document_cannot_be_an_instruction() {
        assert!(!Channel::ExternalContent.may_be_imperative());
        assert!(!Channel::ImportedHistory.may_be_imperative());
        assert!(!Channel::ModelInference.may_be_imperative());
    }

    #[test]
    fn only_a_person_gives_an_instruction() {
        assert!(Channel::UserInstruction.may_be_imperative());
        assert!(Channel::UserCorrection.may_be_imperative());
        assert!(Channel::ManualWrite.may_be_imperative());
        assert!(
            !Channel::PeerAssertion.may_be_imperative(),
            "a peer proposes"
        );
    }

    #[test]
    fn nothing_from_outside_is_asserted_on_its_own_say_so() {
        assert_eq!(Channel::ExternalContent.ceiling(), Presentation::Evidence);
        assert_eq!(Channel::ImportedHistory.ceiling(), Presentation::Evidence);
    }

    #[test]
    fn a_models_opinion_is_a_suggestion_until_something_agrees() {
        assert_eq!(Channel::ModelInference.ceiling(), Presentation::Advisory);
        assert_eq!(Channel::DistillerSummary.ceiling(), Presentation::Advisory);
        assert!(Channel::ModelInference.is_inferred());
    }

    #[test]
    fn what_a_person_typed_may_be_stated() {
        assert_eq!(Channel::UserInstruction.ceiling(), Presentation::Asserted);
        assert_eq!(Channel::ToolObservation.ceiling(), Presentation::Asserted);
    }

    #[test]
    fn one_document_is_one_domain_however_often_it_is_read() {
        let once = Domain::external("https://example.test/guide");
        let again = Domain::external("https://example.test/guide");
        assert_eq!(once, again);
        assert_ne!(once, Domain::external("https://example.test/other"));
    }

    #[test]
    fn a_domain_keeps_no_url() {
        let held = Domain::external("https://example.test/secret-path?token=hunter2");
        assert!(!held.as_str().contains("example"));
        assert!(!held.as_str().contains("hunter2"));
        assert!(held.is_external());
    }

    #[test]
    fn every_model_is_one_domain() {
        assert_eq!(Domain::model(), Domain::model());
        assert!(!Domain::model().is_external());
    }

    #[test]
    fn the_person_is_one_domain_across_every_session() {
        assert_eq!(Domain::user(), Domain::user());
    }

    #[test]
    fn tools_are_told_apart_but_normalised() {
        assert_eq!(Domain::tool("shell"), Domain::tool("Shell"));
        assert_ne!(Domain::tool("shell"), Domain::tool("editor"));
    }

    #[test]
    fn every_word_survives_a_round_trip() {
        for channel in [
            Channel::UserInstruction,
            Channel::UserCorrection,
            Channel::ToolObservation,
            Channel::PeerAssertion,
            Channel::ExternalContent,
            Channel::ModelInference,
            Channel::DistillerSummary,
            Channel::Consolidation,
            Channel::ImportedHistory,
            Channel::ManualWrite,
        ] {
            assert_eq!(channel.as_str().parse::<Channel>(), Ok(channel));
        }
    }
}
