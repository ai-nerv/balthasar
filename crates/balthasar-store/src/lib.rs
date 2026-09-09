//! Where memory is kept.
//!
//! SQLite, one file per scope, WAL. Nothing is deleted: superseded, contradicted, decayed past
//! the floor and forgotten on purpose are all columns, and the one statement in this crate that
//! removes a row lives in [`purge`]. The partial unique index in the schema makes two
//! simultaneously-true answers to one slot impossible at the database level.

mod collide;
mod decay;
mod entity;
mod episode;
mod export;
mod layout;
mod mint;
mod paths;
mod purge;
mod read;
mod relate;
mod row;
mod schema;
mod score;
mod scratchpad;
mod scroll;
mod session;
mod spans;
mod transcript;
mod usage;
mod write;

pub use decay::{Faded, Weakened};
pub use entity::{Entity, Kind as EntityKind, extract as entities_in, rarity};
pub use episode::Episode;
pub use export::Row as TrainingRow;
pub use mint::mint;
pub use paths::{
    HOME, Tool, data_dir, home_of, make_home, project_home, run_dir_in, scope_of, scope_path,
    session_dir, session_dir_in, session_path, tools_in,
};
pub use purge::{
    Closure, closure_of, purge, purge_domain, purge_run, purge_scratch, purge_session,
};
pub use read::{Cluster, Recall};
pub use relate::Reach;
pub use schema::VERSION as SCHEMA_VERSION;
pub use score::{Scored, Weights, cosine, coverage, frecency, fts_query};
pub use scratchpad::Scratchpad;
pub use scroll::{Budget, Read, Want, tokens_of};
pub use session::{Session, name_for};
pub use spans::Span;
pub use transcript::{Run, State, Transcript, Turn, transcript_path};
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
    /// A row came back with a column this build does not understand, meaning the file was
    /// written by a different balthasar.
    #[error("this store holds a '{0}' that this build does not know")]
    Foreign(String),
    /// A caller referred to something that is not in this store.
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
    /// WAL, because consolidation reads while a session writes. `foreign_keys` on, so a witness
    /// cannot outlive the memory it is evidence for. `secure_delete` on, because SQLite does not
    /// zero a freed page by default and the words stay in the file where `strings` finds them.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Io(parent.to_owned(), e))?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "secure_delete", true)?;
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

    /// How many bytes this store occupies. Asked of SQLite rather than the filesystem, so an
    /// in-memory store answers too.
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

    pub(crate) fn db(&self) -> &Connection {
        &self.connection
    }

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
        // FTS5 is the retrieval floor: without it there is no non-embedding path.
        let store = Store::ephemeral().expect("open");
        store
            .db()
            .execute_batch("CREATE VIRTUAL TABLE probe USING fts5(body);")
            .expect("FTS5 must be compiled in");
    }
}
