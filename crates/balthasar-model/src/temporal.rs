//! The four clocks.
//!
//! Three is the usual bug. A store that records only when it was told, and treats that as when
//! the thing happened and as when the claim started being true, cannot answer "what did I
//! believe in March" and will date a backfill of six months of journals to today.

/// Unix seconds. Signed, because a `happened_at` read out of somebody else's file can be wrong
/// and an unsigned type turns that into a very large number rather than an obvious one.
pub type Timestamp = i64;

/// When a memory was learned, when it happened, and for how long it is claimed to be true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Temporal {
    /// When balthasar was told.
    pub observed_at: Timestamp,
    /// When the thing occurred, if that differs from being told about it.
    ///
    /// `None` means "the same as `observed_at`" rather than "unknown": a fact stated in
    /// conversation happened when it was stated, and forcing every caller to say so twice
    /// would only produce two fields that disagree.
    pub happened_at: Option<Timestamp>,
    /// When the claim started being true.
    pub valid_from: Timestamp,
    /// When it stopped. `None` is the whole point: it means *still true*, and it is what the
    /// partial unique index keys on so the database itself forbids two live answers to one slot.
    pub valid_to: Option<Timestamp>,
}

impl Temporal {
    /// A claim learned at `now` and true from then until something says otherwise.
    #[must_use]
    pub fn observed(now: Timestamp) -> Self {
        Self {
            observed_at: now,
            happened_at: None,
            valid_from: now,
            valid_to: None,
        }
    }

    /// A claim learned at `now` about something that happened at `then`.
    ///
    /// `valid_from` follows the happening, not the telling. Backfilling six months of journals
    /// must not claim every one of them started being true this afternoon.
    #[must_use]
    pub fn recalled(now: Timestamp, then: Timestamp) -> Self {
        Self {
            observed_at: now,
            happened_at: Some(then),
            valid_from: then,
            valid_to: None,
        }
    }

    /// When this happened, falling back to when we were told.
    #[must_use]
    pub fn when(&self) -> Timestamp {
        self.happened_at.unwrap_or(self.observed_at)
    }

    /// Whether the claim is still standing.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.valid_to.is_none()
    }

    /// Whether the claim was true at `at`.
    ///
    /// This is what makes "what was the deploy target in March" answerable without a graph
    /// database: the interval is on the row, and the question is a comparison.
    #[must_use]
    pub fn was_true_at(&self, at: Timestamp) -> bool {
        at >= self.valid_from && self.valid_to.is_none_or(|end| at < end)
    }

    /// Close the interval at `at`, because something superseded it.
    ///
    /// Idempotent, and it never moves an existing end: a fact contradicted twice was
    /// contradicted once, and the second correction should not rewrite when it stopped being
    /// true.
    pub fn close(&mut self, at: Timestamp) {
        if self.valid_to.is_none() {
            self.valid_to = Some(at.max(self.valid_from));
        }
    }

    /// Whole days between `self.observed_at` (or the happening) and `now`, never negative.
    #[must_use]
    pub fn age_days(&self, now: Timestamp) -> f64 {
        let seconds = (now - self.when()).max(0);
        seconds as f64 / 86_400.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARCH: Timestamp = 1_710_000_000;
    const AUGUST: Timestamp = 1_756_000_000;

    #[test]
    fn a_fresh_claim_is_live() {
        let t = Temporal::observed(AUGUST);
        assert!(t.is_live());
        assert!(t.was_true_at(AUGUST));
    }

    #[test]
    fn a_backfilled_claim_dates_from_the_happening() {
        // Ingesting six months of journals must not claim all of it started today.
        let t = Temporal::recalled(AUGUST, MARCH);
        assert_eq!(t.valid_from, MARCH);
        assert_eq!(t.observed_at, AUGUST);
        assert!(t.was_true_at(MARCH + 1));
    }

    #[test]
    fn a_closed_interval_answers_both_questions() {
        let mut t = Temporal::observed(MARCH);
        t.close(AUGUST);
        assert!(t.was_true_at(MARCH + 10), "it was true in March");
        assert!(!t.was_true_at(AUGUST + 10), "it is not true now");
        assert!(!t.is_live());
    }

    #[test]
    fn closing_twice_does_not_move_the_end() {
        // A fact contradicted twice was contradicted once. The second correction must not
        // rewrite when the first one stopped being true.
        let mut t = Temporal::observed(MARCH);
        t.close(AUGUST);
        t.close(AUGUST + 999);
        assert_eq!(t.valid_to, Some(AUGUST));
    }

    #[test]
    fn an_end_never_precedes_the_start() {
        let mut t = Temporal::observed(AUGUST);
        t.close(MARCH);
        assert_eq!(t.valid_to, Some(AUGUST));
    }

    #[test]
    fn age_is_never_negative() {
        let t = Temporal::observed(AUGUST);
        assert_eq!(t.age_days(MARCH), 0.0);
    }
}
