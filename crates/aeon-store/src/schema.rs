//! The tables, and how a store gets from one version of them to the next.
//!
//! Migrations are numbered and forward-only, tracked in SQLite's own `user_version` so no table
//! is needed to say which tables exist. While aeon is the only reader, a migration may be
//! rewritten freely; once somebody else's store is in the field it may not.

use rusqlite::Connection;

/// Every migration, in order. The index is the version it produces.
const MIGRATIONS: &[&str] = &[V1, V2, V3, V4, V5];

/// Bring `connection` up to the current schema.
pub fn migrate(connection: &Connection) -> Result<(), rusqlite::Error> {
    let at: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (index, statements) in MIGRATIONS.iter().enumerate() {
        let version = index as u32 + 1;
        if version > at {
            connection.execute_batch(statements)?;
            connection.pragma_update(None, "user_version", version)?;
        }
    }
    Ok(())
}

/// The store, as first written.
const V1: &str = r#"
CREATE TABLE memory (
  id             TEXT PRIMARY KEY,
  tier           TEXT NOT NULL,
  scope          TEXT NOT NULL,
  session        TEXT,

  -- Lifted out of `body` because the partial unique index below keys on them, and an index
  -- cannot key on a field inside a JSON column without a generated column and a lot of care.
  subject        TEXT,
  predicate      TEXT,
  object         TEXT,

  body           TEXT NOT NULL,
  text           TEXT NOT NULL,
  content_hash   TEXT NOT NULL,

  observed_at    INTEGER NOT NULL,
  happened_at    INTEGER,
  valid_from     INTEGER NOT NULL,
  valid_to       INTEGER,

  importance     TEXT NOT NULL DEFAULT 'normal',
  strength       REAL NOT NULL DEFAULT 1.0,
  last_accessed  INTEGER NOT NULL,
  access_count   INTEGER NOT NULL DEFAULT 0,
  pinned         INTEGER NOT NULL DEFAULT 0,

  confidence     REAL NOT NULL DEFAULT 0.0,
  privacy        TEXT NOT NULL DEFAULT 'open',
  through        TEXT NOT NULL DEFAULT 'local',
  who            TEXT,
  archived_at    INTEGER,

  embedding      BLOB,
  embed_model    TEXT
) STRICT;

-- Two simultaneously-true answers to one slot are impossible, in the database rather than in
-- a policy somebody has to remember to apply. A correction that forgets to close the old
-- interval fails loudly here instead of leaving the store quietly saying both.
CREATE UNIQUE INDEX memory_slot_live ON memory(scope, subject, predicate)
  WHERE valid_to IS NULL AND tier = 'fact' AND archived_at IS NULL;

CREATE INDEX memory_recall  ON memory(scope, tier, archived_at, confidence DESC);
CREATE INDEX memory_decay   ON memory(pinned, archived_at, last_accessed);
CREATE INDEX memory_session ON memory(session, tier);
CREATE INDEX memory_hash    ON memory(scope, content_hash);

CREATE VIRTUAL TABLE memory_fts USING fts5(
  text,
  subject,
  predicate,
  object,
  id UNINDEXED,
  tokenize = 'porter unicode61'
);

-- Keyed on (memory, id) rather than on `id` alone.
--
-- The point of the key is that re-ingesting a transcript adds no evidence twice. The point it
-- must NOT make is that two memories cannot share a caller's naming scheme: with `id` alone,
-- evidence whose id happened to match something recorded for a different memory was silently
-- dropped by the `INSERT OR IGNORE`, and the loss showed up much later as a confident fact
-- with one fewer witness than it earned.
CREATE TABLE witness (
  id        TEXT NOT NULL,
  memory    TEXT NOT NULL REFERENCES memory(id),
  kind      TEXT NOT NULL,
  session   TEXT NOT NULL,
  scope     TEXT NOT NULL,
  at        INTEGER NOT NULL,
  cursor    INTEGER,
  weight    REAL NOT NULL,
  note      TEXT,
  PRIMARY KEY (memory, id)
) STRICT;

