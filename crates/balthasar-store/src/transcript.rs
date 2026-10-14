//! The scrollback, kept verbatim, in a file of its own beside the memory store.
//!
//! The system of record: opened with `synchronous = FULL`. A turn's `raw` is the harness's own
//! record, kept as a string and never parsed, beside a small projection balthasar understands.

use crate::StoreError;
use balthasar_model::{SessionId, Timestamp};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

/// What has become of one turn in the window. A turn is never removed to make room: the text
/// stays and the state says what is sent, which is what makes compaction reversible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Sent as it was written.
    #[default]
    Live,
    /// Sent as a short description of itself.
    Masked,
    /// Not sent. Covered by a summary, or simply too old to matter.
    Dropped,
    /// Part of the span a summary stands in for.
    Summarised,
}

impl State {
    /// The column spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Masked => "masked",
            Self::Dropped => "dropped",
            Self::Summarised => "summarised",
        }
    }

    /// Whether a turn in this state still costs its full length.
    #[must_use]
    pub fn is_live(self) -> bool {
        matches!(self, Self::Live)
    }
}

impl std::str::FromStr for State {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "live" => Ok(Self::Live),
            "masked" => Ok(Self::Masked),
            "dropped" => Ok(Self::Dropped),
            "summarised" => Ok(Self::Summarised),
            other => Err(other.to_owned()),
        }
    }
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One turn, as it was written.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Turn {
    /// Where in the session. The harness's own numbering, never balthasar's.
    pub cursor: u64,
    /// When it settled.
    pub at: Timestamp,
    /// Who said it.
    pub role: String,
    /// What kind of turn.
    pub kind: String,
    /// What was said, for quoting and for search.
    #[serde(default)]
    pub text: String,
    /// Which tool, when it is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Which message this block belongs to, as the harness names it.
    ///
    /// One assistant message carries several blocks, each written as its own turn; they share
    /// this so a read never hands back half a message. `None` means the turn is its own message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// What this cost, when the harness knows.
    ///
    /// `None` is the ordinary case for a turn nobody counted, and callers fall back to the
    /// estimate — a missing count must never read as a free turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u32>,
    /// Whether a tool call succeeded; a call that failed and then succeeded is a repair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// How long a tool took, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    /// What the tool was asked for, as the harness's own JSON, held as text and never parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    /// What is sent for this turn right now.
    #[serde(default)]
    pub state: State,
    /// Whether a plan may not touch it.
    #[serde(default)]
    pub pinned: bool,
    /// The harness's own record, verbatim. balthasar stores it, hands it back, never looks inside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    /// How many times this turn has been revised: a tool entry is written when the call is made
    /// and revised when the result arrives.
    #[serde(default)]
    pub revisions: u32,
    /// The cursor of the assistant entry a tool row belongs to. A group is kept, stubbed,
    /// summarised or dropped whole; `None` makes the turn a group of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u64>,
    /// What the tool said to show in place of its result, when it said anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stub: Option<String>,
    /// How to get the full result back again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    /// Never stubbed or dropped.
    #[serde(default)]
    pub keep: bool,
    /// The tool call failed.
    #[serde(default)]
    pub error: bool,
}

impl Turn {
    /// Which group this turn is sent with: its own cursor when the harness named none.
    #[must_use]
    pub fn unit(&self) -> u64 {
        self.group.unwrap_or(self.cursor)
    }

    /// Whether this is a tool's row rather than something a person or the model said.
    #[must_use]
    pub fn is_tool(&self) -> bool {
        self.role == "tool"
            || matches!(self.kind.as_str(), "tool" | "tool_result")
            || (self.tool.is_some() && self.role != "assistant" && self.role != "user")
    }

    /// What this turn actually costs to send, given its state. A masked turn costs what its
    /// replacement costs rather than nothing: the stub still occupies room.
    #[must_use]
    pub fn cost(&self, masked_cost: u32) -> u32 {
        match self.state {
            State::Live => self.weight(),
            State::Masked => masked_cost.min(self.weight()),
            State::Dropped | State::Summarised => 0,
        }
    }

    /// What sending this turn in full would cost, whatever state it is in: the harness's own
    /// count when it has one, and an estimate otherwise.
    #[must_use]
    pub fn weight(&self) -> u32 {
        u32::try_from(crate::tokens_of(self)).unwrap_or(u32::MAX)
    }
}

