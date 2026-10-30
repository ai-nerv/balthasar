//! How far into a run the rules have read, so a run that goes on is read again from where it
//! stopped instead of from the start.

use crate::{Store, StoreError};
use balthasar_model::Timestamp;
use rusqlite::{OptionalExtension, params};

/// What the rules at a version have read of one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunRead {
    /// Nothing yet.
    Unread,
    /// All of it, by a build that kept no cursor.
    Whole,
    /// Every turn up to and including this cursor.
    Through(u64),
}

impl RunRead {
    /// Whether the turn at `cursor` is still to be read.
    #[must_use]
    pub fn pending(self, cursor: u64) -> bool {
        match self {
            Self::Unread => true,
            Self::Whole => false,
            Self::Through(through) => cursor > through,
        }
    }
}

impl Store {
    /// What the rules at `version` or later have read of `reference`.
    pub fn read_of(
        &self,
        source: &str,
        reference: &str,
        version: i64,
    ) -> Result<RunRead, StoreError> {
        let found: Option<(i64, Option<i64>)> = self
            .db()
            .query_row(
                "SELECT version, through FROM stamp \
                 WHERE source = ?1 AND ref = ?2 AND extractor = 'rules'",
                params![source, reference],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match found {
            Some((held, through)) if held >= version => {
                through.map_or(RunRead::Whole, |c| RunRead::Through(c.max(0) as u64))
            }
            _ => RunRead::Unread,
        })
    }

    /// Note that the rules at `version` have read `reference` up to and including `through`.
    pub fn stamp_through(
        &mut self,
        source: &str,
        reference: &str,
        version: i64,
        through: u64,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT INTO stamp (source, ref, extractor, version, at, through) \
             VALUES (?1, ?2, 'rules', ?3, ?4, ?5) \
             ON CONFLICT(source, ref, extractor) DO UPDATE SET version = ?3, at = ?4, through = ?5",
            params![source, reference, version, now, through as i64],
        )?;
        Ok(())
    }
}
