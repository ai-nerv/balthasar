//! Prepared working state: derived from the transcript, kept beside it, and swapped in whole.
//!
//! A generation is built, made ready, and then activated in one transaction that retires whatever
//! was active before. A partial unique index carries the "one active" rule, so a process that dies
//! mid-activation reopens on exactly one of the two.

use crate::{StoreError, Transcript};
use balthasar_model::{SessionId, Timestamp};
use serde_json::Value;

/// Applied on open, like every other part of the scrollback: writing it again is a no-op, so a
/// file from before generations existed gains them without a migration step of its own.
pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS generation (
  id          TEXT PRIMARY KEY,
  session     TEXT NOT NULL,
  state       TEXT NOT NULL,
  parent      TEXT,
  covers_from INTEGER NOT NULL,
  covers_to   INTEGER NOT NULL,
  policy      TEXT NOT NULL,
  provenance  TEXT NOT NULL,
  content     TEXT NOT NULL,
  made        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS generation_session ON generation(session, made);
CREATE UNIQUE INDEX IF NOT EXISTS generation_active
  ON generation(session) WHERE state = 'active';

CREATE TABLE IF NOT EXISTS chunk (
  session  TEXT NOT NULL,
  cursor   INTEGER NOT NULL,
  revision TEXT NOT NULL,
  offset   INTEGER NOT NULL,
  length   INTEGER NOT NULL,
  PRIMARY KEY (session, cursor, revision, offset)
);
"#;

/// How far a generation has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ready {
    /// Written, not yet offered.
    Building,
    /// Complete and eligible to be activated.
    Ready,
    /// What a request is laid out from.
    Active,
    /// Superseded. Kept for inspection until retention drops it.
    Retired,
}

impl Ready {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Building => "building",
            Self::Ready => "ready",
            Self::Active => "active",
            Self::Retired => "retired",
        }
    }

    #[must_use]
    pub fn read(said: &str) -> Option<Self> {
        Some(match said {
            "building" => Self::Building,
            "ready" => Self::Ready,
            "active" => Self::Active,
            "retired" => Self::Retired,
            _ => return None,
        })
    }
}

/// One prepared representation of a span.
#[derive(Debug, Clone, PartialEq)]
pub struct Generation {
    pub id: String,
    pub session: SessionId,
    pub state: Ready,
    /// The generation this was prepared from, when it was prepared from one.
    pub parent: Option<String>,
    /// The rows it represents, inclusive.
    pub covers: (u64, u64),
    /// The budget it was prepared against, so an incompatible one is not activated.
    pub policy: Value,
    /// Which rows it was built from, as extraction records them.
    pub provenance: Value,
    pub content: String,
    pub made: Timestamp,
}

/// Why a generation cannot be activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stale {
    /// No generation of that id.
    Unknown,
    /// It is retired, or already active.
    NotReady(Ready),
    /// The transcript has moved on in a way this no longer represents.
    Amended { at: u64 },
    /// Prepared against more room than the request now has.
    Outgrown { prepared_for: u64, now: u64 },
}

