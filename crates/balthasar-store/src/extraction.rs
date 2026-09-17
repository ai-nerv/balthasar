//! Successfully acknowledged extraction ranges and legacy coverage uncertainty.

use crate::{StoreError, Transcript};
use balthasar_model::SessionId;
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ExtractionProgress {
    pub through: Option<u64>,
    pub legacy_through: Option<u64>,
    pub blocked: Option<Value>,
}

impl Transcript {
    pub fn extraction_progress(
        &self,
        session: &SessionId,
    ) -> Result<ExtractionProgress, StoreError> {
        let row = self.db().query_row("SELECT through, legacy_through, blocked FROM extraction_progress WHERE session = ?1",
            [session.as_str()], |r| Ok((r.get::<_, Option<u64>>(0)?, r.get::<_, Option<u64>>(1)?, r.get::<_, Option<String>>(2)?))).optional()?;
        match row {
            Some((through, legacy_through, blocked)) => Ok(ExtractionProgress {
                through,
                legacy_through,
                blocked: blocked.map(|s| serde_json::from_str(&s)).transpose()?,
            }),
            None => {
                let legacy = self.counter(session, "extracted")?;
                Ok(ExtractionProgress {
                    through: legacy,
                    legacy_through: legacy,
                    blocked: None,
                })
            }
        }
    }

    fn prepare_extraction(&self, session: &SessionId) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT OR IGNORE INTO extraction_progress (session, through, legacy_through)
            VALUES (?1, (SELECT value FROM counter WHERE session = ?1 AND name = 'extracted'),
                        (SELECT value FROM counter WHERE session = ?1 AND name = 'extracted'))",
            [session.as_str()],
        )?;
        Ok(())
    }

    /// Record an input-bound failure, or clear it when a new bounded job is queued.
    pub fn block_extraction(
        &self,
        session: &SessionId,
        reason: Option<&Value>,
    ) -> Result<(), StoreError> {
        self.prepare_extraction(session)?;
        self.db().execute(
            "UPDATE extraction_progress SET blocked = ?2 WHERE session = ?1",
            params![session.as_str(), reason.map(ToString::to_string)],
        )?;
        Ok(())
    }

    /// Acknowledge represented coverage inside the job-completion transaction.
    pub fn acknowledge_extraction(
        &self,
        session: &SessionId,
        job: &str,
        from: u64,
        to: u64,
    ) -> Result<(), StoreError> {
        if from > to {
            return Err(StoreError::Foreign("invalid extraction coverage".into()));
        }
        self.prepare_extraction(session)?;
        self.db().execute("INSERT OR IGNORE INTO extraction_ack (job, session, from_cursor, to_cursor) VALUES (?1, ?2, ?3, ?4)", params![job, session.as_str(), from, to])?;
        let recorded: (String, u64, u64) = self.db().query_row(
            "SELECT session, from_cursor, to_cursor FROM extraction_ack WHERE job = ?1",
            [job],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if recorded != (session.as_str().to_owned(), from, to) {
            return Err(StoreError::Foreign(
                "extraction acknowledgment conflicts with its job".into(),
            ));
        }
        let old = self.extraction_progress(session)?.through;
        let mut through = old;
        let mut next = old.map_or(0, |n| n.saturating_add(1));
        let mut query = self.db().prepare("SELECT from_cursor, to_cursor FROM extraction_ack WHERE session = ?1 AND to_cursor >= ?2 ORDER BY from_cursor, to_cursor")?;
        for row in query.query_map(params![session.as_str(), old.unwrap_or(0)], |r| {
            Ok((r.get::<_, u64>(0)?, r.get::<_, u64>(1)?))
        })? {
            let (from, to) = row?;
            if from > next {
                break;
            }
            if to >= next {
                through = Some(to);
                next = to.saturating_add(1);
            }
        }
        if through != old {
            self.db().execute(
                "UPDATE extraction_progress SET through = ?2, blocked = NULL WHERE session = ?1",
                params![session.as_str(), through],
            )?;
            if let Some(to) = through {
                self.set_counter(session, "extracted", to)?;
            }
        }
        Ok(())
    }
}

pub(crate) const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS extraction_progress (
  session TEXT PRIMARY KEY, through INTEGER, legacy_through INTEGER, blocked TEXT
) STRICT;
CREATE TABLE IF NOT EXISTS extraction_ack (
  job TEXT PRIMARY KEY, session TEXT NOT NULL, from_cursor INTEGER NOT NULL, to_cursor INTEGER NOT NULL,
  CHECK (from_cursor >= 0 AND to_cursor >= from_cursor)
) STRICT;
CREATE INDEX IF NOT EXISTS extraction_ack_session ON extraction_ack(session, to_cursor);
";
