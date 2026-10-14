//! A store kept inside a project moves with it. Its rows name the scope by path, so a moved
//! checkout's store would otherwise hold the old path's answers beside the new one's.
//!
//! The file is the project's wherever it now lives: every scope it holds other than `global` is
//! rewritten to the current one, in one transaction, the newer answer to a slot keeping its place.

use crate::{Store, StoreError, Transcript};
use balthasar_model::{ScopeId, Timestamp};
use rusqlite::{Connection, Transaction, TransactionBehavior, params};

/// Every table of the memory store that names a scope.
pub const SCOPED: &[&str] = &["memory", "witness", "session", "entity", "recall_run"];

/// Every table of the scrollback that names a scope.
pub const SCOPED_SCROLLBACK: &[&str] = &["run", "layout_effect"];

/// The scopes a store holds that are neither `global` nor `current`.
fn elsewhere(db: &Connection, tables: &[&str], current: &str) -> Result<Vec<String>, StoreError> {
    let union = tables
        .iter()
        .map(|table| format!("SELECT scope FROM {table}"))
        .collect::<Vec<_>>()
        .join(" UNION ");
    let mut statement = db.prepare(&format!(
        "SELECT scope FROM ({union}) WHERE scope NOT IN ('global', ?1)"
    ))?;
    let found = statement
        .query_map(params![current], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(found)
}

/// Whether any table indexed by scope holds one other than `global` and `current`: index range
/// reads either side of `current`, so an unmoved store pays nothing more and takes no lock.
fn moved(db: &Connection, current: &str) -> Result<bool, StoreError> {
    for table in ["memory", "session", "entity", "recall_run"] {
        let found: bool = db.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM {table} WHERE scope < ?1) \
                     OR EXISTS(SELECT 1 FROM {table} WHERE scope > ?1 AND scope <> 'global')"
            ),
            params![current],
            |r| r.get(0),
        )?;
        if found {
            return Ok(true);
        }
    }
    Ok(false)
}

impl Store {
    /// Rewrite every scope this store holds to `current`, which is where it now lives. A slot
    /// answered under both is left to the answer given under `current`; the other is closed.
    /// Answers how many scopes were brought over.
    pub fn settle_scope(&mut self, current: &ScopeId, now: Timestamp) -> Result<usize, StoreError> {
        if current.is_global() || !moved(self.db(), current.as_str())? {
            return Ok(0);
        }
        let tx = Transaction::new_unchecked(self.db(), TransactionBehavior::Immediate)?;
        let old = elsewhere(&tx, SCOPED, current.as_str())?;
        for was in &old {
            tx.execute(
                "UPDATE memory SET valid_to = ?3 WHERE scope = ?1 AND valid_to IS NULL \
                   AND tier = 'fact' AND archived_at IS NULL AND single_valued = 1 \
                   AND EXISTS (SELECT 1 FROM memory AS here WHERE here.scope = ?2 \
                     AND here.subject IS memory.subject AND here.predicate IS memory.predicate \
                     AND here.valid_to IS NULL AND here.tier = 'fact' \
                     AND here.archived_at IS NULL AND here.single_valued = 1)",
                params![was, current.as_str(), now],
            )?;
            for table in SCOPED {
                tx.execute(
                    &format!("UPDATE OR IGNORE {table} SET scope = ?2 WHERE scope = ?1"),
                    params![was, current.as_str()],
                )?;
            }
            // The entity index is derived; what it could not move is already held under `current`.
            tx.execute("DELETE FROM entity WHERE scope = ?1", params![was])?;
        }
        tx.commit()?;
        Ok(old.len())
    }
}

impl Transcript {
    /// The same for the scrollback beside the store, so effects still owed are found again.
    pub fn settle_scope(&self, current: &ScopeId) -> Result<usize, StoreError> {
        if current.is_global()
            || elsewhere(self.db(), SCOPED_SCROLLBACK, current.as_str())?.is_empty()
        {
            return Ok(0);
        }
        let tx = Transaction::new_unchecked(self.db(), TransactionBehavior::Immediate)?;
        let old = elsewhere(&tx, SCOPED_SCROLLBACK, current.as_str())?;
        for was in &old {
            for table in SCOPED_SCROLLBACK {
                tx.execute(
                    &format!("UPDATE OR IGNORE {table} SET scope = ?2 WHERE scope = ?1"),
                    params![was, current.as_str()],
                )?;
            }
        }
        tx.commit()?;
        Ok(old.len())
    }
}
