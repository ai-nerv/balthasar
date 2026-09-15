//! Notes a project keeps about itself, and every change made to them, with what it replaced.

use crate::{StoreError, Transcript};
use balthasar_model::{SessionId, Timestamp};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

/// One note.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub id: String,
    pub title: String,
    pub text: String,
    /// One line, shown in the list of notes.
    pub description: String,
    /// In front of the model every time, rather than listed.
    pub pinned: bool,
    pub retired: Option<Timestamp>,
    pub updated: Timestamp,
}

impl Note {
    /// What a change records of it.
    #[must_use]
    pub fn fields(&self) -> Value {
        json!({ "title": self.title, "text": self.text, "description": self.description,
                "pinned": self.pinned })
    }
}

/// One change to the notes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Change {
    pub id: String,
    pub note: Option<String>,
    /// `add`, `update`, `retire` or `revert`.
    pub op: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
    /// `staged`, `applied`, `rejected` or `undone`.
    pub state: String,
    /// The job that proposed it, or who did.
    pub by: String,
    pub at: Timestamp,
}

impl Change {
    /// As the change log answers it.
    #[must_use]
    pub fn as_json(&self) -> Value {
        json!({ "id": self.id, "note": self.note, "op": self.op, "before": self.before,
                "after": self.after, "at": self.at, "by": self.by, "state": self.state })
    }
}

impl Transcript {
    /// Every live note, pinned first.
    pub fn notes(&self) -> Result<Vec<Note>, StoreError> {
        let mut statement = self.db().prepare(&format!(
            "SELECT {NOTE} FROM note WHERE retired IS NULL ORDER BY pinned DESC, n"
        ))?;
        let found = statement
            .query_map([], note_of)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }

    /// One note, live or retired.
    pub fn note(&self, id: &str) -> Result<Option<Note>, StoreError> {
        Ok(self
            .db()
            .query_row(
                &format!("SELECT {NOTE} FROM note WHERE n = ?1"),
                params![number(id, "N-")],
                note_of,
            )
            .optional()?)
    }

    /// Make a note stand as `after`: a new one when there is no `id`, retired when there is no
    /// `after`. Answers which note it was.
    pub fn put_note(
        &self,
        id: Option<&str>,
        after: Option<&Value>,
        at: Timestamp,
    ) -> Result<Option<String>, StoreError> {
        let field = |name: &str| {
            after
                .and_then(|a| a[name].as_str())
                .unwrap_or_default()
                .to_owned()
        };
        let pinned = i64::from(after.and_then(|a| a["pinned"].as_bool()).unwrap_or(false));
        match (id, after) {
            (None, None) => Ok(None),
            (Some(id), None) => {
                self.db().execute(
                    "UPDATE note SET retired = ?2, updated = ?2 WHERE n = ?1",
                    params![number(id, "N-"), at],
                )?;
                Ok(Some(id.to_owned()))
            }
            (None, Some(_)) => {
                self.db().execute(
                    "INSERT INTO note (title, text, description, pinned, created, updated) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    params![
                        field("title"),
                        field("text"),
                        field("description"),
                        pinned,
                        at
                    ],
                )?;
                Ok(Some(format!("N-{}", self.db().last_insert_rowid())))
            }
            (Some(id), Some(_)) => {
                self.db().execute(
                    "UPDATE note SET title = ?2, text = ?3, description = ?4, pinned = ?5, \
                     retired = NULL, updated = ?6 WHERE n = ?1",
                    params![
                        number(id, "N-"),
                        field("title"),
                        field("text"),
                        field("description"),
                        pinned,
                        at
                    ],
                )?;
                Ok(Some(id.to_owned()))
            }
        }
    }

    /// Log a change; its `id` is ignored and the one it is given is answered.
    pub fn record_change(
        &self,
        change: &Change,
        session: Option<&SessionId>,
    ) -> Result<String, StoreError> {
        let text = |held: &Option<Value>| held.as_ref().map(ToString::to_string);
        self.db().execute(
            "INSERT INTO note_change (note, op, before, after, state, by, session, at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                change.note,
                change.op,
                text(&change.before),
                text(&change.after),
                change.state,
                change.by,
                session.map(SessionId::as_str),
                change.at
            ],
        )?;
        Ok(format!("C-{}", self.db().last_insert_rowid()))
    }

    /// One change, by name.
    pub fn change(&self, id: &str) -> Result<Option<Change>, StoreError> {
        let found = self
            .db()
            .query_row(
                &format!("SELECT {CHANGE} FROM note_change WHERE n = ?1"),
                params![number(id, "C-")],
                change_of,
            )
            .optional()?;
        found.transpose()
    }

    /// The change log, newest first.
    pub fn changes(&self, limit: usize) -> Result<Vec<Change>, StoreError> {
        let mut statement = self.db().prepare(&format!(
            "SELECT {CHANGE} FROM note_change ORDER BY n DESC LIMIT ?1"
        ))?;
        let found = statement
            .query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], change_of)?
            .collect::<Result<Vec<_>, _>>()?;
        found.into_iter().collect()
    }

    /// Move a change on, naming the note it made when it made one.
    pub fn set_change(&self, id: &str, state: &str, note: Option<&str>) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE note_change SET state = ?2, note = COALESCE(?3, note) WHERE n = ?1",
            params![number(id, "C-"), state, note],
        )?;
        Ok(())
    }

    /// A count or cursor kept for a run, or for the project under the empty session.
    pub fn counter(&self, session: &SessionId, name: &str) -> Result<Option<u64>, StoreError> {
        Ok(self
            .db()
            .query_row(
                "SELECT value FROM counter WHERE session = ?1 AND name = ?2",
                params![session.as_str(), name],
                |r| r.get::<_, i64>(0).map(|n| n.max(0) as u64),
            )
            .optional()?)
    }

    /// Set a count or cursor.
    pub fn set_counter(
        &self,
        session: &SessionId,
        name: &str,
        value: u64,
    ) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT INTO counter (session, name, value) VALUES (?1, ?2, ?3) \
             ON CONFLICT(session, name) DO UPDATE SET value = ?3",
            params![
                session.as_str(),
                name,
                i64::try_from(value).unwrap_or(i64::MAX)
            ],
        )?;
        Ok(())
    }
}