/// One run, as the scrollback records it.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// The harness's identity for it.
    pub session: SessionId,
    /// Which project.
    pub scope: String,
    /// Where it ran.
    pub cwd: String,
    /// Which harness.
    pub harness: String,
    /// When it started.
    pub opened: Timestamp,
    /// When it ended, if it has.
    pub closed: Option<Timestamp>,
    /// How many turns it holds.
    pub turns: u64,
}

/// The scrollback for one project.
pub struct Transcript {
    connection: Connection,
    path: PathBuf,
}

/// Where a project's scrollback lives, beside the memory store and named after it.
#[must_use]
pub fn transcript_path(scope: &balthasar_model::ScopeId, tool: &crate::Tool) -> PathBuf {
    let memory = crate::scope_path(scope, tool);
    let stem = memory
        .file_stem()
        .map_or_else(|| "scope".to_owned(), |s| s.to_string_lossy().into_owned());
    memory.with_file_name(format!("{stem}-transcript.db"))
}

impl Transcript {
    /// Open the scrollback at `path`, creating it if need be. `synchronous = FULL` rather than
    /// the WAL default of `NORMAL`, which can lose the last transactions to a power cut.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        crate::private::before_open(path)?;
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        // The scrollback holds whatever was pasted into it verbatim, which makes it the most
        // likely place for a secret to be sitting. A freed page SQLite has not zeroed keeps it.
        connection.pragma_update(None, "secure_delete", true)?;
        crate::private::tighten(path)?;
        prepare(&connection)?;
        Ok(Self {
            connection,
            path: path.to_owned(),
        })
    }

    /// A scrollback in memory, for tests.
    pub fn ephemeral() -> Result<Self, StoreError> {
        let connection = Connection::open_in_memory()?;
        prepare(&connection)?;
        Ok(Self {
            connection,
            path: PathBuf::from(":memory:"),
        })
    }

    /// The connection, for the modules that read this store.
    pub(crate) fn db(&self) -> &Connection {
        &self.connection
    }

    /// Where it lives.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Record that a run has started, or take the one already recorded.
    pub fn open_run(
        &mut self,
        session: &SessionId,
        scope: &str,
        cwd: &str,
        harness: &str,
        opened: Timestamp,
    ) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT OR IGNORE INTO run (session, scope, cwd, harness, opened) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session.as_str(), scope, cwd, harness, opened],
        )?;
        Ok(())
    }

    /// Record that a run has ended.
    pub fn close_run(&mut self, session: &SessionId, at: Timestamp) -> Result<(), StoreError> {
        self.connection.execute(
            "UPDATE run SET closed = ?2 WHERE session = ?1 AND closed IS NULL",
            params![session.as_str(), at],
        )?;
        Ok(())
    }

    /// Everything a run said, in order. Answers the turns as they finally stood, not as they
    /// were first written — a tool call comes back with its result.
    pub fn replay(&self, session: &SessionId) -> Result<Vec<Turn>, StoreError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {COLUMNS} FROM turn WHERE session = ?1 ORDER BY cursor"
        ))?;
        let found = statement
            .query_map(params![session.as_str()], read)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }

    /// The turns from `from` on, in order, handed one at a time to `take` until it says stop.
    ///
    /// Stepped rather than collected, so what is not taken is not read.
    pub fn replay_while(
        &self,
        session: &SessionId,
        from: u64,
        mut take: impl FnMut(Turn) -> bool,
    ) -> Result<(), StoreError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {COLUMNS} FROM turn WHERE session = ?1 AND cursor >= ?2 ORDER BY cursor"
        ))?;
        let floor = i64::try_from(from).unwrap_or(i64::MAX);
        let mut rows = statement.query(params![session.as_str(), floor])?;
        while let Some(row) = rows.next()? {
            if !take(read(row)?) {
                break;
            }
        }
        Ok(())
    }

    /// How many turns a run holds.
    pub fn count(&self, session: &SessionId) -> Result<u64, StoreError> {
        let held: i64 = self.connection.query_row(
            "SELECT count(*) FROM turn WHERE session = ?1",
            params![session.as_str()],
            |r| r.get(0),
        )?;
        Ok(held.max(0) as u64)
    }

    /// One turn, for quoting.
    pub fn at(&self, session: &SessionId, cursor: u64) -> Result<Option<Turn>, StoreError> {
        let found = self
            .connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM turn WHERE session = ?1 AND cursor = ?2"),
                params![session.as_str(), cursor as i64],
                read,
            )
            .optional()?;
        Ok(found)
    }

    /// The cursor a resuming harness should allocate next: one past the highest written.
    /// Guessing wrong overwrites a turn.
    pub fn next_cursor(&self, session: &SessionId) -> Result<u64, StoreError> {
        let highest: Option<i64> = self.connection.query_row(
            "SELECT max(cursor) FROM turn WHERE session = ?1",
            params![session.as_str()],
            |r| r.get(0),
        )?;
        Ok(highest.map_or(0, |c| c.max(0) as u64 + 1))
    }

    /// Every run, newest first.
    pub fn runs(&self, limit: usize) -> Result<Vec<Run>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT r.session, r.scope, r.cwd, r.harness, r.opened, r.closed, \
                    (SELECT count(*) FROM turn t WHERE t.session = r.session) \
             FROM run r ORDER BY r.opened DESC, r.rowid DESC LIMIT ?1",
        )?;
        let found = statement
            .query_map(params![limit as i64], |r| {
                Ok(Run {
                    session: SessionId::new(r.get::<_, String>(0)?),
                    scope: r.get(1)?,
                    cwd: r.get(2)?,
                    harness: r.get(3)?,
                    opened: r.get(4)?,
                    closed: r.get(5)?,
                    turns: r.get::<_, i64>(6)?.max(0) as u64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }

    /// Say what model a run talks to, and how much it holds. Told once per run; a harness that
    /// never says this gets the shipped default.
    pub fn note_model(
        &self,
        session: &SessionId,
        model: &str,
        context: u32,
    ) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO run_model (session, model, context) VALUES (?1, ?2, ?3) \
             ON CONFLICT(session) DO UPDATE SET model = ?2, context = ?3",
            params![session.as_str(), model, context],
        )?;
        self.connection.execute(
            "UPDATE run SET model = ?2, context = ?3 WHERE session = ?1",
            params![session.as_str(), model, context],
        )?;
        Ok(())
    }

    /// What model a run talks to, and how much it holds.
    pub fn model_of(&self, session: &SessionId) -> Result<Option<(String, u32)>, StoreError> {
        let told = self
            .connection
            .query_row(
                "SELECT model, context FROM run_model WHERE session = ?1",
                params![session.as_str()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?;
        if let Some((model, context)) = told {
            return Ok(Some((model, u32::try_from(context).unwrap_or(u32::MAX))));
        }
        let found = self
            .connection
            .query_row(
                "SELECT model, context FROM run WHERE session = ?1",
                params![session.as_str()],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<i64>>(1)?)),
            )
            .optional()?;
        Ok(match found {
            Some((Some(model), Some(context))) => {
                Some((model, u32::try_from(context).unwrap_or(u32::MAX)))
            }
            _ => None,
        })
    }

    /// The turns a plan still has anything to say about, without their text.
    ///
    /// A dropped or summarised turn costs nothing to send and cannot be masked again; summaries
    /// are always taken from the front, so those are a prefix and skipping them is the sliding
    /// window. A masked turn keeps its words and the planner needs none of them, so the count
    /// comes back instead of the text — otherwise every request reads every masked turn in full.
    pub fn in_window(&self, session: &SessionId) -> Result<Vec<Turn>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT cursor, at, role, kind, \
                    CASE state WHEN 'live' THEN text ELSE '' END, \
                    tool, raw, entry, \
                    COALESCE(tokens, (LENGTH(text) + 3) / 4), \
                    state, pinned, ok, ms, args, revisions, grp, stub, handle, keep, error \
             FROM turn WHERE session = ?1 AND state IN ('live', 'masked') ORDER BY cursor",
        )?;
        let found = statement
            .query_map(params![session.as_str()], read)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }

    /// Record what a plan did to a turn. The text is untouched: compaction decides what is sent,
    /// never what was said.
    pub fn mark(&self, session: &SessionId, cursor: u64, state: State) -> Result<(), StoreError> {
        self.connection.execute(
            "UPDATE turn SET state = ?3 WHERE session = ?1 AND cursor = ?2",
            params![session.as_str(), cursor as i64, state.as_str()],
        )?;
        Ok(())
    }

    /// How many turns are held, across every run.
    pub fn census(&self) -> Result<(u64, u64), StoreError> {
        let turns: i64 = self
            .connection
            .query_row("SELECT count(*) FROM turn", [], |r| r.get(0))?;
        let runs: i64 = self
            .connection
            .query_row("SELECT count(*) FROM run", [], |r| r.get(0))?;
        Ok((runs.max(0) as u64, turns.max(0) as u64))
    }
}

