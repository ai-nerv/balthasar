//! Closing memories on the repository's word.

use crate::{Store, StoreError};
use balthasar_model::{MemoryId, Timestamp};
use rusqlite::params;

impl Store {
    /// Close a memory with nothing standing in for it: still kept, no longer current.
    pub fn close(&mut self, id: &MemoryId, now: Timestamp) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE memory SET valid_to = ?2 WHERE id = ?1 AND valid_to IS NULL",
            params![id.as_str(), now],
        )?;
        self.rescore(id, now)?;
        Ok(())
    }

    /// Open memories a commit witness vouches for whose note holds `fragment`.
    pub fn committed_with(&self, fragment: &str) -> Result<Vec<MemoryId>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT DISTINCT w.memory FROM witness w JOIN live_memory m ON m.id = w.memory \
             WHERE w.kind = 'commit' AND instr(w.note, ?1) > 0 ORDER BY w.memory",
        )?;
        let found = statement
            .query_map([fragment], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found.into_iter().map(MemoryId::new).collect())
    }

    /// Whether any commit from `source` has been read into this store yet.
    pub fn has_read_any(&self, source: &str) -> Result<bool, StoreError> {
        Ok(self.db().query_row(
            "SELECT EXISTS (SELECT 1 FROM stamp WHERE source = ?1)",
            [source],
            |r| r.get(0),
        )?)
    }
}
