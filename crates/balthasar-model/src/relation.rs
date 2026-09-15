//! Derived relationships between memories.
//!
//! Kept apart from the asserted links in [`crate::LinkRelation`]: a derived edge is a retrieval
//! aid, and throwing every one away costs nothing but a rebuild. A derived edge never raises
//! confidence, and every edge names where it came from.

use std::fmt;
use std::str::FromStr;

/// What kind of relationship one memory has to another.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum View {
    /// This happened before that.
    Before,
    /// This happened after that.
    After,
    /// Their spans overlap.
    Overlaps,
    /// This led to that.
    Caused,
    /// This fixed that.
    Resolved,
    /// This failed because of that.
    FailedBecause,
    /// They are about the same thing.
    SameEntity,
    /// They say close to the same thing.
    SimilarTo,
    /// They were working towards the same end.
    SameGoal,
    /// They ran under the same conditions.
    SameEnvironment,
}

impl View {
    /// Which family this belongs to.
    #[must_use]
    pub fn family(self) -> Family {
        match self {
            Self::Before | Self::After | Self::Overlaps => Family::Temporal,
            Self::Caused | Self::Resolved | Self::FailedBecause => Family::Causal,
            Self::SameEntity => Family::Entity,
            Self::SimilarTo | Self::SameGoal | Self::SameEnvironment => Family::Semantic,
        }
    }

    /// The edge pointing the other way, when there is one.
    ///
    /// Causal edges are deliberately one-directional.
    #[must_use]
    pub fn inverse(self) -> Option<Self> {
        match self {
            Self::Before => Some(Self::After),
            Self::After => Some(Self::Before),
            Self::Overlaps => Some(Self::Overlaps),
            Self::SameEntity => Some(Self::SameEntity),
            Self::SimilarTo => Some(Self::SimilarTo),
            Self::SameGoal => Some(Self::SameGoal),
            Self::SameEnvironment => Some(Self::SameEnvironment),
            Self::Caused | Self::Resolved | Self::FailedBecause => None,
        }
    }

    /// The column and wire spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::After => "after",
            Self::Overlaps => "overlaps",
            Self::Caused => "caused",
            Self::Resolved => "resolved",
            Self::FailedBecause => "failed-because",
            Self::SameEntity => "same-entity",
            Self::SimilarTo => "similar-to",
            Self::SameGoal => "same-goal",
            Self::SameEnvironment => "same-environment",
        }
    }
}

impl FromStr for View {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "before" => Ok(Self::Before),
            "after" => Ok(Self::After),
            "overlaps" => Ok(Self::Overlaps),
            "caused" => Ok(Self::Caused),
            "resolved" => Ok(Self::Resolved),
            "failed-because" => Ok(Self::FailedBecause),
            "same-entity" => Ok(Self::SameEntity),
            "similar-to" => Ok(Self::SimilarTo),
            "same-goal" => Ok(Self::SameGoal),
            "same-environment" => Ok(Self::SameEnvironment),
            _ => Err(()),
        }
    }
}

impl fmt::Display for View {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which kind of question an edge helps answer.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// When, and in what order.
    Temporal,
    /// Why, and what fixed it.
    Causal,
    /// What it is about.
    Entity,
    /// What it resembles.
    Semantic,
}

impl Family {
    /// The word this is spelled with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Temporal => "temporal",
            Self::Causal => "causal",
            Self::Entity => "entity",
            Self::Semantic => "semantic",
        }
    }

    /// Whether this family survives with no embedder.
    #[must_use]
    pub fn needs_no_embedder(self) -> bool {
        !matches!(self, Self::Semantic)
    }
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What produced an edge.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Derivation {
    /// A deterministic rule over clocks, spans and slots.
    Rule,
    /// The shape of the transcript itself — a failure, then a different success.
    Structure,
    /// Cosine similarity between embeddings.
    Embedding,
    /// A model proposed it.
    Distiller,
    /// A person asserted it.
    Manual,
}

impl Derivation {
    /// Whether this can be recomputed from what the store already holds.
    ///
    /// A manual assertion cannot: rebuilding the index must not throw it away.
    #[must_use]
    pub fn is_rebuildable(self) -> bool {
        !matches!(self, Self::Manual)
    }

