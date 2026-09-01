//! The scrollback, kept verbatim.
//!
//! A separate file from the memory store, and separate for reasons that are not tidiness. A
//! transcript is roughly three orders of magnitude larger than the memories distilled from it;
//! it is append-mostly where the memory store is rewritten constantly; and it is pruned by age
//! where memories are pruned by decay. Sharing a file would make every recall walk past it and
//! give a retention policy nowhere to bite.
//!
//! **This is the system of record.** A harness that keeps no journal of its own has nowhere
//! else to recover from, so this store is opened with `synchronous = FULL` — every commit is on
//! the platter before it is acknowledged. That is stronger than a flushed append to a file, and
//! it is the price of being the only copy.
//!
//! **What a turn holds is opaque.** memo keeps the harness's own record as a string it never
//! parses, alongside a small projection it does understand — enough to quote a turn and to
//! search one. A harness gets back exactly what it wrote; memo never needs to know what an
//! `Entry` is, and commitment 1 survives contact with being the only place the data lives.

use crate::StoreError;
use memo_model::{SessionId, Timestamp};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

/// What has become of one turn in the window.
///
/// A turn is never removed to make room. The text stays and the state says what is sent, which
/// is what makes compaction reversible: a masked turn can be unmasked, and a summarised span
/// can be read back in full long after the summary replaced it.
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
    /// Where in the session. The harness's own numbering, never memo's.
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
    /// One assistant message carries several blocks — some prose, a thought, three tool calls —
    /// and each is written as its own turn so a span can address one of them. They share this,
    /// which is what stops a read from handing back half a message: an assistant turn shown
    /// without the tool call it made is worse than not showing it.
    ///
    /// `None` means the turn is its own message, which is what a plain user turn is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// What this cost, when the harness knows.
    ///
    /// It billed the request and memo estimated one; four characters to a token is right about
    /// the order and wrong about the number, and being wrong compounds across a long window.
    /// Kept here rather than derived so a budget is spent against what was actually charged.
    ///
    /// `None` is the ordinary case for a turn nobody counted, and callers fall back to the
    /// estimate — a missing count must never read as a free turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u32>,
    /// What is sent for this turn right now.
    #[serde(default)]
    pub state: State,
    /// Whether a plan may not touch it.
    #[serde(default)]
    pub pinned: bool,
    /// The harness's own record, verbatim.
    ///
    /// Opaque. memo stores it, hands it back, and never looks inside — which is what lets a
    /// harness treat this as its journal without memo knowing what its records mean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    /// How many times this turn has been revised.
    ///
    /// Not zero more often than you would think. A tool entry is written when the call is made
    /// and revised when the result arrives, so the same cursor is written twice and the second
    /// write is the one that matters.
    #[serde(default)]
    pub revisions: u32,
}

impl Turn {
    /// What this turn actually costs to send, given its state.
    ///
    /// A masked turn costs what its replacement costs rather than nothing: the stub still
    /// occupies room, and a planner that counted it as free would keep masking things that
    /// were already masked.
    #[must_use]
    pub fn cost(&self, masked_cost: u32) -> u32 {
        match self.state {
            State::Live => self.weight(),
            State::Masked => masked_cost.min(self.weight()),
            State::Dropped | State::Summarised => 0,
        }
    }

    /// What sending this turn in full would cost, whatever state it is in.
    ///
    /// The harness's own count when it has one, and an estimate otherwise. What a planner needs
    /// to know before deciding whether masking a turn is worth it — a masked turn's saving is
    /// measured against what it would cost live, not against what it costs now.
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

/// Where a project's scrollback lives.
///
/// Beside the memory store and named after it, so the two are obviously a pair and just as
/// obviously separate files.
#[must_use]
pub fn transcript_path(scope: &memo_model::ScopeId, tool: &crate::Tool) -> PathBuf {
    let memory = crate::scope_path(scope, tool);
    let stem = memory
        .file_stem()
        .map_or_else(|| "scope".to_owned(), |s| s.to_string_lossy().into_owned());
    memory.with_file_name(format!("{stem}-transcript.db"))
}

impl Transcript {
    /// Open the scrollback at `path`, creating it if need be.
    ///
    /// `synchronous = FULL` rather than the WAL default of `NORMAL`. NORMAL can lose the last
    /// transactions to a power cut while keeping the database consistent — which is the right
    /// trade for a cache and the wrong one for the only copy of what was said.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Io(parent.to_owned(), e))?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        // The scrollback holds whatever was pasted into it verbatim, which makes it the most
        // likely place for a secret to be sitting. A freed page SQLite has not zeroed keeps it.
        connection.pragma_update(None, "secure_delete", true)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(SCHEMA)?;
        Ok(Self {
            connection,
            path: path.to_owned(),
        })
    }