const NOTE: &str = "n, title, text, description, pinned, retired, updated";

fn note_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: format!("N-{}", r.get::<_, i64>(0)?),
        title: r.get(1)?,
        text: r.get(2)?,
        description: r.get(3)?,
        pinned: r.get::<_, i64>(4)? != 0,
        retired: r.get(5)?,
        updated: r.get(6)?,
    })
}

const CHANGE: &str = "n, note, op, before, after, state, by, at";

fn change_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Change, StoreError>> {
    let parse = |held: Option<String>| {
        held.map(|text| serde_json::from_str::<Value>(&text))
            .transpose()
            .map_err(StoreError::from)
    };
    let (before, after) = (r.get(3)?, r.get(4)?);
    let change = Change {
        id: format!("C-{}", r.get::<_, i64>(0)?),
        note: r.get(1)?,
        op: r.get(2)?,
        state: r.get(5)?,
        by: r.get(6)?,
        at: r.get(7)?,
        ..Change::default()
    };
    Ok(parse(before).and_then(|before| {
        Ok(Change {
            before,
            after: parse(after)?,
            ..change
        })
    }))
}

fn number(id: &str, prefix: &str) -> i64 {
    id.strip_prefix(prefix)
        .and_then(|n| n.parse().ok())
        .unwrap_or(-1)
}

/// The notes, their change log, and the counts that pace the jobs keeping them.
pub(crate) const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS note (
  n            INTEGER PRIMARY KEY AUTOINCREMENT,
  title        TEXT NOT NULL,
  text         TEXT NOT NULL,
  description  TEXT NOT NULL,
  pinned       INTEGER NOT NULL DEFAULT 0,
  retired      INTEGER,
  created      INTEGER NOT NULL,
  updated      INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS note_change (
  n        INTEGER PRIMARY KEY AUTOINCREMENT,
  note     TEXT,
  op       TEXT NOT NULL,
  before   TEXT,
  after    TEXT,
  state    TEXT NOT NULL,
  by       TEXT NOT NULL,
  session  TEXT,
  at       INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS counter (
  session  TEXT NOT NULL,
  name     TEXT NOT NULL,
  value    INTEGER NOT NULL,
  PRIMARY KEY (session, name)
) STRICT;
";
