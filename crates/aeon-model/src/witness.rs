//! How a memory knows what it knows.
//!
//! Every durable memory carries the evidence that promoted it. This is the record of one piece
//! of that evidence, and the reason `aeon why` can print an argument rather than a number.

use crate::{ScopeId, SessionId, Timestamp, WitnessId};
use std::fmt;
use std::str::FromStr;

/// Which of the six paths across the gate produced this evidence.
///
/// The weights are the plan's, and the ordering they impose is the design: what a person asked
/// for outranks what they corrected, which outranks what cost something to learn, which
/// outranks what merely recurred, which outranks what happened to scroll out of a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WitnessKind {
    /// The user said to remember it. Crosses alone, and pins.
    Imperative,
    /// The user corrected something. Crosses alone.
    Correction,
    /// It was expensive to learn — a repair, a slow command, a file read again and again.
    Cost,
    /// It recurred across distinct sessions.
    Repetition,
    /// It fell out of a span leaving the context window. Deliberately below the floor.
    Distillation,
    /// A sleep pass produced it. Never crosses alone.
    Consolidation,
    /// Typed at the CLI, or written by a peer. Proposes; does not assert.
    Manual,
}

impl WitnessKind {
    /// What one such witness is worth before recency is applied.
    #[must_use]
    pub fn weight(self) -> f64 {
        match self {
            Self::Imperative => 1.0,
            Self::Correction => 0.8,
            Self::Cost => 0.5,
            Self::Repetition => 0.25,
            Self::Distillation => 0.3,
            Self::Consolidation => 0.2,
            Self::Manual => 0.4,
        }
    }

    /// Whether one witness of this kind is enough to leave the session it was learned in.
    ///
    /// Measured against the promotion floor rather than asserted, so the two cannot drift:
    /// changing a weight changes what crosses, which is the intent.
    #[must_use]
    pub fn crosses_alone(self, floor: f64) -> bool {
        self.weight() >= floor
    }

    /// Whether this kind pins by default.
    #[must_use]
    pub fn pins(self) -> bool {
        matches!(self, Self::Imperative)
    }

    /// The wire and column spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Imperative => "imperative",
            Self::Correction => "correction",
            Self::Cost => "cost",
            Self::Repetition => "repetition",
            Self::Distillation => "distillation",
            Self::Consolidation => "consolidation",
            Self::Manual => "manual",
        }
    }
}

/// One piece of evidence for one memory.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Witness {
    /// Its own identity, so a duplicate ingest can be idempotent.
    pub id: WitnessId,
    /// Which path produced it.
    pub kind: WitnessKind,
    /// The run that saw it. Counted for diversity, which is why it is not optional.
    pub session: SessionId,
    /// Where it was seen.
    pub scope: ScopeId,
    /// When.
    pub at: Timestamp,
    /// Where in that session's transcript, so `aeon why` can point at it.
    pub cursor: Option<u64>,
    /// What this one is worth. Defaults from the kind, and may be damped by a caller that
    /// knows better — several mentions in one session are one witness, not several.
    pub weight: f64,
    /// Which backend produced it, or which peer asked for it.
    ///
    /// Distilled output that came out of the rules must not be indistinguishable from output
    /// that came out of a model, and a write from a peer must name the peer.
    pub note: Option<String>,
}

impl Witness {
    /// Evidence of `kind`, seen in `session` at `at`, worth what the kind says it is worth.
    #[must_use]
    pub fn new(
        id: WitnessId,
        kind: WitnessKind,
        session: SessionId,
        scope: ScopeId,
        at: Timestamp,
    ) -> Self {
        Self {
            id,
            kind,
            session,
            scope,
            at,
            cursor: None,
            weight: kind.weight(),
            note: None,
        }
    }

