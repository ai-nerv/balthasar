//! The one place a row is removed.
//!
//! Everything else in this crate closes an interval, sets `archived_at`, or draws an edge. This
//! function deletes, and it exists for exactly one sentence: "delete the API key I pasted"
//! must be answerable with yes.
//!
//! `gate-no-delete` greps for `DELETE FROM` outside this file. Adding a second one is not a
//! style violation; it is the commitment failing.

use crate::{Store, StoreError};
use aeon_model::MemoryId;
use rusqlite::params;

/// Remove a memory, its evidence and its edges, permanently.
///
/// Answers how many memories went. The caller is responsible for having asked a person first;
/// there is no confirmation in here, because a library that prompts cannot be scripted and a
/// library that prompts *sometimes* is worse.
pub fn purge(store: &mut Store, id: &MemoryId) -> Result<usize, StoreError> {
    let tx = store.db_mut().transaction()?;
    // Edges first, then evidence, then the row: the foreign keys point that way, and a
    // partial failure that left evidence for nothing would be worse than not starting.
    tx.execute(
        "DELETE FROM link WHERE src = ?1 OR dst = ?1",
        params![id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM witness WHERE memory = ?1",
        params![id.as_str()],
    )?;
    tx.execute("DELETE FROM memory_fts WHERE id = ?1", params![id.as_str()])?;
    let gone = tx.execute("DELETE FROM memory WHERE id = ?1", params![id.as_str()])?;
    tx.commit()?;
    Ok(gone)
}