CREATE INDEX witness_of        ON witness(memory);
-- Diversity is counted, not assumed: one session cannot be its own crowd.
CREATE INDEX witness_diversity ON witness(memory, session);

CREATE TABLE link (
  src  TEXT NOT NULL REFERENCES memory(id),
  rel  TEXT NOT NULL,
  dst  TEXT NOT NULL REFERENCES memory(id),
  at   INTEGER NOT NULL,
  PRIMARY KEY (src, rel, dst)
) STRICT;

CREATE INDEX link_to ON link(dst, rel);

CREATE TABLE session (
  id       TEXT PRIMARY KEY,
  scope    TEXT NOT NULL,
  cwd      TEXT NOT NULL,
  harness  TEXT NOT NULL,
  opened   INTEGER NOT NULL,
  closed   INTEGER,
  branch   TEXT,
  parent   TEXT
) STRICT;

-- Every ingest of every source, so a re-run is idempotent and a better extractor can be run
-- over old material without redoing what has not changed.
CREATE TABLE stamp (
  source     TEXT NOT NULL,
  ref        TEXT NOT NULL,
  extractor  TEXT NOT NULL,
  version    INTEGER NOT NULL,
  at         INTEGER NOT NULL,
  PRIMARY KEY (source, ref, extractor)
) STRICT;
"#;

/// Sessions get a name a person can say, and a title saying what they were for.
///
/// A project has many sessions, and "which session did I learn that in" is a question with no
/// good answer when the only handle is a twenty-six character id. The name is short and
/// typeable; the title is whatever was asked first, which is the closest thing to a name that
/// exists without asking a model to invent one.
const V2: &str = r#"
ALTER TABLE session ADD COLUMN name TEXT;
ALTER TABLE session ADD COLUMN title TEXT;
CREATE INDEX session_named ON session(scope, name);
CREATE INDEX session_recent ON session(scope, opened DESC);
"#;

/// The ledger: what is in a session's context window, and what it costs.
///
/// Its own table rather than columns on `memory`, because the two answer different questions
/// and change at different rates. A memory is a claim that may outlive every session; a ledger
/// row is one turn's place in one window, and it is rewritten every time a plan masks or
/// summarises something. Keeping them apart is what lets the window be replanned without
/// touching a single durable record.
const V3: &str = r#"
CREATE TABLE ledger (
  session  TEXT NOT NULL,
  cursor   INTEGER NOT NULL,
  memory   TEXT REFERENCES memory(id),
  role     TEXT NOT NULL,
  kind     TEXT NOT NULL,
  tool     TEXT,
  tokens   INTEGER NOT NULL,
  state    TEXT NOT NULL DEFAULT 'live',
  pinned   INTEGER NOT NULL DEFAULT 0,
  at       INTEGER NOT NULL,
  PRIMARY KEY (session, cursor)
) STRICT;

CREATE INDEX ledger_live ON ledger(session, state, cursor);
"#;

/// What a memory is about, and how many answers a predicate may hold at once.
///
/// Two additions that arrived together because they are the same realisation: a store that
/// knows only words cannot tell `deployment` from `we deploy with fly`, and a store that
/// assumes every predicate holds one answer cannot record that somebody likes two things.
///
/// The unique index is rebuilt to key on `single_valued`. It was applied to every fact, which
/// meant `likes → sushi` and `likes → pizza` could not both be true — a constraint doing real
/// work for `deploy_target` and silently wrong for anything a person can have several of.
const V4: &str = r#"
CREATE TABLE entity (
  scope    TEXT NOT NULL,
  name     TEXT NOT NULL,
  display  TEXT NOT NULL,
  memory   TEXT NOT NULL REFERENCES memory(id),
  kind     TEXT NOT NULL,
  PRIMARY KEY (scope, name, memory)
) STRICT;

CREATE INDEX entity_name ON entity(scope, name);
CREATE INDEX entity_of   ON entity(memory);

ALTER TABLE memory ADD COLUMN single_valued INTEGER NOT NULL DEFAULT 1;

DROP INDEX memory_slot_live;
CREATE UNIQUE INDEX memory_slot_live ON memory(scope, subject, predicate)
  WHERE valid_to IS NULL AND tier = 'fact' AND archived_at IS NULL AND single_valued = 1;