    /// A scrollback in memory, for tests.
    pub fn ephemeral() -> Result<Self, StoreError> {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(SCHEMA)?;
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

    /// Write one turn, or revise the one already at that cursor.
    ///
    /// Revising is ordinary rather than exceptional: a tool call is written when it is made and
    /// written again when its result arrives. The revision count is kept because a harness
    /// replaying a session wants the final form, and anybody debugging one wants to know the
    /// cursor was touched twice.
    ///
    /// Durable on return. A caller that has no other copy must be able to treat this answering
    /// as the turn being safe.
    pub fn write(&mut self, session: &SessionId, turn: &Turn) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT INTO turn \
             (session, cursor, at, role, kind, text, tool, raw, entry, tokens, pinned, revisions) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0) \
             ON CONFLICT(session, cursor) DO UPDATE SET \
               at = ?3, role = ?4, kind = ?5, text = ?6, tool = ?7, raw = ?8, entry = ?9, \
               tokens = ?10, pinned = ?11, \
               revisions = revisions + 1",
            params![
                session.as_str(),
                turn.cursor as i64,
                turn.at,
                turn.role,
                turn.kind,
                turn.text,
                turn.tool,
                turn.raw,
                turn.entry,
                turn.tokens,
                i64::from(turn.pinned),
            ],
        )?;
        Ok(())
    }

    /// Everything a run said, in order.
    ///
    /// What a harness restores from. Answers the turns as they finally stood, not as they were
    /// first written — a tool call comes back with its result.
    pub fn replay(&self, session: &SessionId) -> Result<Vec<Turn>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT cursor, at, role, kind, text, tool, raw, entry, tokens, state, pinned, revisions \
             FROM turn WHERE session = ?1 ORDER BY cursor",
        )?;
        let found = statement
            .query_map(params![session.as_str()], |r| Ok(read(r)))?
            .collect::<Result<Vec<_>, _>>()?;
        found.into_iter().collect()
    }

    /// One turn, for quoting.
    ///
    /// The reason `memo why` can show what a witness saw rather than naming a number at it.
    pub fn at(&self, session: &SessionId, cursor: u64) -> Result<Option<Turn>, StoreError> {
        let found = self
            .connection
            .query_row(
                "SELECT cursor, at, role, kind, text, tool, raw, entry, tokens, state, pinned, revisions \
                 FROM turn WHERE session = ?1 AND cursor = ?2",
                params![session.as_str(), cursor as i64],
                |r| Ok(read(r)),
            )
            .optional()?;
        found.transpose()
    }

    /// The cursor a resuming harness should allocate next.
    ///
    /// One past the highest written. A harness with no journal of its own has no other way to
    /// know where it was, and guessing wrong overwrites a turn.
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
             FROM run r ORDER BY r.opened DESC LIMIT ?1",
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

    /// Say what model a run talks to, and how much it holds.
    ///
    /// Told once per run rather than repeated on every plan. A harness that never says this
    /// gets the shipped default, which is a guess — and a guess about the one number that
    /// decides whether a prompt is accepted or refused.
    pub fn note_model(
        &self,
        session: &SessionId,
        model: &str,
        context: u32,
    ) -> Result<(), StoreError> {
        self.connection.execute(
            "UPDATE run SET model = ?2, context = ?3 WHERE session = ?1",
            params![session.as_str(), model, context],
        )?;
        Ok(())
    }

    /// What model a run talks to, and how much it holds.
    pub fn model_of(&self, session: &SessionId) -> Result<Option<(String, u32)>, StoreError> {
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
    /// Not the whole run. A scrollback has no upper bound — it is the only copy of the
    /// conversation and nothing prunes it — so a plan that read every turn would cost more the
    /// longer a session ran, which is exactly backwards. Two bounds, and both are needed:
    ///
    /// A dropped or summarised turn costs nothing to send and cannot be masked again. Because a
    /// summary is always taken from the front, those are always a prefix, and skipping them is
    /// the sliding window — the floor rises and never falls.
    ///
    /// A masked turn keeps its words, which is the whole point of masking being reversible, and
    /// the planner needs none of them: it has a stub already, and what the turn would have cost
    /// is a number. So the text stays in the file and the count comes back instead. Without
    /// this a window holding twenty thousand masked turns reads every byte any of them ever
    /// said, on every request.
    pub fn in_window(&self, session: &SessionId) -> Result<Vec<Turn>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT cursor, at, role, kind, \
                    CASE state WHEN 'live' THEN text ELSE '' END, \
                    tool, raw, entry, \
                    COALESCE(tokens, (LENGTH(text) + 3) / 4), \
                    state, pinned, revisions \
             FROM turn WHERE session = ?1 AND state IN ('live', 'masked') ORDER BY cursor",
        )?;
        let found = statement
            .query_map(params![session.as_str()], |r| Ok(read(r)))?
            .collect::<Result<Vec<_>, _>>()?;
        found.into_iter().collect()
    }

    /// Record what a plan did to a turn.
    ///
    /// The text is untouched. Compaction decides what is sent, never what was said, so every
    /// masked or summarised turn can still be read back in full.
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