/// Every column a turn is read from, in the order [`read`] takes them.
pub(crate) const COLUMNS: &str = "cursor, at, role, kind, text, tool, raw, entry, tokens, state, \
     pinned, ok, ms, args, revisions, grp, stub, handle, keep, error";

/// One turn from a row of [`COLUMNS`].
pub(crate) fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Turn> {
    Ok(Turn {
        cursor: row.get::<_, i64>(0)?.max(0) as u64,
        at: row.get(1)?,
        role: row.get(2)?,
        kind: row.get(3)?,
        text: row.get(4)?,
        tool: row.get(5)?,
        raw: row.get(6)?,
        entry: row.get(7)?,
        tokens: row.get::<_, Option<i64>>(8)?.map(|n| n.max(0) as u32),
        state: row.get::<_, String>(9)?.parse().unwrap_or_default(),
        pinned: row.get::<_, i64>(10)? != 0,
        ok: row.get(11)?,
        ms: row.get::<_, Option<i64>>(12)?.map(|n| n.max(0) as u64),
        args: row.get(13)?,
        revisions: row.get::<_, i64>(14)?.max(0) as u32,
        group: row.get::<_, Option<i64>>(15)?.map(|n| n.max(0) as u64),
        stub: row.get(16)?,
        handle: row.get(17)?,
        keep: row.get::<_, i64>(18)? != 0,
        error: row.get::<_, i64>(19)? != 0,
    })
}

