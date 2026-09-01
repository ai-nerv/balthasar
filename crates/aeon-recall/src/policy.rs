//! Which retrieval behaviour a query gets, and how a different one is tried safely.
//!
//! A policy decides where candidates come from, how far to walk, and how the signals are
//! weighted. It does **not** decide what may be asserted, what may be injected, or what a peer
//! is allowed to see — those are hard constraints outside every policy, because a policy is a
//! thing you experiment with and a constraint is a thing you do not.
//!
//! **Shadow mode** is what makes experimenting safe. A shadow policy computes its own candidate
//! list and its answer is thrown away; only the comparison is kept. Nobody is served an
//! experiment, and the data to judge one accumulates anyway.

use crate::Shape;
use aeon_model::Family;

/// A named retrieval behaviour.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// What it is called, in explanations and in comparisons.
    pub name: &'static str,
    /// Which relationship families it walks.
    pub families: Vec<Family>,
    /// How far it walks.
    pub hops: usize,
    /// How many candidates it may gather.
    pub candidates: usize,
    /// Whether it may use vectors when they exist.
    pub vectors: bool,
    /// Whether it may reach into the archive.
    pub archive: bool,
    /// Why this policy was chosen, for `--explain`.
    pub because: &'static str,
}

impl Policy {
    /// The shipped default: what aeon did before policies existed.
    #[must_use]
    pub fn balanced() -> Self {
        Self {
            name: "balanced",
            families: Vec::new(),
            hops: 1,
            candidates: 500,
            vectors: true,
            archive: false,
            because: "nothing in the query asked for anything else",
        }
    }

    /// The permanent floor and the control group.
    ///
    /// No vectors, no traversal. Every experiment is measured against this, and if a clever
    /// policy cannot beat it then the cleverness is not worth its latency.
    #[must_use]
    pub fn lexical_only() -> Self {
        Self {
            name: "lexical-only",
            families: Vec::new(),
            hops: 0,
            candidates: 500,
            vectors: false,
            archive: false,
            because: "the control group: full-text search and nothing else",
        }
    }

    /// What a query's shape asks for.
    #[must_use]
    pub fn for_shape(shape: Shape) -> Self {
        match shape {
            Shape::Plain => Self::balanced(),
            Shape::Current => Self {
                name: "current-fact",
                families: Vec::new(),
                hops: 0,
                candidates: 200,
                vectors: true,
                archive: false,
                because: shape.because(),
            },
            Shape::Temporal => Self {
                name: "temporal",
                families: vec![Family::Temporal],
                hops: 1,
                candidates: 400,
                vectors: true,
                // What happened before something is often no longer live.
                archive: true,
                because: shape.because(),
            },
            Shape::Causal => Self {
                name: "repair",
                families: vec![Family::Causal, Family::Temporal],
                hops: 1,
                candidates: 400,
                vectors: true,
                archive: true,
                because: shape.because(),
            },
            Shape::Entity => Self {
                name: "entity",
                families: vec![Family::Entity],
                hops: 1,
                candidates: 500,
                vectors: true,
                archive: false,
                because: shape.because(),
            },
            Shape::Semantic => Self {
                name: "similar",
                families: vec![Family::Semantic, Family::Entity],
                hops: 1,
                candidates: 500,
                vectors: true,
                archive: false,
                because: shape.because(),
            },
            Shape::Procedural => Self {
                name: "procedure",
                families: vec![Family::Entity, Family::Causal],
                hops: 1,
                candidates: 400,
                vectors: true,
                archive: false,
                because: shape.because(),
            },
        }
    }

    /// The same policy with vectors unavailable.
    ///
    /// Not a different policy — the same one, degraded. A timeout or a missing embedder must
    /// fall back to something that still works rather than to something that behaves
    /// differently in ways nobody predicted.
    #[must_use]
    pub fn without_vectors(mut self) -> Self {
        self.vectors = false;
        self
    }

    /// Whether this policy would do anything a plain search does not.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        self.families.is_empty() && !self.archive
    }
}

/// What running a shadow policy beside the real one found.
///
/// Bounded on purpose. A shadow comparison that stored both result sets would be a second copy
/// of the store's contents keyed by query, which is a privacy problem wearing a research hat.
#[derive(Debug, Clone, PartialEq)]
pub struct Shadow {
    /// The policy that was served.
    pub served: &'static str,
    /// The policy that was only computed.
    pub shadowed: &'static str,
    /// How many of the served results the shadow also found, over how many were served.
    pub overlap: f64,
    /// How many candidates the shadow would have returned.
    pub returned: usize,
    /// What the shadow's answer would have cost, in tokens.
    pub tokens: usize,
    /// How long the shadow took.
    pub micros: u64,
}

