//! The one place a row is removed.
//!
//! Everything else in this crate closes an interval, sets `archived_at`, or draws an edge. This
//! function deletes, and it exists for exactly one sentence: "delete the API key I pasted"
//! must be answerable with yes.
//!
//! `gate-no-delete` greps for `DELETE FROM` outside this file and the ledger's retention. Adding
//! a third is not a style violation; it is the commitment failing.
//!
//! **The closure is the whole difficulty.** A memory is not one row. It has evidence, asserted
//! edges, derived edges, an entity index, a full-text row, an embedding, and a trail through the
//! ledger — and a purge that removes the memory while leaving any of those has not answered the
//! sentence above. Worse, some of them can reconstruct what was purged: a derived edge still
//! points at the id, and the entity index still says what it was about.

use crate::{Store, StoreError};
use aeon_model::MemoryId;
use rusqlite::params;

/// What a purge would remove, counted before anything goes.
///
/// Shown first because a purge cannot be undone, and "this will also remove four derived
/// relationships and nine ledger rows" is something a person deserves to see while they can
/// still say no.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Closure {
    /// Pieces of evidence.
    pub witnesses: usize,
    /// Asserted links, in either direction.
    pub links: usize,
    /// Derived relationships, in either direction.
    pub relations: usize,
    /// Entity index rows.
    pub entities: usize,
    /// Ledger rows naming this memory.
    pub ledger: usize,
    /// Whether it carries an embedding.
    pub embedded: bool,
}

impl Closure {
    /// How many rows in total, beside the memory itself.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.witnesses + self.links + self.relations + self.entities + self.ledger
    }
}

/// What removing `id` would take with it.
pub fn closure_of(store: &Store, id: &MemoryId) -> Result<Closure, StoreError> {
    let one = |sql: &str| -> Result<usize, StoreError> {
        let n: i64 = store
            .db()
            .query_row(sql, params![id.as_str()], |r| r.get(0))?;
        Ok(n.max(0) as usize)
    };
    Ok(Closure {
        witnesses: one("SELECT count(*) FROM witness WHERE memory = ?1")?,
        links: one("SELECT count(*) FROM link WHERE src = ?1 OR dst = ?1")?,
        relations: one(
            "SELECT count(*) FROM relation_view WHERE from_memory = ?1 OR to_memory = ?1",
        )?,
        entities: one("SELECT count(*) FROM entity WHERE memory = ?1")?,
        ledger: one(
            "SELECT (SELECT count(*) FROM recall_candidate WHERE memory_id = ?1) \
                  + (SELECT count(*) FROM injection_memory WHERE memory_id = ?1) \
                  + (SELECT count(*) FROM action_memory WHERE memory_id = ?1)",
        )?,
        embedded: one("SELECT count(*) FROM memory WHERE id = ?1 AND embedding IS NOT NULL")? > 0,
    })
}

/// Remove a memory and everything that points at it, permanently.
///
/// Answers how many memories went. The caller is responsible for having asked a person first;
/// there is no confirmation in here, because a library that prompts cannot be scripted and a
/// library that prompts *sometimes* is worse.
///
/// Order matters: everything referring to the memory goes before the memory, because the
/// foreign keys point that way and a partial failure that left evidence for nothing would be
/// worse than not starting. The embedding needs no statement of its own — it is a column, and
/// it goes with the row.
pub fn purge(store: &mut Store, id: &MemoryId) -> Result<usize, StoreError> {
    let tx = store.db_mut().transaction()?;

    // Asserted edges, in both directions.
    tx.execute(
        "DELETE FROM link WHERE src = ?1 OR dst = ?1",
        params![id.as_str()],
    )?;
    // Derived edges. No foreign key holds these, so nothing would have complained — they would
    // simply have stayed, pointing at an id that no longer exists, and a traversal would still
    // have reached the hole where the secret was.
    tx.execute(
        "DELETE FROM relation_view WHERE from_memory = ?1 OR to_memory = ?1",
        params![id.as_str()],
    )?;
    // The entity index. This one has a foreign key, so leaving it out did not leave a dangling
    // row — it made the whole purge fail on any memory that had ever been indexed.
    tx.execute("DELETE FROM entity WHERE memory = ?1", params![id.as_str()])?;
    // The ledger's trail. These carry no content, but they name the id, and a purge that leaves
    // a record of what was retrieved has not removed what somebody asked to have removed.
    tx.execute(
        "DELETE FROM recall_candidate WHERE memory_id = ?1",
        params![id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM injection_memory WHERE memory_id = ?1",
        params![id.as_str()],
    )?;
    tx.execute(
        "DELETE FROM action_memory WHERE memory_id = ?1",
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
