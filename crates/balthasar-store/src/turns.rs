//! Writing a turn: inserted once, revised with every earlier revision kept, and what was noticed
//! at a cursor kept beside it rather than over it.

use crate::transcript::{COLUMNS, read};
use crate::{StoreError, Transcript, Turn};
use balthasar_model::{SessionId, Timestamp};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

/// What an insert found at its cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inserted {
    /// Nothing was there, and now the turn is.
    Fresh,
    /// The same turn was already there: a retry, and nothing changed.
    Same,
    /// A different turn holds the cursor, and nothing changed.
    Different,
}

/// Something that happened at a cursor and is not a turn: a permission, a retry, a compaction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Noticed {
    pub cursor: u64,
    /// Which of several at one cursor, in the order they arrived.
    pub seq: u64,
    pub at: Timestamp,
    pub kind: String,
    pub text: String,
}

/// Whether two turns say the same thing. When they settled, and what has become of them in the
/// window since, are not part of what was said.
#[must_use]
pub fn same(a: &Turn, b: &Turn) -> bool {
    (
        &a.role, &a.kind, &a.text, &a.tool, &a.raw, &a.entry, &a.tokens, &a.ok, &a.ms, &a.args,
    ) == (
        &b.role, &b.kind, &b.text, &b.tool, &b.raw, &b.entry, &b.tokens, &b.ok, &b.ms, &b.args,
    ) && (&a.group, &a.stub, &a.handle, a.keep, a.error, a.pinned)
        == (&b.group, &b.stub, &b.handle, b.keep, b.error, b.pinned)
}