impl Shadow {
    /// Compare two result sets by identity.
    ///
    /// Overlap of one means the shadow would have changed nothing and is not worth serving.
    /// Overlap of zero means it is a different system, and the outcome data will say which is
    /// better long before anybody's intuition does.
    #[must_use]
    pub fn of(
        served: &Policy,
        shadowed: &Policy,
        served_ids: &[String],
        shadow_ids: &[String],
        tokens: usize,
        micros: u64,
    ) -> Self {
        let shared = served_ids
            .iter()
            .filter(|id| shadow_ids.contains(id))
            .count();
        let overlap = if served_ids.is_empty() {
            0.0
        } else {
            shared as f64 / served_ids.len() as f64
        };
        Self {
            served: served.name,
            shadowed: shadowed.name,
            overlap,
            returned: shadow_ids.len(),
            tokens,
            micros,
        }
    }

    /// Whether the shadow found anything the served policy did not.
    #[must_use]
    pub fn differs(&self) -> bool {
        self.overlap < 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_query_gets_what_aeon_always_did() {
        let held = Policy::for_shape(Shape::Plain);
        assert_eq!(held.name, "balanced");
        assert!(held.is_plain(), "it walks nothing and costs nothing extra");
    }

    #[test]
    fn what_is_true_now_does_not_reach_into_the_archive() {
        // A current-fact question answered with archived facts is answered wrongly.
        let held = Policy::for_shape(Shape::Current);
        assert!(!held.archive);
        assert_eq!(held.hops, 0);
    }

    #[test]
    fn a_question_about_the_past_may_look_in_the_archive() {
        // What happened before something is often no longer live, and refusing to look there
        // makes the temporal family useless exactly where it is needed.
        assert!(Policy::for_shape(Shape::Temporal).archive);
        assert!(Policy::for_shape(Shape::Causal).archive);
    }

    #[test]
    fn the_control_group_uses_nothing_clever() {
        // Every experiment is measured against this. If a policy cannot beat full-text search
        // then its latency is not buying anything.
        let held = Policy::lexical_only();
        assert!(!held.vectors);
        assert_eq!(held.hops, 0);
        assert!(held.families.is_empty());
    }

    #[test]
    fn degrading_keeps_the_policy_rather_than_swapping_it() {
        // A missing embedder must not silently become a different retrieval strategy.
        let held = Policy::for_shape(Shape::Entity);
        let degraded = held.clone().without_vectors();
        assert_eq!(degraded.name, held.name);
        assert_eq!(degraded.families, held.families);
        assert!(!degraded.vectors);
    }

    #[test]
    fn every_policy_can_say_why_it_was_chosen() {
        for shape in [
            Shape::Plain,
            Shape::Current,
            Shape::Temporal,
            Shape::Causal,
            Shape::Entity,
            Shape::Semantic,
            Shape::Procedural,
        ] {
            let held = Policy::for_shape(shape);
            assert!(!held.because.is_empty(), "{shape:?}");
            assert!(!held.name.is_empty());
        }
    }

    #[test]
    fn a_shadow_that_agrees_completely_is_not_worth_serving() {
        let served = Policy::balanced();
        let shadow = Policy::lexical_only();
        let ids = vec!["a".to_owned(), "b".to_owned()];
        let held = Shadow::of(&served, &shadow, &ids, &ids, 40, 900);

        assert!((held.overlap - 1.0).abs() < f64::EPSILON);
        assert!(!held.differs());
    }

    #[test]
    fn a_shadow_that_finds_something_else_says_so() {
        let served = Policy::balanced();
        let shadow = Policy::for_shape(Shape::Entity);
        let served_ids = vec!["a".to_owned(), "b".to_owned()];
        let shadow_ids = vec!["a".to_owned(), "c".to_owned(), "d".to_owned()];
        let held = Shadow::of(&served, &shadow, &served_ids, &shadow_ids, 60, 1200);

        assert!((held.overlap - 0.5).abs() < f64::EPSILON);
        assert!(held.differs());
        assert_eq!(held.returned, 3);
    }

    #[test]
    fn a_shadow_of_nothing_is_not_a_perfect_score() {
        // An empty served set with an empty shadow must not read as complete agreement, or a
        // policy that returns nothing would look like the best one.
        let served = Policy::balanced();
        let shadow = Policy::lexical_only();
        let held = Shadow::of(&served, &shadow, &[], &[], 0, 100);
        assert!((held.overlap - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_shadow_records_what_it_would_have_cost() {
        // A policy that wins by injecting twice as much has not won, and without the token
        // count nothing would say so.
        let held = Shadow::of(
            &Policy::balanced(),
            &Policy::for_shape(Shape::Entity),
            &["a".to_owned()],
            &["a".to_owned(), "b".to_owned()],
            180,
            2400,
        );
        assert_eq!(held.tokens, 180);
        assert_eq!(held.micros, 2400);
    }
}
