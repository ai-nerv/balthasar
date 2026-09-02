//! Searching what was said.
//!
//! The other half of recall. `memory` holds only what crossed the ladder; this reads the turns
//! themselves, so a claim stated once and never written down is findable by its own words.
//!
//! Kept apart from `transcript.rs` because it is a different question about the same file —
//! that one is about what a run said and in what order, this one is about finding it again.

use crate::{StoreError, Transcript};
use memo_model::{SessionId, Timestamp};
use rusqlite::params;

/// Something that was said, found by searching what was said.
///
/// Deliberately not a [`Memory`](memo_model::Memory) and deliberately not convertible into one.
/// A span has no witnesses, so it has no derived confidence, so nothing can assert it — it is
/// offered as evidence that a thing was said, which is a different claim from the thing being
/// true. Everything that reads one is required to keep that distinction.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    /// Which run said it.
    pub session: SessionId,
    /// Where in that run, so it can be quoted and re-read.
    pub cursor: u64,
    /// What was said, verbatim.
    pub text: String,
    /// When.
    pub at: Timestamp,
    /// Who said it.
    pub role: String,
    /// The full-text rank. Negative, and smaller is better.
    pub rank: f64,
}

impl Transcript {
    /// The turns whose words match, best first.
    ///
    /// The half of recall that no memory can answer: a claim stated once, never repeated, never
    /// marked and never extracted is in here and nowhere else. Bounded like every other read on
    /// this file, which has no upper size.
    ///
    /// Ranked by `bm25` and then by recency, because two spans that match equally well are not
    /// equally useful — the later one is what somebody believes now.
    pub fn spans_matching(&self, terms: &str, limit: usize) -> Result<Vec<Span>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT turn_fts.session, turn_fts.cursor, turn_fts.text, turn.at, turn.role, \
                    bm25(turn_fts) AS rank \
             FROM turn_fts \
             JOIN turn ON turn.session = turn_fts.session AND turn.cursor = turn_fts.cursor \
             WHERE turn_fts MATCH ?1 \
             ORDER BY rank, turn.at DESC LIMIT ?2",
        )?;
        let found = statement
            .query_map(
                params![terms, limit as i64],
                |r| -> rusqlite::Result<Span> {
                    Ok(Span {
                        session: SessionId::new(r.get::<_, String>(0)?),
                        cursor: r.get::<_, i64>(1)?.max(0) as u64,
                        text: r.get(2)?,
                        at: r.get(3)?,
                        role: r.get(4)?,
                        rank: r.get(5)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }

    /// Rebuild the search index over every turn held.
    ///
    /// For a scrollback written before the index existed, and for `memo reindex`. Cheap to run
    /// again: the table is dropped and refilled rather than diffed.
    pub fn reindex(&self) -> Result<usize, StoreError> {
        self.db().execute("DELETE FROM turn_fts", [])?;
        let n = self.db().execute(
            "INSERT INTO turn_fts (text, session, cursor) \
             SELECT text, session, cursor FROM turn WHERE trim(text) != ''",
            [],
        )?;
        Ok(n)
    }
}
