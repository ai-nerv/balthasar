//! Work handed to a helper model through the harness, and what came back.

use crate::{StoreError, Transcript};
use balthasar_model::{SessionId, Timestamp};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

/// Where a job stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// Made, and not yet handed out.
    Queued,
    /// Handed out, and waiting for its answer.
    Issued,
    Done,
    Failed,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Issued => "issued",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "queued" => Self::Queued,
            "issued" => Self::Issued,
            "done" => Self::Done,
            _ => Self::Failed,
        }
    }
}

/// One job.
#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: String,
    pub session: SessionId,
    pub kind: String,
    pub state: JobState,
    /// How many times it has been handed out.
    pub attempts: u32,
    /// The job as handed out, without its id.
    pub spec: Value,
    /// What only balthasar needs to settle it.
    pub context: Value,
    pub issued: Option<Timestamp>,
    pub result: Option<Value>,
}

impl Job {
    /// The job as a harness receives it.
    #[must_use]
    pub fn handed(&self) -> Value {
        let mut out = self.spec.clone();
        out["id"] = Value::String(self.id.clone());
        out
    }
}

impl Transcript {
    /// Make a job, to be handed out by the next `layout` or `jobs`.
    pub fn queue_job(
        &self,
        session: &SessionId,
        kind: &str,
        spec: &Value,
        context: &Value,
        at: Timestamp,
    ) -> Result<String, StoreError> {
        self.db().execute(
            "INSERT INTO job (session, kind, state, spec, context, created) \
             VALUES (?1, ?2, 'queued', ?3, ?4, ?5)",
            params![
                session.as_str(),
                kind,
                spec.to_string(),
                context.to_string(),
                at
            ],
        )?;
        Ok(format!("J-{}", self.db().last_insert_rowid()))
    }

    /// A job, by name.
    pub fn job(&self, id: &str) -> Result<Option<Job>, StoreError> {
        let Some(n) = id.strip_prefix("J-").and_then(|n| n.parse::<i64>().ok()) else {
            return Ok(None);
        };
        let found = self
            .db()
            .query_row(
                &format!("SELECT {JOB} FROM job WHERE n = ?1"),
                params![n],
                job_of,
            )
            .optional()?;
        found.transpose()
    }

    /// Every job of a run, oldest first.
    pub fn jobs_of(&self, session: &SessionId) -> Result<Vec<Job>, StoreError> {
        let mut statement = self.db().prepare(&format!(
            "SELECT {JOB} FROM job WHERE session = ?1 ORDER BY n"
        ))?;
        let found = statement
            .query_map(params![session.as_str()], job_of)?
            .collect::<Result<Vec<_>, _>>()?;
        found.into_iter().collect()
    }

    /// Record that a job was handed out.
    pub fn issue_job(&self, id: &str, at: Timestamp) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE job SET state = 'issued', attempts = attempts + 1, issued = ?2 \
             WHERE n = ?1",
            params![number(id), at],
        )?;
        Ok(())
    }

    /// Move a job on, with what came back when anything did.
    pub fn settle_job(
        &self,
        id: &str,
        state: JobState,
        result: Option<&Value>,
        at: Timestamp,
    ) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE job SET state = ?2, result = COALESCE(?3, result), settled = ?4 WHERE n = ?1",
            params![
                number(id),
                state.as_str(),
                result.map(ToString::to_string),
                at
            ],
        )?;
        Ok(())
    }
}

/// The columns [`job_of`] reads.
const JOB: &str = "n, session, kind, state, attempts, spec, context, issued, result";

fn job_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<Job, StoreError>> {
    let parse = |text: String| serde_json::from_str::<Value>(&text).map_err(StoreError::from);
    let (spec, context) = (r.get::<_, String>(5)?, r.get::<_, String>(6)?);
    let result = r.get::<_, Option<String>>(8)?;
    let (id, session, kind, state) = (
        format!("J-{}", r.get::<_, i64>(0)?),
        SessionId::new(r.get::<_, String>(1)?),
        r.get::<_, String>(2)?,
        JobState::parse(&r.get::<_, String>(3)?),
    );
    let (attempts, issued) = (r.get::<_, i64>(4)?.max(0) as u32, r.get(7)?);
    Ok((|| {
        Ok(Job {
            id,
            session,
            kind,
            state,
            attempts,
            spec: parse(spec)?,
            context: parse(context)?,
            issued,
            result: result.map(parse).transpose()?,
        })
    })())
}

fn number(id: &str) -> i64 {
    id.strip_prefix("J-")
        .and_then(|n| n.parse().ok())
        .unwrap_or(-1)
}

/// The job table.
pub(crate) const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS job (
  n         INTEGER PRIMARY KEY AUTOINCREMENT,
  session   TEXT NOT NULL,
  kind      TEXT NOT NULL,
  state     TEXT NOT NULL,
  attempts  INTEGER NOT NULL DEFAULT 0,
  spec      TEXT NOT NULL,
  context   TEXT NOT NULL,
  result    TEXT,
  created   INTEGER NOT NULL,
  issued    INTEGER,
  settled   INTEGER
) STRICT;
CREATE INDEX IF NOT EXISTS job_of ON job(session, n);
";
