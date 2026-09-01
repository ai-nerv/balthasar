//! Where memory is kept.
//!
//! SQLite, one file per scope, WAL. Not interesting and not negotiable: the reference
//! implementations reach 156 µs ingest and 568 µs search on it, and a memory layer that needs a
//! server running is a memory layer nobody runs.
//!
//! Two rules hold everywhere below, and everything else is detail.
//!
//! **Nothing is deleted.** Superseded, contradicted, decayed past the floor, forgotten on
//! purpose — every one of those is a column, not a `DELETE`. There is exactly one statement in
//! this crate that removes a row and it lives in [`purge`], behind a confirmation, because
//! "delete the key I pasted" must be answerable with yes.
//!
//! **A fact answers for itself.** The partial unique index in the schema makes two
//! simultaneously-true answers to one slot impossible at the database level, so contradiction
//! handling is a constraint that fails loudly rather than a policy code must remember.

mod decay;
mod entity;
mod episode;
mod ledger;
mod mint;
mod paths;
mod purge;
mod read;
mod relate;
mod row;
mod schema;
mod score;
mod scratchpad;
mod session;
mod transcript;
mod usage;
mod write;

pub use decay::{Faded, Weakened};
pub use entity::{Entity, Kind as EntityKind, extract as entities_in, rarity};
pub use episode::Episode;
pub use ledger::{Entry, State};
pub use mint::mint;
pub use paths::{
    HOME, Tool, data_dir, home_of, make_home, project_home, scope_of, scope_path, session_dir,
    session_dir_in, session_path, session_transcript_path, tools_in,
};
pub use purge::{Closure, closure_of, purge};
pub use read::{Cluster, Recall};
pub use relate::Reach;
pub use schema::VERSION as SCHEMA_VERSION;
pub use score::{Scored, Weights, cosine, coverage, frecency};
pub use scratchpad::Scratchpad;
pub use session::{Session, name_for};
pub use transcript::{Run, Transcript, Turn, transcript_path};
pub use usage::{Candidate, Injection, RecallRun, Signals, Trace, TracedAction, Use, Verdict};
pub use write::Landing;

use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// What went wrong.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite said no.
    #[error("the store: {0}")]
    Sql(#[from] rusqlite::Error),
    /// A body or a witness would not encode.
    #[error("a record would not encode: {0}")]
    Encode(#[from] serde_json::Error),
    /// The store's directory could not be made.
    #[error("{0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
    /// A row came back with a column this build does not understand.
    ///
    /// Its own variant rather than a generic decode failure: it means the file was written by
    /// a different aeon, and telling somebody that is more useful than telling them a string
    /// did not parse.
    #[error("this store holds a '{0}' that this build does not know")]
    Foreign(String),
    /// A caller referred to something that is not in this store.
    ///
    /// Its own variant rather than a foreign-key failure reaching the surface: "no injection
    /// called 'i-nope'" tells a caller what to fix, and "FOREIGN KEY constraint failed" tells
    /// them to read someone else's schema.
    #[error("no {0}")]
    Unknown(String),
}

/// One scope's memories.
pub struct Store {
    connection: Connection,
    path: PathBuf,
}

impl Store {
    /// Open the store at `path`, creating and migrating it if need be.
    ///
    /// WAL, because consolidation reads while a session writes and the default journal mode
    /// makes those wait for each other. `foreign_keys` on, so a witness cannot outlive the
    /// memory it is evidence for.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Io(parent.to_owned(), e))?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        // A busy store is a store being consolidated. Waiting is right; failing is not.
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        schema::migrate(&connection)?;
        Ok(Self {
            connection,
            path: path.to_owned(),
        })
    }

    /// A store in memory, for tests and for `--dry-run`.
    pub fn ephemeral() -> Result<Self, StoreError> {
        let connection = Connection::open_in_memory()?;
        connection.pragma_update(None, "foreign_keys", true)?;
        schema::migrate(&connection)?;
        Ok(Self {
            connection,
            path: PathBuf::from(":memory:"),
        })
    }

    /// How many bytes this store occupies.
    ///
    /// Asked of SQLite rather than the filesystem, so it answers the same way for an in-memory
    /// store as for a file — a benchmark runs in memory and still has to report what it cost.
    pub fn bytes(&self) -> Result<u64, StoreError> {
        let pages: i64 = self
            .connection
            .pragma_query_value(None, "page_count", |r| r.get(0))?;
        let size: i64 = self
            .connection
            .pragma_query_value(None, "page_size", |r| r.get(0))?;
        Ok((pages.max(0) as u64) * (size.max(0) as u64))
    }
    /// Where this store lives.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The connection, for the modules that make up this crate.
    pub(crate) fn db(&self) -> &Connection {
        &self.connection
    }

    /// The connection, mutably, for the ones that need a transaction.
    pub(crate) fn db_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_store_has_the_schema() {
        let store = Store::ephemeral().expect("open");
        let tables: i64 = store
            .db()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN \
                 ('memory', 'witness', 'link', 'session', 'stamp')",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(tables, 5);
    }

    #[test]
    fn full_text_search_is_available() {
        // FTS5 is the retrieval floor: without it there is no non-embedding path, and
        // commitment 3 is a sentence rather than a fact. Better to find out here than at M4.
        let store = Store::ephemeral().expect("open");
        store
            .db()
            .execute_batch("CREATE VIRTUAL TABLE probe USING fts5(body);")
            .expect("FTS5 must be compiled in");
    }
}