"#;

/// A claim's opening words, so a revision of it can be found without scanning.
///
/// Most of what people say carries no slot: "we deploy to heroku" is prose. Without this,
/// aeon asserted that and "we deploy to fly.io" at the same time, a month apart, with nothing
/// to tell them apart.
const V5: &str = r#"
ALTER TABLE memory ADD COLUMN lead TEXT;
CREATE INDEX memory_lead ON memory(scope, tier, lead) WHERE valid_to IS NULL;
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn migrated() -> Connection {
        let connection = Connection::open_in_memory().expect("open");
        migrate(&connection).expect("migrate");
        connection
    }

    #[test]
    fn a_predicate_that_holds_many_answers_may_hold_many() {
        // `likes sushi` and `likes pizza` are both true. The unique index applied to every
        // fact made that impossible — a constraint doing real work for `deploy_target` and
        // silently wrong for anything a person can have several of.
        let connection = migrated();
        let insert = "INSERT INTO memory \
             (id, tier, scope, subject, predicate, object, body, text, content_hash, \
              observed_at, valid_from, last_accessed, single_valued) \
             VALUES (?, 'fact', 'global', 'you', 'likes', ?, '{}', '', '', 0, 0, 0, 0)";
        connection
            .execute(insert, rusqlite::params!["a", "sushi"])
            .expect("first");
        connection
            .execute(insert, rusqlite::params!["b", "pizza"])
            .expect("second");
        let held: i64 = connection
            .query_row(
                "SELECT count(*) FROM memory WHERE predicate = 'likes'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(held, 2);
    }

    #[test]
    fn a_predicate_that_holds_one_answer_still_holds_one() {
        let connection = migrated();
        let insert = "INSERT INTO memory \
             (id, tier, scope, subject, predicate, object, body, text, content_hash, \
              observed_at, valid_from, last_accessed, single_valued) \
             VALUES (?, 'fact', 'global', 'project', 'deploy_target', ?, '{}', '', '', 0, 0, 0, 1)";
        connection
            .execute(insert, rusqlite::params!["a", "heroku"])
            .expect("first");
        assert!(
            connection
                .execute(insert, rusqlite::params!["b", "fly.io"])
                .is_err(),
            "the store must still refuse to say both"
        );
    }

    #[test]
    fn a_turn_occupies_one_place_in_one_window() {
        // Keyed on (session, cursor): a harness that re-sends a turn is correcting what it
        // said about it, not adding a second copy of it to the window.
        let connection = migrated();
        let insert = "INSERT OR REPLACE INTO ledger \
             (session, cursor, role, kind, tokens, at) VALUES ('s', 7, 'tool', ?, 100, 0)";
        connection
            .execute(insert, rusqlite::params!["tool_result"])
            .expect("first");
        connection
            .execute(insert, rusqlite::params!["prose"])
            .expect("again");
        let rows: i64 = connection
            .query_row("SELECT count(*) FROM ledger", [], |r| r.get(0))
            .expect("count");
        assert_eq!(rows, 1);
    }

    #[test]
    fn a_session_can_be_named_and_titled() {
        let connection = migrated();
        connection
            .execute(
                "INSERT INTO session (id, scope, cwd, harness, opened, name, title) \
                 VALUES ('01H', 'p', '/w', 'cli', 0, '0831-k3fa', 'run the tests')",
                [],
            )
            .expect("a named session");
        let name: String = connection
            .query_row("SELECT name FROM session WHERE id = '01H'", [], |r| {
                r.get(0)
            })
            .expect("read it back");
        assert_eq!(name, "0831-k3fa");
    }

    #[test]
    fn migrating_twice_changes_nothing() {
        let connection = migrated();
        migrate(&connection).expect("a second pass must be a no-op");
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("version");
        assert_eq!(version, MIGRATIONS.len() as u32);
    }

    #[test]
    fn one_slot_cannot_hold_two_live_facts() {
        // The constraint that turns contradiction handling from a policy into a failure.
        let connection = migrated();
        let insert = "INSERT INTO memory \
             (id, tier, scope, subject, predicate, object, body, text, content_hash, \
              observed_at, valid_from, last_accessed) \
             VALUES (?, 'fact', 'global', 'project', 'test_command', ?, '{}', '', '', 0, 0, 0)";
        connection
            .execute(insert, rusqlite::params!["a", "make test"])
            .expect("the first answer");
        let second = connection.execute(insert, rusqlite::params!["b", "cargo test"]);
        assert!(second.is_err(), "the store must refuse to say both");
    }

    #[test]
    fn a_closed_interval_frees_the_slot() {
        // A correction closes the old interval and the new answer fits. If this failed,
        // every correction would need the old row deleted, and nothing here deletes.
        let connection = migrated();
        connection
            .execute(
                "INSERT INTO memory \
                 (id, tier, scope, subject, predicate, object, body, text, content_hash, \
                  observed_at, valid_from, valid_to, last_accessed) \
                 VALUES ('a', 'fact', 'global', 'project', 'test_command', 'cargo test', \
                         '{}', '', '', 0, 0, 100, 0)",
                [],
            )
            .expect("the superseded answer");
        connection
            .execute(
                "INSERT INTO memory \
                 (id, tier, scope, subject, predicate, object, body, text, content_hash, \
                  observed_at, valid_from, last_accessed) \
                 VALUES ('b', 'fact', 'global', 'project', 'test_command', 'make test', \
                         '{}', '', '', 100, 100, 100)",
                [],
            )
            .expect("the current answer must fit");
    }

    #[test]
    fn one_witness_id_may_be_evidence_for_two_memories() {
        // With `id` alone as the key, the second insert was silently ignored and a fact
        // quietly lost a witness it had earned. Silence is what makes that kind of bug
        // expensive, so it gets a test of its own.
        let connection = migrated();
        for id in ["a", "b"] {
            connection
                .execute(
                    "INSERT INTO memory (id, tier, scope, body, text, content_hash, \
                     observed_at, valid_from, last_accessed) \
                     VALUES (?, 'episode', 'global', '{}', '', '', 0, 0, 0)",
                    rusqlite::params![id],
                )
                .expect("memory");
            connection
                .execute(
                    "INSERT OR IGNORE INTO witness (id, memory, kind, session, scope, at, weight) \
                     VALUES ('shared', ?, 'manual', 's', 'global', 0, 0.4)",
                    rusqlite::params![id],
                )
                .expect("witness");
        }
        let kept: i64 = connection
            .query_row("SELECT count(*) FROM witness", [], |r| r.get(0))
            .expect("count");
        assert_eq!(
            kept, 2,
            "evidence for a different memory is different evidence"
        );
    }

    #[test]
    fn the_same_witness_for_the_same_memory_lands_once() {
        // The idempotency the key exists for: re-ingesting a transcript adds nothing.
        let connection = migrated();
        connection
            .execute(
                "INSERT INTO memory (id, tier, scope, body, text, content_hash, \
                 observed_at, valid_from, last_accessed) \
                 VALUES ('a', 'episode', 'global', '{}', '', '', 0, 0, 0)",
                [],
            )
            .expect("memory");
        for _ in 0..2 {
            connection
                .execute(
                    "INSERT OR IGNORE INTO witness (id, memory, kind, session, scope, at, weight) \
                     VALUES ('w', 'a', 'manual', 's', 'global', 0, 0.4)",
                    [],
                )
                .expect("witness");
        }
        let kept: i64 = connection
            .query_row("SELECT count(*) FROM witness", [], |r| r.get(0))
            .expect("count");
        assert_eq!(kept, 1);
    }

    #[test]
    fn a_witness_cannot_outlive_the_memory_it_is_evidence_for() {
        let connection = migrated();
        connection
            .pragma_update(None, "foreign_keys", true)
            .expect("pragma");
        let orphan = connection.execute(
            "INSERT INTO witness (id, memory, kind, session, scope, at, weight) \
             VALUES ('w', 'nothing', 'manual', 's', 'global', 0, 0.4)",
            [],
        );
        assert!(orphan.is_err(), "evidence for nothing is not evidence");
    }
}
