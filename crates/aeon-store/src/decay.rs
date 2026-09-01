//! Forgetting, as a pass over the store.
//!
//! The arithmetic lives in the model; this is what applies it, and what can show its work
//! first. Forgetting is the most alarming thing aeon does and it should never be a surprise —
//! so the preview and the pass are the same code with one flag, rather than two functions that
//! can drift into disagreeing about what would happen.
//!
//! Nothing here deletes. A memory that fades past the floor moves to the archive with its
//! evidence, its edges and its embedding intact, and comes back when the live results are weak.

use crate::{Store, StoreError, row};
use aeon_model::{MemoryId, Timestamp, floor};
use rusqlite::params;

/// What a pass did, or would do.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Faded {
    /// Memories whose strength fell, and by how much.
    pub weakened: Vec<Weakened>,
    /// Memories that fell past the floor and left the live set.
    pub swept: Vec<Weakened>,
    /// Memories that did not fade because somebody pinned them.
    pub pinned: usize,
    /// Whether this was a rehearsal.
    pub preview: bool,
}

/// One memory, and what a pass does to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Weakened {
    /// Which memory.
    pub id: MemoryId,
    /// What it says, so a report reads without a second query.
    pub text: String,
    /// Strength before.
    pub was: f64,
    /// Strength after.
    pub now: f64,
    /// How long since it was last needed.
    pub idle_days: f64,
}

impl Store {
    /// Show what a decay pass would do, without doing it.
    pub fn decay_preview(&mut self, now: Timestamp) -> Result<Faded, StoreError> {
        self.fade(now, true, true)
    }

    /// Apply the fade, and sweep what is spent into the archive.
    ///
    /// The two together, for a caller that only wants the whole thing. A consolidation cycle
    /// wants them apart — see [`Store::weaken`] and [`Store::sweep`].
    pub fn decay(&mut self, now: Timestamp) -> Result<Faded, StoreError> {
        self.fade(now, false, true)
    }

    /// Apply the fade, and leave what is spent where it is.
    ///
    /// The first step of a consolidation cycle. Sweeping here would archive the very scratch
    /// the cycle is about to look at for corroboration, and the promotion it was going to make
    /// would silently never happen — which is exactly what it did until this was split out.
    pub fn weaken(&mut self, now: Timestamp) -> Result<Faded, StoreError> {
        self.fade(now, false, false)
    }

    /// Move what is spent into the archive.
    ///
    /// The last step. By now the ladder has had its chance at everything, so what is left is
    /// genuinely finished with.
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
            // Tier-aware. A fact barely fades; an afternoon's episode does, and a session's
            // own scratch fades fastest of all.
            let now_value = memory.strength.at_tier(memory.tier, now);
            let spent = sweeping && now_value < floor::SPENT;
            // Unchanged is not worth reporting. A pass run twice in a minute should say it
            // did nothing rather than list every memory in the store as "weakened by 0.00".
            //
            // Except when it is already spent. Consolidation weakens early and sweeps at the
            // end of the same pass with the same clock, so by the time sweeping looks, every
            // strength has already moved and nothing appears to have changed — which meant
            // the sweep archived nothing at all, ever.
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
