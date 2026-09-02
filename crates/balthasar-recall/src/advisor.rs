//! Where a learned policy is allowed to have an opinion, and where it is not.
//!
//! There is no model here. What is here is the boundary a model would sit behind — the list of
//! things it may propose, the shorter list of things it may never do, and the deterministic
//! answer that is used whenever it is slow, absent, or wrong.
//!
//! Building the boundary before the model is the point. A learned component added to a working
//! system tends to arrive with its authority already assumed; deciding the limits first, while
//! nothing depends on them, is the only time the decision is cheap.
//!
//! **Nothing here is on the turn path.** Every function returns a *proposal*, and the caller is
//! free to ignore it — which is what the timeout in [`Advisory::or`] makes concrete.

use crate::Policy;
use balthasar_model::{Presentation, Tier};

/// The stages a learned policy passes through, in order.
///
/// No automatic promotion. Each step needs a written comparison, because the failure mode of
/// learned components is that they get promoted by inertia rather than by evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Stage {
    /// Score historical candidate sets. Touches nothing live.
    #[default]
    Replay,
    /// Run beside the deterministic policy; the answer is discarded.
    Shadow,
    /// Surface suggestions in reports, where a person reads them.
    Advisory,
    /// Allowed to make bounded retrieval choices, having earned it.
    Opted,
}

impl Stage {
    /// Whether a proposal at this stage may change what a caller is served.
    #[must_use]
    pub fn may_serve(self) -> bool {
        matches!(self, Self::Opted)
    }

    /// Whether it may appear where a person will see it.
    #[must_use]
    pub fn may_show(self) -> bool {
        matches!(self, Self::Advisory | Self::Opted)
    }

    /// The word this is spelled with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Replay => "replay",
            Self::Shadow => "shadow",
            Self::Advisory => "advisory",
            Self::Opted => "opted-in",
        }
    }
}

/// Something a learned policy may propose.
///
/// Every variant is a suggestion about *retrieval or shape*. There is deliberately no variant
/// for confidence, for assertion, for purging, or for minting a witness — see [`Forbidden`].
#[derive(Debug, Clone, PartialEq)]
pub enum Proposal {
    /// Use this retrieval policy.
    Retrieval(Policy),
    /// This candidate is worth storing, or is not.
    Keep(bool),
    /// Store it at this tier.
    AtTier(Tier),
    /// A boundary belongs at this cursor.
    Boundary(u64),
    /// These two memories are related this way.
    Related(balthasar_model::View),
    /// Show it this way — but only downward. See [`Advisory::bounded`].
    Present(Presentation),
}

/// The things a learned policy may never do, whatever stage it has reached.
///
/// Written as data so the list can be asserted against rather than remembered. Each of these is
/// a way a model could make itself authoritative rather than useful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forbidden {
    /// Remove anything.
    Purge,
    /// Claim a person said something.
    MintImperative,
    /// Set a number that is supposed to be derived.
    AssignConfidence,
    /// Promote something to being stated as true.
    Assert,
    /// Reach further than the caller could.
    EscalateScope,
    /// Send a secret somewhere remote.
    ExposeSecret,
    /// Run something.
    Execute,
    /// Change what was recorded as having happened.
    RewriteHistory,
    /// Change how an action was judged.
    OverwriteOutcome,
}

impl Forbidden {
    /// Everything a learned policy may never do.
    #[must_use]
    pub fn every() -> &'static [Forbidden] {
        &[
            Self::Purge,
            Self::MintImperative,
            Self::AssignConfidence,
            Self::Assert,
            Self::EscalateScope,
            Self::ExposeSecret,
            Self::Execute,
            Self::RewriteHistory,
            Self::OverwriteOutcome,
        ]
    }

    /// Why, in a sentence.
    #[must_use]
    pub fn because(self) -> &'static str {
        match self {
            Self::Purge => "forgetting is irreversible and belongs to the person",
            Self::MintImperative => "only a person gives an instruction",
            Self::AssignConfidence => "confidence is derived from evidence or it means nothing",
            Self::Assert => "assertion requires witnesses, not a prediction",
            Self::EscalateScope => "a policy cannot reach where its caller cannot",
            Self::ExposeSecret => "a secret's boundary is not a retrieval decision",
            Self::Execute => "balthasar describes procedures and never runs them",
            Self::RewriteHistory => "the transcript is what happened",
            Self::OverwriteOutcome => "how it went is an observation, not a prediction",
        }
    }
}

/// A proposal, and what happens when it cannot be had.
#[derive(Debug, Clone, PartialEq)]
pub struct Advisory {
    /// Which stage the policy making it has reached.
    pub stage: Stage,
    /// What it proposes.
    pub proposal: Proposal,
    /// How confident the model claims to be. Recorded, never trusted.
    pub claimed: f64,
}