fn clamp(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

impl Transcript {
    /// Write a turn at a cursor nothing holds yet. A cursor that holds the same turn is a retry
    /// and changes nothing; one that holds a different turn is left alone and said so. The row
    /// and its search entry land in one transaction, durable on return.
    pub fn insert(&self, session: &SessionId, turn: &Turn) -> Result<Inserted, StoreError> {
        let tx = Transaction::new_unchecked(self.db(), TransactionBehavior::Immediate)?;
        let held = held(&tx, session, turn.cursor)?;
        let outcome = match held {
            Some(existing) if same(&existing, turn) => Inserted::Same,
            Some(_) => Inserted::Different,
            None => {
                put(&tx, session, turn)?;
                Inserted::Fresh
            }
        };
        tx.commit()?;
        Ok(outcome)
    }

    /// Write one turn, or revise the one already at that cursor, keeping what it said before.
    ///
    /// The row, its earlier revision and its search entry land in one transaction. Writing the
    /// same turn again changes nothing.
    pub fn write(&mut self, session: &SessionId, turn: &Turn) -> Result<(), StoreError> {
        let tx = Transaction::new_unchecked(self.db(), TransactionBehavior::Immediate)?;
        match held(&tx, session, turn.cursor)? {
            None => put(&tx, session, turn)?,
            Some(existing) if same(&existing, turn) => {}
            Some(_) => revise(&tx, session, turn)?,
        }
        tx.commit()?;
        Ok(())
    }

    /// Every earlier revision of the turn at a cursor, oldest first.
    pub fn revisions_of(&self, session: &SessionId, cursor: u64) -> Result<Vec<Turn>, StoreError> {
        let mut statement = self.db().prepare(&format!(
            "SELECT {COLUMNS} FROM turn_revision WHERE session = ?1 AND cursor = ?2 \
             ORDER BY revisions"
        ))?;
        let found = statement
            .query_map(params![session.as_str(), clamp(cursor)], read)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }

    /// Keep what was noticed at a cursor, beside the turn there and never over it.
    pub fn notice(
        &self,
        session: &SessionId,
        cursor: u64,
        at: Timestamp,
        kind: &str,
        text: &str,
    ) -> Result<u64, StoreError> {
        let tx = Transaction::new_unchecked(self.db(), TransactionBehavior::Immediate)?;
        let seq: i64 = tx.query_row(
            "SELECT COALESCE(max(seq) + 1, 0) FROM trace WHERE session = ?1 AND cursor = ?2",
            params![session.as_str(), clamp(cursor)],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO trace (session, cursor, seq, at, kind, text) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![session.as_str(), clamp(cursor), seq, at, kind, text],
        )?;
        tx.commit()?;
        Ok(seq.max(0) as u64)
    }

    /// What was noticed between two cursors, inclusive, in order.
    pub fn noticed(
        &self,
        session: &SessionId,
        from: u64,
        to: u64,
    ) -> Result<Vec<Noticed>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT cursor, seq, at, kind, text FROM trace \
             WHERE session = ?1 AND cursor >= ?2 AND cursor <= ?3 ORDER BY cursor, seq",
        )?;
        let found = statement
            .query_map(params![session.as_str(), clamp(from), clamp(to)], |r| {
                Ok(Noticed {
                    cursor: r.get::<_, i64>(0)?.max(0) as u64,
                    seq: r.get::<_, i64>(1)?.max(0) as u64,
                    at: r.get(2)?,
                    kind: r.get(3)?,
                    text: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }
}

/// The turn at a cursor, inside a transaction.
fn held(
    tx: &Transaction<'_>,
    session: &SessionId,
    cursor: u64,
) -> Result<Option<Turn>, StoreError> {
    Ok(tx
        .query_row(
            &format!("SELECT {COLUMNS} FROM turn WHERE session = ?1 AND cursor = ?2"),
            params![session.as_str(), clamp(cursor)],
            read,
        )
        .optional()?)
}

/// A new row, and its search entry.
fn put(tx: &Transaction<'_>, session: &SessionId, turn: &Turn) -> Result<(), StoreError> {
    tx.execute(
        "INSERT INTO turn \
         (session, cursor, at, role, kind, text, tool, raw, entry, tokens, pinned, \
          ok, ms, args, revisions, grp, stub, handle, keep, error) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0, \
                 ?15, ?16, ?17, ?18, ?19)",
        params![
            session.as_str(),
            clamp(turn.cursor),
            turn.at,
            turn.role,
            turn.kind,
            turn.text,
            turn.tool,
            turn.raw,
            turn.entry,
            turn.tokens,
            i64::from(turn.pinned),
            turn.ok,
            turn.ms.map(clamp),
            turn.args,
            turn.group.map(clamp),
            turn.stub,
            turn.handle,
            i64::from(turn.keep),
            i64::from(turn.error),
        ],
    )?;
    index(tx, session, turn, false)
}

/// Keep the row as it stands, then overwrite it, and its search entry.
fn revise(tx: &Transaction<'_>, session: &SessionId, turn: &Turn) -> Result<(), StoreError> {
    tx.execute(
        &format!(
            "INSERT INTO turn_revision (session, {COLUMNS}) \
             SELECT session, {COLUMNS} FROM turn WHERE session = ?1 AND cursor = ?2"
        ),
        params![session.as_str(), clamp(turn.cursor)],
    )?;
    tx.execute(
        "UPDATE turn SET at = ?3, role = ?4, kind = ?5, text = ?6, tool = ?7, raw = ?8, \
           entry = ?9, tokens = ?10, pinned = ?11, ok = ?12, ms = ?13, args = ?14, \
           grp = ?15, stub = ?16, handle = ?17, keep = ?18, error = ?19, \
           revisions = revisions + 1 \
         WHERE session = ?1 AND cursor = ?2",
        params![
            session.as_str(),
            clamp(turn.cursor),
            turn.at,
            turn.role,
            turn.kind,
            turn.text,
            turn.tool,
            turn.raw,
            turn.entry,
            turn.tokens,
            i64::from(turn.pinned),
            turn.ok,
            turn.ms.map(clamp),
            turn.args,
            turn.group.map(clamp),
            turn.stub,
            turn.handle,
            i64::from(turn.keep),
            i64::from(turn.error),
        ],
    )?;
    index(tx, session, turn, true)
}

/// The search entry for a turn; one already there is removed first when `replacing`. A fresh
/// row has none, and the removal scans the index, so it is not paid for then.
fn index(
    tx: &Transaction<'_>,
    session: &SessionId,
    turn: &Turn,
    replacing: bool,
) -> Result<(), StoreError> {
    if replacing {
        tx.execute(
            "DELETE FROM turn_fts WHERE session = ?1 AND cursor = ?2",
            params![session.as_str(), clamp(turn.cursor)],
        )?;
    }
    if !turn.text.trim().is_empty() {
        tx.execute(
            "INSERT INTO turn_fts (text, session, cursor) VALUES (?1, ?2, ?3)",
            params![turn.text, session.as_str(), clamp(turn.cursor)],
        )?;
    }
    Ok(())
}
