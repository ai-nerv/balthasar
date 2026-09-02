//! What comes off the ladder, and the gate it passes.

use memo_model::{Body, Tier, Timestamp, WitnessKind};

/// Something a path across the gate proposes keeping.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// What it says.
    pub body: Body,
    /// Which tier it wants to be.
    pub tier: Tier,
    /// What produced it.
    pub witness: WitnessKind,
    /// Where in the transcript, so the evidence can point at it.
    pub cursor: Option<u64>,
    /// How fast it should be allowed to fade.
    pub importance: memo_model::Importance,
    /// Which extractor made it, for `witness.note`.
    pub from: String,
    /// Whether it should be kept from fading.
    ///
    /// Only somebody insisting sets this. A person who has been asked the same thing repeatedly
    /// is not asking for a memory that decays.
    pub pinned: bool,
}

impl Candidate {
    /// A proposal from one extractor.
    #[must_use]
    pub fn new(body: Body, tier: Tier, witness: WitnessKind, from: impl Into<String>) -> Self {
        Self {
            body,
            tier,
            witness,
            cursor: None,
            importance: memo_model::Importance::Normal,
            from: from.into(),
            pinned: false,
        }
    }

    /// Kept from fading, because somebody insisted.
    #[must_use]
    pub fn pinned(mut self) -> Self {
        self.pinned = true;
        self.importance = memo_model::Importance::Critical;
        self
    }

    /// Where it was seen.
    #[must_use]
    pub fn at(mut self, cursor: Option<u64>) -> Self {
        self.cursor = cursor;
        self
    }

    /// How fast it may fade.
    #[must_use]
    pub fn fading(mut self, importance: memo_model::Importance) -> Self {
        self.importance = importance;
        self
    }

    /// What it says, as one line.
    #[must_use]
    pub fn text(&self) -> String {
        self.body.text()
    }

    /// What this candidate scores before anything else has seen it.
    ///
    /// One witness, so this is the weight of the path that produced it. The whole design of the
    /// weights is here: what a person asked for crosses alone, what merely scrolled out of a
    /// window does not.
    #[must_use]
    pub fn score(&self, weight_of: impl Fn(WitnessKind) -> f64) -> f64 {
        weight_of(self.witness)
    }

    /// As a table a Lua gate can read.
    #[must_use]
    pub fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "text": self.text(),
            "tier": self.tier.as_str(),
            "witness": self.witness.as_str(),
            "importance": self.importance.as_str(),
            "from": self.from,
            "cursor": self.cursor,
            "pinned": self.pinned,
        })
    }
}

/// What the gate decided.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// It crosses.
    Promote {
        /// How fast it may fade, possibly amended by a handler.
        importance: memo_model::Importance,
        /// Whether it is pinned.
        pinned: bool,
    },
    /// It waits in scratch for a second witness rather than dying with the session.
    Hold,
    /// It stays where it is and goes no further.
    Refuse {
        /// Why, so `memo ingest --explain` can say.
        reason: String,
    },
}

/// Decide a candidate against the floors, before any configuration has its say.
///
/// Three outcomes rather than two, because "not yet" and "no" are different answers and
/// collapsing them is what makes a memory system either forgetful or credulous.
#[must_use]
pub fn weigh(score: f64, promote: f64, hold: f64) -> Verdict {
    if score >= promote {
        Verdict::Promote {
            importance: memo_model::Importance::Normal,
            pinned: false,
        }
    } else if score >= hold {
        Verdict::Hold
    } else {
        Verdict::Refuse {
            reason: format!("{score:.2} is under the {hold:.2} worth holding"),
        }
    }
}

/// How a candidate lands once everything has had its say.
#[derive(Debug, Clone, PartialEq)]
pub struct Decided {
    /// What was proposed.
    pub candidate: Candidate,
    /// What the gate said.
    pub verdict: Verdict,
    /// When.
    pub at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(kind: WitnessKind) -> Candidate {
        Candidate::new(
            Body::fact("project", "test_command", "make test"),
            Tier::Fact,
            kind,
            "test",
        )
    }

    #[test]
    fn what_a_person_asked_for_crosses_alone() {
        let score = candidate(WitnessKind::Imperative).score(WitnessKind::weight);
        assert!(matches!(weigh(score, 0.5, 0.3), Verdict::Promote { .. }));
    }

    #[test]
    fn what_merely_left_the_window_waits_instead() {
        // §5.1's whole point: distillation is worth 0.3, the promotion floor is 0.5, and the
        // gap between them is the main defence against believing something said once.
        let score = candidate(WitnessKind::Distillation).score(WitnessKind::weight);
        assert_eq!(weigh(score, 0.5, 0.3), Verdict::Hold);
    }

    #[test]
    fn a_sleep_pass_alone_is_refused() {
        let score = candidate(WitnessKind::Consolidation).score(WitnessKind::weight);
        assert!(matches!(weigh(score, 0.5, 0.3), Verdict::Refuse { .. }));
    }

    #[test]
    fn a_refusal_says_why() {
        let Verdict::Refuse { reason } = weigh(0.1, 0.5, 0.3) else {
            panic!("expected a refusal");
        };
        assert!(
            reason.contains("0.10") && reason.contains("0.30"),
            "{reason}"
        );
    }

    #[test]
    fn a_candidate_reads_as_a_table_a_gate_can_use() {
        // The shipped gate reads `text`, `tier` and `witness`. If any of them stopped being
        // there the gate would silently stop firing.
        let json = candidate(WitnessKind::Cost).as_json();
        for field in ["text", "tier", "witness", "importance", "from"] {
            assert!(json.get(field).is_some(), "{field} is missing");
        }
    }
}