/// One turn from a row.
fn read(row: &rusqlite::Row<'_>) -> Result<Turn, StoreError> {
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
        revisions: row.get::<_, i64>(11)?.max(0) as u32,
    })
}

/// The scrollback, as first written.
///
/// No migration machinery. While a harness is the only writer this can be rewritten freely, and
/// the day it cannot is the day it earns a `user_version` like the memory store has.
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS run (
  session  TEXT PRIMARY KEY,
  scope    TEXT NOT NULL,
  cwd      TEXT NOT NULL,
  harness  TEXT NOT NULL,
  opened   INTEGER NOT NULL,
  closed   INTEGER,
  -- What model this run talks to, and how much it will take. memo does the compacting, so memo
  -- has to know the size of the thing it is compacting for: a plan made against 200k defaults
  -- when the model holds 8k has already overflowed, and one made against 8k when the model
  -- holds a million throws away context nobody needed to lose.
  model    TEXT,
  context  INTEGER
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
  -- What is sent for this turn, and whether a plan may touch it. Compaction changes these and
  -- never the text, which is what makes it reversible: a masked turn still has its words here.
  state      TEXT NOT NULL DEFAULT 'live',
  pinned     INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (session, cursor)
) STRICT;

CREATE INDEX IF NOT EXISTS turn_of ON turn(session, cursor);
CREATE INDEX IF NOT EXISTS turn_live ON turn(session, state, cursor);
CREATE INDEX IF NOT EXISTS turn_entry ON turn(session, entry) WHERE entry IS NOT NULL;
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_occupies_one_place_in_one_run() {
        // Keyed on (session, cursor): a harness that re-sends a turn is correcting what it
        // said about it, not adding a second copy of it to the window.
        let mut held = Transcript::ephemeral().expect("a transcript");
        let session = SessionId::new("s");
        held.write(&session, &turn(7, "first")).expect("first");
        held.write(&session, &turn(7, "corrected")).expect("again");
        let back = held.replay(&session).expect("replay");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].text, "corrected");
    }

    fn turn(cursor: u64, text: &str) -> Turn {
        Turn {
            cursor,
            at: 1_756_000_000,
            role: "user".into(),
            kind: "prose".into(),
            text: text.to_owned(),
            tool: None,
            entry: None,
            tokens: None,
            state: State::default(),
            pinned: false,
            raw: Some(format!(r#"{{"type":"user","text":{text:?}}}"#)),
            revisions: 0,
        }
    }

    fn held() -> Transcript {
        Transcript::ephemeral().expect("a scrollback")
    }

    #[test]
    fn a_run_comes_back_in_the_order_it_was_said() {
        let mut t = held();
        let s = SessionId::new("s1");
        for cursor in [2, 0, 1] {
            t.write(&s, &turn(cursor, &format!("turn {cursor}")))
                .expect("write");
        }
        let cursors: Vec<u64> = t
            .replay(&s)
            .expect("replay")
            .iter()
            .map(|t| t.cursor)
            .collect();
        assert_eq!(cursors, [0, 1, 2]);
    }

    #[test]
    fn the_same_thing_said_twice_is_two_turns() {
        // The memory store deduplicates within a session, which is right for memory and fatal
        // for a transcript. Two identical turns are two turns.
        let mut t = held();
        let s = SessionId::new("s1");
        t.write(&s, &turn(0, "carry on")).expect("write");
        t.write(&s, &turn(1, "carry on")).expect("write");
        assert_eq!(t.replay(&s).expect("replay").len(), 2);
    }

    #[test]
    fn a_turn_can_be_revised_where_it_stands() {
        // Ordinary rather than exceptional: a tool call is written when it is made and again
        // when its result arrives.
        let mut t = held();
        let s = SessionId::new("s1");
        t.write(&s, &turn(0, "running")).expect("write");
        t.write(&s, &turn(0, "done")).expect("revise");

        let back = t.replay(&s).expect("replay");
        assert_eq!(back.len(), 1, "a revision is not a second turn");
        assert_eq!(back[0].text, "done");
        assert_eq!(back[0].revisions, 1, "and it is visible that it happened");
    }

    #[test]
    fn what_a_harness_wrote_comes_back_untouched() {
        // memo stores the record and never parses it. That is what lets a harness treat this
        // as its journal without memo knowing what its records mean.
        let mut t = held();
        let s = SessionId::new("s1");
        let raw = r#"{"type":"tool","id":"t1","name":"shell","result":{"output":"ok"}}"#;
        t.write(
            &s,
            &Turn {
                raw: Some(raw.to_owned()),
                ..turn(0, "")
            },
        )
        .expect("write");
        assert_eq!(t.replay(&s).expect("replay")[0].raw.as_deref(), Some(raw));
    }

    #[test]
    fn a_resuming_harness_is_told_where_it_was() {
        // With no journal of its own it has no other way to know, and guessing wrong
        // overwrites a turn.
        let mut t = held();
        let s = SessionId::new("s1");
        assert_eq!(t.next_cursor(&s).expect("next"), 0, "nothing yet");
        t.write(&s, &turn(0, "a")).expect("write");
        t.write(&s, &turn(7, "b")).expect("write");
        assert_eq!(t.next_cursor(&s).expect("next"), 8);
    }

    #[test]
    fn one_turn_can_be_fetched_for_quoting() {
        let mut t = held();
        let s = SessionId::new("s1");
        t.write(&s, &turn(3, "the deploy target is fly.io"))
            .expect("write");
        let one = t.at(&s, 3).expect("at").expect("there");
        assert_eq!(one.text, "the deploy target is fly.io");
        assert!(t.at(&s, 99).expect("at").is_none());
    }

    #[test]
    fn two_runs_keep_separate_scrollbacks() {
        let mut t = held();
        t.write(&SessionId::new("a"), &turn(0, "one"))
            .expect("write");
        t.write(&SessionId::new("b"), &turn(0, "two"))
            .expect("write");
        assert_eq!(t.replay(&SessionId::new("a")).expect("replay").len(), 1);
        assert_eq!(
            t.replay(&SessionId::new("b")).expect("replay")[0].text,
            "two"
        );
    }

    #[test]
    fn a_run_is_opened_once_however_often_it_reconnects() {
        let mut t = held();
        let s = SessionId::new("s1");
        for _ in 0..3 {
            t.open_run(&s, "/w/thing", "/w/thing", "harness", 100)
                .expect("open");
        }
        assert_eq!(t.runs(10).expect("runs").len(), 1);
    }

    #[test]
    fn a_run_says_how_much_it_holds() {
        let mut t = held();
        let s = SessionId::new("s1");
        t.open_run(&s, "/w/thing", "/w/thing", "harness", 100)
            .expect("open");
        for cursor in 0..4 {
            t.write(&s, &turn(cursor, "x")).expect("write");
        }
        assert_eq!(t.runs(10).expect("runs")[0].turns, 4);
        assert_eq!(t.census().expect("census"), (1, 4));
    }

    #[test]
    fn the_scrollback_lives_beside_the_memory_but_not_in_it() {
        // Sharing a file would make every recall walk past a transcript three orders of
        // magnitude larger than the memories distilled from it.
        let scope = memo_model::ScopeId::new("/w/thing");
        let memory = crate::scope_path(&scope, &crate::Tool::default());
        let scrollback = transcript_path(&scope, &crate::Tool::default());
        assert_ne!(memory, scrollback);
        assert_eq!(memory.parent(), scrollback.parent());
        assert!(
            scrollback.to_string_lossy().ends_with("-transcript.db"),
            "{}",
            scrollback.display()
        );
    }
}
