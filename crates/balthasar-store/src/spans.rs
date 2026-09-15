//! Searching what was said. `memory` holds only what crossed the ladder; this reads the turns
//! themselves, so a claim stated once and never written down is findable by its own words.

use crate::{StoreError, Transcript};
use balthasar_model::{SessionId, Timestamp};
use rusqlite::params;

/// Something that was said, found by searching what was said. Not a
/// [`Memory`](balthasar_model::Memory): a span has no witnesses, so nothing can assert it.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub session: SessionId,
    pub cursor: u64,
    pub text: String,
    pub at: Timestamp,
    pub role: String,
    /// The full-text rank. Negative, and smaller is better.
    pub rank: f64,
}

impl Transcript {
    /// The turns whose words match, best first. Ranked by `bm25` and then by recency, because
    /// the later of two equally-matching spans is what somebody believes now.
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

    /// Rebuild the search index over every turn held. The table is dropped and refilled.
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
