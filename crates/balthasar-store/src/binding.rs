//! Durable transcript-to-run bindings for agent-local scratch.

use crate::{StoreError, Transcript};
use balthasar_model::SessionId;
use rusqlite::{Connection, OptionalExtension, params};

impl Transcript {
    /// The scratch run for a transcript, or its own key for an unbound transcript.
    pub fn run_of(&self, transcript: &SessionId) -> Result<SessionId, StoreError> {
        let run: Option<String> = self
            .db()
            .query_row(
                "SELECT run FROM transcript_run WHERE session = ?1",
                [transcript.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(run.map_or_else(|| transcript.clone(), SessionId::new))
    }

    /// Bind a transcript once, retaining its binding when the caller omits `run`.
    pub fn bind_run(&self, transcript: &SessionId, run: Option<&str>) -> Result<(), StoreError> {
        let tx = rusqlite::Transaction::new_unchecked(
            self.db(),
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let run = decide(&tx, transcript, run)?;
        pin(&tx, transcript, &run)?;
        tx.commit()?;
        Ok(())
    }

    /// The run a transcript would be bound to given `run`, without binding it. A run that
    /// contradicts the one already recorded is refused.
    pub fn run_for(&self, transcript: &SessionId, run: Option<&str>) -> Result<String, StoreError> {
        decide(self.db(), transcript, run)
    }

    /// Record a binding already decided by [`Transcript::run_for`]. One already there stays.
    pub fn pin_run(&self, transcript: &SessionId, run: &str) -> Result<(), StoreError> {
        pin(self.db(), transcript, run)
    }
}

fn decide(
    db: &Connection,
    transcript: &SessionId,
    run: Option<&str>,
) -> Result<String, StoreError> {
    if run.is_some_and(|run| run.trim().is_empty()) {
        return Err(StoreError::Foreign("empty scratch run".into()));
    }
    let held: Option<String> = db
        .query_row(
            "SELECT run FROM transcript_run WHERE session = ?1",
            [transcript.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    let recorded: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM turn WHERE session = ?1)",
        [transcript.as_str()],
        |row| row.get(0),
    )?;
    let previous = held
        .as_deref()
        .or_else(|| recorded.then_some(transcript.as_str()));
    if let (Some(previous), Some(run)) = (previous, run)
        && previous != run
    {
        return Err(StoreError::Foreign("conflicting transcript run".into()));
    }
    Ok(previous.or(run).unwrap_or(transcript.as_str()).to_owned())
}

fn pin(db: &Connection, transcript: &SessionId, run: &str) -> Result<(), StoreError> {
    db.execute(
        "INSERT OR IGNORE INTO transcript_run (session, run) VALUES (?1, ?2)",
        params![transcript.as_str(), run],
    )?;
    Ok(())
}