    /// Where in the transcript this was seen.
    #[must_use]
    pub fn at_cursor(mut self, cursor: u64) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// Who or what produced it.
    #[must_use]
    pub fn noted(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// Worth less than its kind suggests — several mentions in one session, say.
    #[must_use]
    pub fn damped(mut self, factor: f64) -> Self {
        self.weight *= factor.clamp(0.0, 1.0);
        self
    }

    /// How much this still counts for at `now`.
    ///
    /// Evidence ages, but it does not expire: something witnessed two years ago was still
    /// witnessed. The floor keeps old evidence meaningful, and the curve keeps recent evidence
    /// worth more.
    #[must_use]
    pub fn value(&self, now: Timestamp) -> f64 {
        const FLOOR: f64 = 0.25;
        let days = ((now - self.at).max(0)) as f64 / 86_400.0;
        let recency = FLOOR + (1.0 - FLOOR) * (-days / 365.0).exp();
        self.weight * recency
    }
}

/// What a parse of an unknown witness kind says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownKind(pub String);

impl fmt::Display for UnknownKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}' is not a witness aeon knows", self.0)
    }
}

impl std::error::Error for UnknownKind {}

impl FromStr for WitnessKind {
    type Err = UnknownKind;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "imperative" => Ok(Self::Imperative),
            "correction" => Ok(Self::Correction),
            "cost" => Ok(Self::Cost),
            "repetition" => Ok(Self::Repetition),
            "distillation" => Ok(Self::Distillation),
            "consolidation" => Ok(Self::Consolidation),
            "manual" => Ok(Self::Manual),
            other => Err(UnknownKind(other.to_owned())),
        }
    }
}

impl fmt::Display for WitnessKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = 1_756_000_000;
    const YEAR: Timestamp = 365 * 86_400;

    fn witness(kind: WitnessKind, at: Timestamp) -> Witness {
        Witness::new(
            WitnessId::new("w"),
            kind,
            SessionId::new("s"),
            ScopeId::global(),
            at,
        )
    }

    #[test]
    fn distillation_alone_does_not_reach_the_promotion_floor() {
        // The main defence against "the model said it once and aeon believes it forever":
        // a thing that merely scrolled out of the window is a candidate, not a fact.
        assert!(!WitnessKind::Distillation.crosses_alone(0.5));
        assert!(!WitnessKind::Consolidation.crosses_alone(0.5));
        assert!(!WitnessKind::Repetition.crosses_alone(0.5));
    }

    #[test]
    fn what_was_asked_for_and_what_was_corrected_cross_alone() {
        assert!(WitnessKind::Imperative.crosses_alone(0.5));
        assert!(WitnessKind::Correction.crosses_alone(0.5));
        assert!(WitnessKind::Cost.crosses_alone(0.5));
    }

    #[test]
    fn a_peers_write_does_not_cross_alone() {
        // A socket peer proposes. The ladder still decides.
        assert!(!WitnessKind::Manual.crosses_alone(0.5));
    }

    #[test]
    fn only_an_imperative_pins() {
        assert!(WitnessKind::Imperative.pins());
        assert!(!WitnessKind::Correction.pins());
    }

    #[test]
    fn evidence_ages_but_never_expires() {
        let fresh = witness(WitnessKind::Imperative, NOW).value(NOW);
        let old = witness(WitnessKind::Imperative, NOW - 5 * YEAR).value(NOW);
        assert!(old < fresh);
        assert!(
            old >= 0.25,
            "something witnessed years ago was still witnessed"
        );
    }

    #[test]
    fn damping_makes_one_session_worth_less_than_many() {
        let full = witness(WitnessKind::Repetition, NOW);
        let damped = witness(WitnessKind::Repetition, NOW).damped(0.5);
        assert!(damped.value(NOW) < full.value(NOW));
    }

    #[test]
    fn every_kind_round_trips_through_its_column_spelling() {
        for kind in [
            WitnessKind::Imperative,
            WitnessKind::Correction,
            WitnessKind::Cost,
            WitnessKind::Repetition,
            WitnessKind::Distillation,
            WitnessKind::Consolidation,
            WitnessKind::Manual,
        ] {
            assert_eq!(kind.as_str().parse(), Ok(kind));
        }
    }
}