    /// Whether this needs a model or an embedder to exist.
    #[must_use]
    pub fn is_optional(self) -> bool {
        matches!(self, Self::Embedding | Self::Distiller)
    }

    /// The word this is spelled with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Structure => "structure",
            Self::Embedding => "embedding",
            Self::Distiller => "distiller",
            Self::Manual => "manual",
        }
    }
}

impl FromStr for Derivation {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "rule" => Ok(Self::Rule),
            "structure" => Ok(Self::Structure),
            "embedding" => Ok(Self::Embedding),
            "distiller" => Ok(Self::Distiller),
            "manual" => Ok(Self::Manual),
            _ => Err(()),
        }
    }
}

impl fmt::Display for Derivation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One derived edge.
#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    /// Where it points from.
    pub from: crate::MemoryId,
    /// Where it points to.
    pub to: crate::MemoryId,
    /// What kind.
    pub view: View,
    /// How strongly, between zero and one.
    pub weight: f64,
    /// What produced it.
    pub source: Derivation,
    /// Which version of that derivation.
    pub derivation_version: u32,
    /// Where in the transcript the case for it is, when there is one.
    pub evidence_cursor: Option<u64>,
    /// When it was derived.
    pub created_at: crate::Timestamp,
}

impl Relation {
    /// The sentence a person reads, with its provenance attached.
    #[must_use]
    pub fn explain(&self) -> String {
        format!("{} (from {})", self.view, self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_causal_edge_does_not_point_both_ways() {
        assert_eq!(View::Resolved.inverse(), None);
        assert_eq!(View::Caused.inverse(), None);
        assert_eq!(View::FailedBecause.inverse(), None);
    }

    #[test]
    fn order_edges_are_each_others_opposite() {
        assert_eq!(View::Before.inverse(), Some(View::After));
        assert_eq!(View::After.inverse(), Some(View::Before));
    }

    #[test]
    fn sameness_edges_are_their_own_opposite() {
        for view in [
            View::Overlaps,
            View::SameEntity,
            View::SimilarTo,
            View::SameGoal,
            View::SameEnvironment,
        ] {
            assert_eq!(view.inverse(), Some(view));
        }
    }

    #[test]
    fn three_families_survive_with_no_embedder() {
        assert!(Family::Temporal.needs_no_embedder());
        assert!(Family::Causal.needs_no_embedder());
        assert!(Family::Entity.needs_no_embedder());
        assert!(!Family::Semantic.needs_no_embedder());
    }

    #[test]
    fn what_a_person_asserted_is_not_rebuildable() {
        assert!(!Derivation::Manual.is_rebuildable());
        for source in [
            Derivation::Rule,
            Derivation::Structure,
            Derivation::Embedding,
            Derivation::Distiller,
        ] {
            assert!(source.is_rebuildable(), "{source}");
        }
    }

    #[test]
    fn optional_derivations_are_the_ones_needing_something_extra() {
        assert!(Derivation::Embedding.is_optional());
        assert!(Derivation::Distiller.is_optional());
        assert!(!Derivation::Rule.is_optional());
        assert!(!Derivation::Structure.is_optional());
    }

    #[test]
    fn an_edge_never_prints_its_label_without_its_source() {
        let held = Relation {
            from: crate::MemoryId::new("a"),
            to: crate::MemoryId::new("b"),
            view: View::Resolved,
            weight: 0.8,
            source: Derivation::Structure,
            derivation_version: 1,
            evidence_cursor: Some(4),
            created_at: 0,
        };
        let said = held.explain();
        assert!(said.contains("resolved"));
        assert!(said.contains("structure"), "{said}");
    }

    #[test]
    fn every_word_survives_a_round_trip() {
        for view in [
            View::Before,
            View::After,
            View::Overlaps,
            View::Caused,
            View::Resolved,
            View::FailedBecause,
            View::SameEntity,
            View::SimilarTo,
            View::SameGoal,
            View::SameEnvironment,
        ] {
            assert_eq!(view.as_str().parse::<View>(), Ok(view));
        }
        for source in [
            Derivation::Rule,
            Derivation::Structure,
            Derivation::Embedding,
            Derivation::Distiller,
            Derivation::Manual,
        ] {
            assert_eq!(source.as_str().parse::<Derivation>(), Ok(source));
        }
    }
}