impl Advisory {
    /// The retrieval policy to actually use.
    ///
    /// `fallback` is the deterministic answer, and it wins in every case except one: a policy
    /// that has been explicitly opted into, proposing something within its bounds. A learned
    /// component that is slow, missing, or in an earlier stage costs nothing — the caller gets
    /// the rules, which is what it would have got anyway.
    #[must_use]
    pub fn or(held: Option<&Self>, fallback: Policy) -> Policy {
        match held {
            Some(Self {
                stage,
                proposal: Proposal::Retrieval(policy),
                ..
            }) if stage.may_serve() => policy.clone(),
            _ => fallback,
        }
    }

    /// A presentation proposal, clamped so it can only ever weaken.
    ///
    /// The one proposal that touches safety. A model may say "this looks like an attack, show
    /// it as evidence" and be listened to; it may not say "this is fine, assert it", because
    /// then an attacker who can influence the model can influence what is asserted.
    #[must_use]
    pub fn bounded(proposed: Presentation, deterministic: Presentation) -> Presentation {
        deterministic.and(proposed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advisory(stage: Stage) -> Advisory {
        Advisory {
            stage,
            proposal: Proposal::Retrieval(Policy::lexical_only()),
            claimed: 0.99,
        }
    }

    #[test]
    fn nothing_short_of_opting_in_changes_what_is_served() {
        // The whole point of the staging. A model in replay, shadow or advisory mode is
        // observed, not obeyed, and no amount of claimed confidence moves it along.
        for stage in [Stage::Replay, Stage::Shadow, Stage::Advisory] {
            let held = advisory(stage);
            let used = Advisory::or(Some(&held), Policy::balanced());
            assert_eq!(used.name, "balanced", "{stage:?} was served");
        }
    }

    #[test]
    fn an_opted_in_policy_is_finally_listened_to() {
        let held = advisory(Stage::Opted);
        let used = Advisory::or(Some(&held), Policy::balanced());
        assert_eq!(used.name, "lexical-only");
    }

    #[test]
    fn an_absent_advisor_costs_nothing() {
        // The timeout case. A learned component that is slow or missing must leave the caller
        // with exactly what the rules would have given it.
        let used = Advisory::or(None, Policy::balanced());
        assert_eq!(used.name, "balanced");
    }

    #[test]
    fn a_model_may_weaken_a_presentation_and_never_strengthen_one() {
        // A model that could promote to asserted would let anyone who can influence the model
        // influence what balthasar states as true.
        assert_eq!(
            Advisory::bounded(Presentation::Quarantined, Presentation::Asserted),
            Presentation::Quarantined,
            "it may say this looks dangerous"
        );
        assert_eq!(
            Advisory::bounded(Presentation::Asserted, Presentation::Quarantined),
            Presentation::Quarantined,
            "and may not say this is fine"
        );
        assert_eq!(
            Advisory::bounded(Presentation::Asserted, Presentation::Evidence),
            Presentation::Evidence
        );
    }

    #[test]
    fn every_forbidden_thing_says_why() {
        // The list is data so it can be asserted against rather than remembered, and a reason
        // nobody wrote down is a limit somebody will argue away later.
        for held in Forbidden::every() {
            assert!(!held.because().is_empty(), "{held:?}");
        }
        assert_eq!(Forbidden::every().len(), 9);
    }

    #[test]
    fn no_proposal_can_express_a_forbidden_thing() {
        // The type is the enforcement. There is no `Proposal::Assert`, no
        // `Proposal::Confidence`, no `Proposal::Purge` — a learned policy cannot ask for them
        // because there is no way to say them.
        let every = [
            Proposal::Retrieval(Policy::balanced()),
            Proposal::Keep(true),
            Proposal::AtTier(Tier::Fact),
            Proposal::Boundary(4),
            Proposal::Related(balthasar_model::View::SameEntity),
            Proposal::Present(Presentation::Evidence),
        ];
        // Every variant is about retrieval or shape. None carries a confidence, a witness kind,
        // or an instruction to remove something.
        for proposal in &every {
            let said = format!("{proposal:?}").to_lowercase();
            assert!(!said.contains("purge"), "{said}");
            assert!(!said.contains("imperative"), "{said}");
            assert!(!said.contains("confidence"), "{said}");
        }
    }

    #[test]
    fn stages_only_move_forward_deliberately() {
        assert!(Stage::Replay < Stage::Shadow);
        assert!(Stage::Shadow < Stage::Advisory);
        assert!(Stage::Advisory < Stage::Opted);
        assert!(!Stage::Replay.may_show(), "replay is not shown to anybody");
        assert!(Stage::Advisory.may_show());
        assert!(!Stage::Advisory.may_serve());
    }

    #[test]
    fn a_claimed_confidence_is_recorded_and_not_acted_on() {
        // A model claiming 0.99 in shadow mode is still in shadow mode.
        let held = Advisory {
            stage: Stage::Shadow,
            proposal: Proposal::Retrieval(Policy::lexical_only()),
            claimed: 1.0,
        };
        assert_eq!(
            Advisory::or(Some(&held), Policy::balanced()).name,
            "balanced"
        );
    }
}
