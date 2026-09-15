//! What balthasar proposed for each request, what was confirmed, and what the provider's own
//! counts taught it. Kept beside the turns, in the scrollback's file.

use crate::{StoreError, Transcript, Turn};
use balthasar_model::{SessionId, Timestamp};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

/// One proposed layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub id: String,
    pub session: SessionId,
    /// Which model's factor it was estimated with.
    pub model: String,
    /// What it was estimated to cost, before the factor.
    pub estimated: u64,
    /// The layout, as it was answered.
    pub body: Value,
    /// What it answered, so an overflow can ask again.
    pub asked: Value,
    pub applied: Option<Timestamp>,
}

/// A summary standing in for a span of a run.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub from: u64,
    pub to: u64,
    pub text: String,
    pub at: Timestamp,
}

/// What one prompt has spent and settled so far.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prompt {
    /// What tells this prompt from the next one.
    pub mark: String,
    pub compactions: u32,
    pub overflows: u32,
    /// The memory slot, settled once per prompt.
    pub memory: Option<Value>,
    /// The pinned slot, likewise.
    pub pinned: Option<Value>,
    /// Which helper roles the harness said it can run.
    pub helpers: Vec<String>,
}

impl Transcript {
    /// Keep a layout as proposed, and name it.
    pub fn propose(
        &self,
        session: &SessionId,
        model: &str,
        estimated: u64,
        body: &Value,
        asked: &Value,
        at: Timestamp,
    ) -> Result<String, StoreError> {
        self.db().execute(
            "INSERT INTO layout (session, at, model, estimated, body, asked) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session.as_str(),
                at,
                model,
                i64::try_from(estimated).unwrap_or(i64::MAX),
                body.to_string(),
                asked.to_string()
            ],
        )?;
        Ok(format!("L-{}", self.db().last_insert_rowid()))
    }

    /// A proposed layout, by name.
    pub fn proposal(&self, id: &str) -> Result<Option<Proposal>, StoreError> {
        let Some(n) = id.strip_prefix("L-").and_then(|n| n.parse::<i64>().ok()) else {
            return Ok(None);
        };
        let found = self
            .db()
            .query_row(
                "SELECT session, model, estimated, body, asked, applied FROM layout WHERE n = ?1",
                params![n],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, Option<i64>>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((session, model, estimated, body, asked, applied)) = found else {
            return Ok(None);
        };
        Ok(Some(Proposal {
            id: id.to_owned(),
            session: SessionId::new(session),
            model,
            estimated: estimated.max(0) as u64,
            body: serde_json::from_str(&body)?,
            asked: serde_json::from_str(&asked)?,
            applied,
        }))
    }

    /// Record that a proposal was sent and accepted. False when it already was.
    pub fn confirm(&self, id: &str, real: Option<u64>, at: Timestamp) -> Result<bool, StoreError> {
        let Some(n) = id.strip_prefix("L-").and_then(|n| n.parse::<i64>().ok()) else {
            return Ok(false);
        };
        let changed = self.db().execute(
            "UPDATE layout SET applied = ?2, real = ?3 WHERE n = ?1 AND applied IS NULL",
            params![n, at, real.map(|r| i64::try_from(r).unwrap_or(i64::MAX))],
        )?;
        Ok(changed == 1)
    }

    /// A model's correction factor and how many counts it rests on, if it has been measured.
    pub fn measured(&self, model: &str) -> Result<Option<(f64, u32)>, StoreError> {
        Ok(self
            .db()
            .query_row(
                "SELECT factor, samples FROM factor WHERE model = ?1",
                params![model],
                |r| Ok((r.get::<_, f64>(0)?, r.get::<_, i64>(1)?.max(0) as u32)),
            )
            .optional()?)
    }

    /// Set a model's correction factor.
    pub fn set_factor(&self, model: &str, factor: f64) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT INTO factor (model, factor, samples) VALUES (?1, ?2, 1) \
             ON CONFLICT(model) DO UPDATE SET factor = ?2, samples = samples + 1",
            params![model, factor],
        )?;
        Ok(())
    }

    /// The newest summary of a run.
    pub fn summary(&self, session: &SessionId) -> Result<Option<Summary>, StoreError> {
        Ok(self
            .db()
            .query_row(
                "SELECT from_cursor, to_cursor, text, at FROM summary WHERE session = ?1 \
                 ORDER BY n DESC LIMIT 1",
                params![session.as_str()],
                |r| {
                    Ok(Summary {
                        from: r.get::<_, i64>(0)?.max(0) as u64,
                        to: r.get::<_, i64>(1)?.max(0) as u64,
                        text: r.get(2)?,
                        at: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Keep a summary. The newest is the one a layout uses; older ones stay as the record.
    pub fn keep_summary(
        &self,
        session: &SessionId,
        from: u64,
        to: u64,
        text: &str,
        at: Timestamp,
    ) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT INTO summary (session, from_cursor, to_cursor, text, at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session.as_str(),
                i64::try_from(from).unwrap_or(i64::MAX),
                i64::try_from(to).unwrap_or(i64::MAX),
                text,
                at
            ],
        )?;
        Ok(())
    }

    /// What the current prompt of a run has spent and settled.
    pub fn prompt(&self, session: &SessionId) -> Result<Option<Prompt>, StoreError> {
        let found = self
            .db()
            .query_row(
                "SELECT mark, compactions, overflows, memory, pinned, helpers FROM prompt \
                 WHERE session = ?1",
                params![session.as_str()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((mark, compactions, overflows, memory, pinned, helpers)) = found else {
            return Ok(None);
        };
        let parse = |held: Option<String>| held.and_then(|s| serde_json::from_str(&s).ok());
        Ok(Some(Prompt {
            mark,
            compactions: compactions.max(0) as u32,
            overflows: overflows.max(0) as u32,
            memory: parse(memory),
            pinned: parse(pinned),
            helpers: helpers
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
        }))
    }

    /// Keep what the current prompt has spent and settled.
    pub fn keep_prompt(&self, session: &SessionId, prompt: &Prompt) -> Result<(), StoreError> {
        let text = |held: &Option<Value>| held.as_ref().map(ToString::to_string);
        self.db().execute(
            "INSERT INTO prompt (session, mark, compactions, overflows, memory, pinned, helpers) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(session) DO UPDATE SET mark = ?2, compactions = ?3, overflows = ?4, \
               memory = ?5, pinned = ?6, helpers = ?7",
            params![
                session.as_str(),
                prompt.mark,
                prompt.compactions,
                prompt.overflows,
                text(&prompt.memory),
                text(&prompt.pinned),
                serde_json::json!(prompt.helpers).to_string()
            ],
        )?;
        Ok(())
    }

    /// Every turn of a run without its words or its raw record: what a layout needs, and no
    /// more. A turn's cost comes back counted or estimated, never as nothing.
    pub fn outline(&self, session: &SessionId) -> Result<Vec<Turn>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT cursor, at, role, kind, '', tool, NULL, entry, \
                    COALESCE(tokens, (LENGTH(text) + 3) / 4), \
                    state, pinned, ok, ms, args, revisions, grp, stub, handle, keep, error \
             FROM turn WHERE session = ?1 ORDER BY cursor",
        )?;
        let found = statement
            .query_map(params![session.as_str()], crate::transcript::read)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }
}

/// The tables behind the layouts.
pub(crate) const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS layout (
  n          INTEGER PRIMARY KEY AUTOINCREMENT,
  session    TEXT NOT NULL,
  at         INTEGER NOT NULL,
  model      TEXT NOT NULL,
  estimated  INTEGER NOT NULL,
  body       TEXT NOT NULL,
  asked      TEXT NOT NULL,
  applied    INTEGER,
  real       INTEGER
) STRICT;
CREATE INDEX IF NOT EXISTS layout_of ON layout(session, n);

CREATE TABLE IF NOT EXISTS factor (
  model    TEXT PRIMARY KEY,
  factor   REAL NOT NULL,
  samples  INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS summary (
  n            INTEGER PRIMARY KEY AUTOINCREMENT,
  session      TEXT NOT NULL,
  from_cursor  INTEGER NOT NULL,
  to_cursor    INTEGER NOT NULL,
  text         TEXT NOT NULL,
  at           INTEGER NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS summary_of ON summary(session, n);

CREATE TABLE IF NOT EXISTS prompt (
  session      TEXT PRIMARY KEY,
  mark         TEXT NOT NULL,
  compactions  INTEGER NOT NULL DEFAULT 0,
  overflows    INTEGER NOT NULL DEFAULT 0,
  memory       TEXT,
  pinned       TEXT,
  helpers      TEXT
) STRICT;
";