/// Columns added after the first release, and how each is declared. A store written before
/// them gains them on open; one that has them is left alone.
const ADDED: &[(&str, &str)] = &[
    ("grp", "INTEGER"),
    ("stub", "TEXT"),
    ("handle", "TEXT"),
    ("keep", "INTEGER NOT NULL DEFAULT 0"),
    ("error", "INTEGER NOT NULL DEFAULT 0"),
];

/// Create what is missing and bring an older file forward, but only when the file was last
/// prepared for a different schema: an open that changes nothing takes no write lock.
fn prepare(connection: &Connection) -> Result<(), StoreError> {
    let wanted = fingerprint();
    let held: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if held == wanted {
        return Ok(());
    }
    let tx =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)?;
    let held: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if held != wanted {
        build(&tx)?;
        tx.pragma_update(None, "user_version", wanted)?;
    }
    tx.commit()?;
    Ok(())
}

/// Every table and column the scrollback holds, created where missing.
fn build(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(SCHEMA)?;
    let mut statement = connection.prepare("SELECT name FROM pragma_table_info('turn')")?;
    let held = statement
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for (name, declared) in ADDED {
        if !held.iter().any(|column| column == name) {
            connection.execute_batch(&format!("ALTER TABLE turn ADD COLUMN {name} {declared}"))?;
        }
    }
    connection.execute_batch(crate::laying::SCHEMA)?;
    connection.execute_batch(crate::jobs::SCHEMA)?;
    crate::effects::prepare(connection)?;
    connection.execute_batch(crate::extraction::SCHEMA)?;
    connection.execute_batch(crate::generation::SCHEMA)?;
    crate::notes::prepare(connection)?;
    Ok(())
}

/// A positive 31-bit digest of everything [`build`] declares, kept as the file's `user_version`.
fn fingerprint() -> i64 {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    for part in [
        SCHEMA,
        crate::laying::SCHEMA,
        crate::jobs::SCHEMA,
        crate::effects::SCHEMA,
        crate::extraction::SCHEMA,
        crate::generation::SCHEMA,
        crate::notes::SCHEMA,
    ] {
        digest.update(part.as_bytes());
    }
    for (name, declared) in ADDED.iter().chain(crate::effects::ADDED) {
        digest.update(format!("{name} {declared};").as_bytes());
    }
    for (table, name, declared) in crate::notes::ADDED {
        digest.update(format!("{table}.{name} {declared};").as_bytes());
    }
    let bytes = digest.finalize();
    i64::from(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) & 0x7fff_ffff) | 1
}

/// The scrollback, as first written. Columns added later are in [`ADDED`] as well, so a file
/// from before them is brought forward on open.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS transcript_run (
  session TEXT PRIMARY KEY,
  run     TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS run (
  session  TEXT PRIMARY KEY,
  scope    TEXT NOT NULL,
  cwd      TEXT NOT NULL,
  harness  TEXT NOT NULL,
  opened   INTEGER NOT NULL,
  closed   INTEGER,
  -- What model this run talks to, and how much it will take. balthasar does the compacting, so balthasar
  -- has to know the size of the thing it is compacting for: a plan made against 200k defaults
  -- when the model holds 8k has already overflowed, and one made against 8k when the model
  -- holds a million throws away context nobody needed to lose.
  model    TEXT,
  context  INTEGER
) STRICT;

