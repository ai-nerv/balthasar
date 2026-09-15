//! Forgetting, as a pass over the store.
//!
//! Nothing here deletes; a memory that fades past the floor moves to the archive intact.

use crate::{Store, StoreError, row};
use balthasar_model::{MemoryId, Timestamp, floor};
use rusqlite::params;

/// What a pass did, or would do.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Faded {
    pub weakened: Vec<Weakened>,
    pub swept: Vec<Weakened>,
    pub pinned: usize,
    pub preview: bool,
}

/// One memory, and what a pass does to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Weakened {
    pub id: MemoryId,
    pub text: String,
    /// Strength before.
    pub was: f64,
    /// Strength after.
    pub now: f64,
    pub idle_days: f64,
}

impl Store {
    pub fn decay_preview(&mut self, now: Timestamp) -> Result<Faded, StoreError> {
        self.fade(now, true, true)
    }

    /// Apply the fade, and sweep what is spent into the archive.
    pub fn decay(&mut self, now: Timestamp) -> Result<Faded, StoreError> {
        self.fade(now, false, true)
    }

    /// Apply the fade, and leave what is spent where it is. Sweeping here would archive the very
    /// scratch the cycle is about to look at for corroboration.
    pub fn weaken(&mut self, now: Timestamp) -> Result<Faded, StoreError> {
        self.fade(now, false, false)
    }

    /// Move what is spent into the archive, the last step of a consolidation cycle.
    pub fn sweep(&mut self, now: Timestamp) -> Result<Faded, StoreError> {
        self.fade(now, false, true)
    }

    /// One pass. `preview` decides whether anything is written; `sweeping` decides whether
    /// what has fallen past the floor leaves the live set.
    fn fade(&mut self, now: Timestamp, preview: bool, sweeping: bool) -> Result<Faded, StoreError> {
        let mut report = Faded {
            preview,
            ..Faded::default()
        };

        let candidates = {
            let mut statement = self.db().prepare(&format!(
                "SELECT {} FROM memory WHERE archived_at IS NULL ORDER BY last_accessed",
                row::COLUMNS
            ))?;
            let found = statement
                .query_map([], |r| Ok(row::memory(r)))?
                .collect::<Result<Vec<_>, _>>()?;
            found.into_iter().collect::<Result<Vec<_>, _>>()?
        };

        for memory in candidates {
            if memory.strength.pinned {
                report.pinned += 1;
                continue;
            }
            let was = memory.strength.value;
            // Tier-aware: a fact barely fades, a session's own scratch fades fastest.
            let now_value = memory.strength.at_tier(memory.tier, now);
            let spent = sweeping && now_value < floor::SPENT;
            // Unchanged is not worth reporting, except when it is already spent: consolidation
            // weakens and sweeps in one pass, so by then no strength appears to have changed.
            if !spent && (was - now_value).abs() < f64::EPSILON {
                continue;
            }

            let entry = Weakened {
                id: memory.id.clone(),
                text: memory.text(),
                was,
                now: now_value,
                idle_days: ((now - memory.strength.last_accessed).max(0)) as f64 / 86_400.0,
            };

            if spent {
                report.swept.push(entry);
                if !preview {
                    self.db().execute(
                        "UPDATE memory SET strength = ?2, last_accessed = ?3, \
                         archived_at = ?3, tier = 'archive' WHERE id = ?1",
                        params![memory.id.as_str(), now_value, now],
                    )?;
                }
            } else {
                report.weakened.push(entry);
                if !preview {
                    self.db().execute(
                        "UPDATE memory SET strength = ?2, last_accessed = ?3 WHERE id = ?1",
                        params![memory.id.as_str(), now_value, now],
                    )?;
                }
            }
        }
        Ok(report)
    }
}

impl Faded {
    /// Whether the pass found anything to do.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.weakened.is_empty() && self.swept.is_empty()
    }
}