impl Transcript {
    /// Write a generation in `Building`. The id is the caller's: a preparer that retries names
    /// the same one and overwrites its own partial work rather than leaving two.
    pub fn put_generation(&self, held: &Generation) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT INTO generation (id, session, state, parent, covers_from, covers_to, \
             policy, provenance, content, made) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) \
             ON CONFLICT(id) DO UPDATE SET state = excluded.state, parent = excluded.parent, \
             covers_from = excluded.covers_from, covers_to = excluded.covers_to, \
             policy = excluded.policy, provenance = excluded.provenance, \
             content = excluded.content, made = excluded.made",
            rusqlite::params![
                held.id,
                held.session.to_string(),
                held.state.as_str(),
                held.parent,
                held.covers.0,
                held.covers.1,
                held.policy.to_string(),
                held.provenance.to_string(),
                held.content,
                held.made,
            ],
        )?;
        Ok(())
    }

    /// One generation by id.
    pub fn generation(&self, id: &str) -> Result<Option<Generation>, StoreError> {
        let db = self.db();
        let mut asked = db.prepare(&format!("SELECT {FIELDS} FROM generation WHERE id = ?1"))?;
        let mut rows = asked.query([id])?;
        Ok(rows.next()?.map(read).transpose()?)
    }

    /// What a request is laid out from, when anything is.
    pub fn active_generation(&self, session: &SessionId) -> Result<Option<Generation>, StoreError> {
        let db = self.db();
        let mut asked = db.prepare(&format!(
            "SELECT {FIELDS} FROM generation WHERE session = ?1 AND state = 'active'"
        ))?;
        let mut rows = asked.query([session.to_string()])?;
        Ok(rows.next()?.map(read).transpose()?)
    }

    /// Every generation of a session, newest first.
    pub fn generations_of(&self, session: &SessionId) -> Result<Vec<Generation>, StoreError> {
        let db = self.db();
        let mut asked = db.prepare(&format!(
            "SELECT {FIELDS} FROM generation WHERE session = ?1 ORDER BY made DESC, id DESC"
        ))?;
        let found = asked
            .query_map([session.to_string()], read)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }

    /// Offer a built generation for activation.
    pub fn ready_generation(&self, id: &str) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE generation SET state = 'ready' WHERE id = ?1 AND state = 'building'",
            [id],
        )?;
        Ok(())
    }

    /// Make `id` the active generation and retire whatever was, in one transaction.
    ///
    /// `through` is how far the transcript has been amended: a generation representing rows that
    /// were revised after it was built no longer describes them, and is refused rather than
    /// activated. Appended rows beyond its span are harmless — it still represents its own.
    ///
    /// # Errors
    /// [`Stale`] when there is no such generation, it is not `Ready`, or its span was amended.
    pub fn activate_generation(
        &self,
        id: &str,
        amended_through: Option<u64>,
        limit_now: Option<u64>,
    ) -> Result<Result<(), Stale>, StoreError> {
        self.atomic(|held| {
            let Some(found) = held.generation(id)? else {
                return Ok(Err(Stale::Unknown));
            };
            if found.state != Ready::Ready {
                return Ok(Err(Stale::NotReady(found.state)));
            }
            if let Some(at) =
                amended_through.filter(|at| *at >= found.covers.0 && *at <= found.covers.1)
            {
                return Ok(Err(Stale::Amended { at }));
            }
            let prepared_for = found.policy["limit"].as_u64();
            if let (Some(prepared_for), Some(now)) = (prepared_for, limit_now)
                && prepared_for > now
            {
                return Ok(Err(Stale::Outgrown { prepared_for, now }));
            }
            held.db().execute(
                "UPDATE generation SET state = 'retired' WHERE session = ?1 AND state = 'active'",
                [found.session.to_string()],
            )?;
            held.db()
                .execute("UPDATE generation SET state = 'active' WHERE id = ?1", [id])?;
            Ok(Ok(()))
        })
    }

    /// Drop all but the newest `keep` retired generations of a session. Originals are untouched:
    /// this removes derived work only.
    pub fn trim_generations(&self, session: &SessionId, keep: usize) -> Result<usize, StoreError> {
        let retired: Vec<String> = self
            .generations_of(session)?
            .into_iter()
            .filter(|held| held.state == Ready::Retired)
            .skip(keep)
            .map(|held| held.id)
            .collect();
        for id in &retired {
            self.db()
                .execute("DELETE FROM generation WHERE id = ?1", [id])?;
        }
        Ok(retired.len())
    }
}

/// Which piece of a row has been read: where it starts, how long it is, and what the row said
/// when it was cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    pub offset: usize,
    pub length: usize,
}

impl Transcript {
    /// Record pieces of one row as read. Writing the same piece twice is the same as writing it
    /// once: the key is the piece, not the writing of it.
    pub fn read_piece(
        &self,
        session: &SessionId,
        cursor: u64,
        revision: &str,
        piece: Piece,
    ) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT INTO chunk (session, cursor, revision, offset, length) \
             VALUES (?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",
            rusqlite::params![
                session.to_string(),
                cursor,
                revision,
                piece.offset as i64,
                piece.length as i64,
            ],
        )?;
        Ok(())
    }

    /// The pieces of one row already read, for the revision named, lowest first.
    pub fn pieces_read(
        &self,
        session: &SessionId,
        cursor: u64,
        revision: &str,
    ) -> Result<Vec<Piece>, StoreError> {
        let db = self.db();
        let mut asked = db.prepare(
            "SELECT offset, length FROM chunk WHERE session = ?1 AND cursor = ?2 \
             AND revision = ?3 ORDER BY offset",
        )?;
        let found = asked
            .query_map(
                rusqlite::params![session.to_string(), cursor, revision],
                |row| {
                    Ok(Piece {
                        offset: row.get::<_, i64>(0)?.max(0) as usize,
                        length: row.get::<_, i64>(1)?.max(0) as usize,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }
}

const FIELDS: &str =
    "id, session, state, parent, covers_from, covers_to, policy, provenance, content, made";

fn read(row: &rusqlite::Row<'_>) -> Result<Generation, rusqlite::Error> {
    let said: String = row.get(2)?;
    let json = |at: usize| -> Result<Value, rusqlite::Error> {
        let text: String = row.get(at)?;
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    };
    Ok(Generation {
        id: row.get(0)?,
        session: SessionId::new(row.get::<_, String>(1)?),
        state: Ready::read(&said).unwrap_or(Ready::Retired),
        parent: row.get(3)?,
        covers: (row.get(4)?, row.get(5)?),
        policy: json(6)?,
        provenance: json(7)?,
        content: row.get(8)?,
        made: row.get(9)?,
    })
}

#[cfg(test)]
#[path = "generation/tests.rs"]
mod tests;