-- A model named before its run was opened: a harness says it at startup, before any turn exists.
CREATE TABLE IF NOT EXISTS run_model (
  session  TEXT PRIMARY KEY,
  model    TEXT NOT NULL,
  context  INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS turn (
  session    TEXT NOT NULL,
  cursor     INTEGER NOT NULL,
  at         INTEGER NOT NULL,
  role       TEXT NOT NULL,
  kind       TEXT NOT NULL,
  text       TEXT NOT NULL DEFAULT '',
  tool       TEXT,
  raw        TEXT,
  revisions  INTEGER NOT NULL DEFAULT 0,
  -- Which message a block belongs to. Blocks of one assistant message share it, so a bounded
  -- read can stop on a message boundary rather than in the middle of one.
  entry      TEXT,
  -- What the harness was charged, when it says. NULL means nobody counted and a reader
  -- estimates -- never that the turn was free.
  tokens     INTEGER,
  -- What the extractors read. `ok` and `ms` are how a repair and a slow command are told
  -- apart from ordinary output; `args` is the harness's own call, kept as text and never
  -- parsed here. A harness that supplies none of them still gets a working transcript --
  -- it simply produces no SCAR.
  ok         INTEGER,
  ms         INTEGER,
  args       TEXT,
  -- What is sent for this turn, and whether a plan may touch it. Compaction changes these and
  -- never the text, which is what makes it reversible: a masked turn still has its words here.
  state      TEXT NOT NULL DEFAULT 'live',
  pinned     INTEGER NOT NULL DEFAULT 0,
  -- What the harness says about a tool row: which group it travels with, what to show in its
  -- place, how to get it back, and whether it may be elided at all.
  grp        INTEGER,
  stub       TEXT,
  handle     TEXT,
  keep       INTEGER NOT NULL DEFAULT 0,
  error      INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (session, cursor)
) STRICT;

-- Every earlier revision of a turn, kept when it is amended rather than lost to the overwrite.
CREATE TABLE IF NOT EXISTS turn_revision (
  session    TEXT NOT NULL,
  cursor     INTEGER NOT NULL,
  at         INTEGER NOT NULL,
  role       TEXT NOT NULL,
  kind       TEXT NOT NULL,
  text       TEXT NOT NULL DEFAULT '',
  tool       TEXT,
  raw        TEXT,
  entry      TEXT,
  tokens     INTEGER,
  state      TEXT NOT NULL DEFAULT 'live',
  pinned     INTEGER NOT NULL DEFAULT 0,
  ok         INTEGER,
  ms         INTEGER,
  args       TEXT,
  revisions  INTEGER NOT NULL,
  grp        INTEGER,
  stub       TEXT,
  handle     TEXT,
  keep       INTEGER NOT NULL DEFAULT 0,
  error      INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (session, cursor, revisions)
) STRICT;

-- What was noticed at a cursor and is not a turn: a permission, a retry, a compaction. Its own
-- space, so a trace sent at an entry's cursor never takes the entry's place.
CREATE TABLE IF NOT EXISTS trace (
  session  TEXT NOT NULL,
  cursor   INTEGER NOT NULL,
  seq      INTEGER NOT NULL,
  at       INTEGER NOT NULL,
  kind     TEXT NOT NULL,
  text     TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (session, cursor, seq)
) STRICT;

CREATE INDEX IF NOT EXISTS turn_of ON turn(session, cursor);
CREATE INDEX IF NOT EXISTS turn_live ON turn(session, state, cursor);
CREATE INDEX IF NOT EXISTS turn_entry ON turn(session, entry) WHERE entry IS NOT NULL;

-- What was actually said, searchable.
--
-- The transcript is the only place a claim nobody extracted still exists. Without this index it
-- is unreachable: recall queries `memory`, and a thing said once and never written down is
-- findable by nobody. A span found here is evidence and never a claim -- see `Span`.
CREATE VIRTUAL TABLE IF NOT EXISTS turn_fts USING fts5(
  text,
  session UNINDEXED,
  cursor UNINDEXED,
  tokenize = 'porter unicode61'
);
"#;

#[cfg(test)]
mod tests;
