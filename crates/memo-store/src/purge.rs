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
use memo_model::MemoryId;
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
    /// Memories distilled or consolidated out of this one, which go with it.
    ///
    /// Counted separately from `links` because these are not rows that point at a memory — they
    /// are memories, and removing this one removes them too. A confirmation that said "four
    /// links" when it meant "four beliefs" would be the wrong prompt.
    pub derived: usize,
}

impl Closure {
    /// How many rows in total, beside the memory itself.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.witnesses + self.links + self.relations + self.entities + self.ledger + self.derived
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
        derived: descendants(store, id)?.len(),
    })
}

impl Store {
    /// Every memory a run owns.
    ///
    /// Owning is not having seen: a memory another run wrote and this one merely witnessed
    /// belongs to the other, and forgetting this run must leave it standing.
    pub fn owned_by(&self, session: &memo_model::SessionId) -> Result<Vec<MemoryId>, StoreError> {
        let mut statement = self
            .db()
            .prepare("SELECT id FROM memory WHERE session = ?1")?;
        let found = statement
            .query_map(params![session.as_str()], |r| {
                Ok(MemoryId::new(r.get::<_, String>(0)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }
}

/// Remove one run's own scratch, permanently.
///
/// The third place a run lives. Its memories are in the project store, its turns are in the
/// scrollback, and everything it thought but never promoted is in a directory of its own —
/// which is where a pasted key would still be sitting after the other two were cleared.
pub fn purge_scratch(
    pad: &mut crate::Scratchpad,
    session: &memo_model::SessionId,
) -> Result<bool, StoreError> {
    let path = pad.path_of(session);
    let Some(dir) = path.parent() else {
        return Ok(false);
    };
    if !dir.is_dir() {
        return Ok(false);
    }
    // Before removing the file, not after: an open connection to a file that has stopped
    // existing is a store that answers questions out of a deleted inode.
    pad.close(session);
    std::fs::remove_dir_all(dir).map_err(|why| StoreError::Foreign(why.to_string()))?;
    Ok(true)
}

/// Remove one run's turns from the scrollback, permanently.
///
/// The other half of forgetting a run. The scrollback is a separate file, so `purge_session`
/// cannot reach it and a caller answering "forget that session" has to do both — the memories
/// it owned and the conversation it held.
pub fn purge_run(
    scrollback: &crate::Transcript,
    session: &memo_model::SessionId,
) -> Result<usize, StoreError> {
    // The search index holds a second copy of every word. Deleting the turn and leaving the
    // index is the failure this whole file exists to prevent, and the byte-level purge test
    // catches it the moment the index is added — which is how this line came to be written.
    scrollback.db().execute(
        "DELETE FROM turn_fts WHERE session = ?1",
        params![session.as_str()],
    )?;
    let gone = scrollback.db().execute(
        "DELETE FROM turn WHERE session = ?1",
        params![session.as_str()],
    )?;
    scrollback.db().execute(
        "DELETE FROM run WHERE session = ?1",
        params![session.as_str()],
    )?;
    Ok(gone)
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
///
/// **Derivations go too.** A memory distilled or consolidated out of this one is a copy of it
/// under another name, and leaving it standing means the claim survives the delete — worse, the
/// next consolidation pass rewrites it back out of the survivor while the record says it was
/// erased. Descendants are removed first, so no edge is ever left pointing at nothing.
pub fn purge(store: &mut Store, id: &MemoryId) -> Result<usize, StoreError> {
    let mut gone = 0;
    for derived in descendants(store, id)? {
        gone += purge_one(store, &derived)?;
    }
    Ok(gone + purge_one(store, id)?)
}

/// Everything made out of this memory, deepest first.
///
/// Breadth-first with a seen set, so a cycle in the derivation graph terminates and a memory
/// reached by two paths is removed once. Reversed on the way out, so a descendant is always
/// purged before whatever it was derived from.
fn descendants(store: &Store, root: &MemoryId) -> Result<Vec<MemoryId>, StoreError> {
    let mut seen = vec![root.clone()];
    let mut order = Vec::new();
    let mut queue = vec![root.clone()];

    while let Some(here) = queue.pop() {
        let mut statement = store
            .db()
            .prepare("SELECT src FROM link WHERE dst = ?1 AND rel = 'derived_from'")?;
        let made: Vec<MemoryId> = statement
            .query_map(params![here.as_str()], |r| {
                Ok(MemoryId::new(r.get::<_, String>(0)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for id in made {
            if !seen.contains(&id) {
                seen.push(id.clone());
                order.push(id.clone());
                queue.push(id);
            }
        }
    }
    order.reverse();
    Ok(order)
}

/// One memory, and everything that points at it.
fn purge_one(store: &mut Store, id: &MemoryId) -> Result<usize, StoreError> {
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

/// Everything one run left behind.
///
/// §10.8's trajectory scope. A person who says "forget that session" means the whole of it —
/// what it observed, what it distilled, and the evidence it filed — not one memory they can
/// name. Returns how many memories went.
///
/// Memories a run merely *witnessed* are not removed. A fact three runs agree on does not
/// belong to any of them, and taking it with one would be a different and much worse operation
/// than the one somebody asked for; its witness from this run goes, and the fact stays with the
/// evidence that remains.
pub fn purge_session(
    store: &mut Store,
    session: &memo_model::SessionId,
) -> Result<usize, StoreError> {
    let owned = store.owned_by(session)?;

    let mut gone = 0;
    for id in &owned {
        gone += purge(store, id)?;
    }

    // The run's own evidence for things it did not own, and its bookkeeping. Neither can
    // reconstruct a memory, and leaving them would keep the run's shape after the run is gone.
    let tx = store.db_mut().transaction()?;
    tx.execute(
        "DELETE FROM witness WHERE session = ?1",
        params![session.as_str()],
    )?;
    tx.execute(
        "DELETE FROM episode_segment WHERE session = ?1",
        params![session.as_str()],
    )?;
    tx.execute(
        "DELETE FROM session WHERE id = ?1",
        params![session.as_str()],
    )?;
    tx.commit()?;
    Ok(gone)
}

/// Everything that came from one source.
///
/// §10.8's environment scope, and the one an incident actually needs: a page turned out to be
/// hostile, and the question is what it touched. Removes every memory whose evidence comes only
/// from that domain.
///
/// A memory with evidence from elsewhere as well is kept, with the tainted witness removed —
/// because it stands on what remains, and deleting it would let one poisoned source take honest
/// memories with it. That is the denial-of-service version of this operation and it is worth
/// refusing.
pub fn purge_domain(store: &mut Store, domain: &memo_model::Domain) -> Result<usize, StoreError> {
    let touched: Vec<MemoryId> = {
        let mut statement = store
            .db()
            .prepare("SELECT DISTINCT memory FROM witness WHERE domain = ?1")?;
        statement
            .query_map(params![domain.as_str()], |r| {
                Ok(MemoryId::new(r.get::<_, String>(0)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };

    let mut gone = 0;
    for id in &touched {
        let total: i64 = store.db().query_row(
            "SELECT count(*) FROM witness WHERE memory = ?1",
            params![id.as_str()],
            |r| r.get(0),
        )?;
        let tainted: i64 = store.db().query_row(
            "SELECT count(*) FROM witness WHERE memory = ?1 AND domain = ?2",
            params![id.as_str(), domain.as_str()],
            |r| r.get(0),
        )?;

        if tainted >= total {
            gone += purge(store, id)?;
        } else {
            store.db().execute(
                "DELETE FROM witness WHERE memory = ?1 AND domain = ?2",
                params![id.as_str(), domain.as_str()],
            )?;
            // Confidence is derived, so it has to be recomputed now that the evidence changed.
            // Leaving it would state a number the remaining witnesses do not support.
            store.rescore(id, memo_model::Timestamp::default())?;
        }
    }
    Ok(gone)
}
