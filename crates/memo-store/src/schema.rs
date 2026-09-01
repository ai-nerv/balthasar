//! The tables.
//!
//! Numbered and forward-only, tracked in SQLite's own `user_version` so no table is needed to
//! say which tables exist. There is one so far, and it is the shape the store settled into
//! rather than the sequence of edits that got there: nothing else has ever read one of these
//! files, so the sequence was archaeology. The next change appends a migration; this one does
//! not have to remember a past nobody lived through.

use rusqlite::Connection;

/// Every migration, in order. The index is the version it produces.
const MIGRATIONS: &[&str] = &[V1, V2, V3, V4, V5];

/// What schema a store this build writes is at.
///
/// Printed in evaluation artifacts, so a number from a benchmark says which store shape
/// produced it. A result whose schema is unknown cannot be compared against a later one.
pub const VERSION: u32 = MIGRATIONS.len() as u32;

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

/// The store.
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

  -- A claim's opening words, so a revision of it can be found without scanning. Most of what
  -- people say carries no slot: "we deploy to heroku" is prose. Without this, memo asserted
  -- that and "we deploy to fly.io" at the same time, a month apart, with nothing to tell them
  -- apart.
  lead           TEXT,

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

  -- Whether this predicate holds one answer or several. A store that assumes every predicate
  -- holds one cannot record that somebody likes two things: `likes -> sushi` and
  -- `likes -> pizza` would contradict each other, while `deploy_target` genuinely may not.
  single_valued  INTEGER NOT NULL DEFAULT 1,

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
--
-- Keyed on `single_valued` as well, so the constraint does real work for `deploy_target` and
-- stays out of the way of anything a person can have several of.
CREATE UNIQUE INDEX memory_slot_live ON memory(scope, subject, predicate)
  WHERE valid_to IS NULL AND tier = 'fact' AND archived_at IS NULL AND single_valued = 1;

CREATE INDEX memory_recall  ON memory(scope, tier, archived_at, confidence DESC);
CREATE INDEX memory_decay   ON memory(pinned, archived_at, last_accessed);
CREATE INDEX memory_session ON memory(session, tier);
CREATE INDEX memory_hash    ON memory(scope, content_hash);
CREATE INDEX memory_lead    ON memory(scope, tier, lead) WHERE valid_to IS NULL;

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

-- `name` and `title` because "which session did I learn that in" has no good answer when the
-- only handle is a twenty-six character id. The name is short and typeable; the title is
-- whatever was asked first, which is the closest thing to a name that exists without asking a
-- model to invent one.
CREATE TABLE session (
  id       TEXT PRIMARY KEY,
  scope    TEXT NOT NULL,
  cwd      TEXT NOT NULL,
  harness  TEXT NOT NULL,
  opened   INTEGER NOT NULL,
  closed   INTEGER,
  branch   TEXT,
  parent   TEXT,
  name     TEXT,
  title    TEXT
) STRICT;

CREATE INDEX session_named  ON session(scope, name);
CREATE INDEX session_recent ON session(scope, opened DESC);

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

-- What a memory is about. A store that knows only words cannot tell `deployment` from
-- `we deploy with fly`.
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
"#;

/// The use-and-outcome ledger: what was retrieved, what was injected, and how it went.
///
/// Access count is not utility. Recalling a poisoned memory ten times should not strengthen it
/// ten times merely because it was retrieved ten times — so retrieval, injection, use and
/// outcome are four separate records rather than one counter, and only an attributed outcome is
/// evidence about a memory's worth.
///
/// **Nothing here touches truth.** No row in these tables is a witness, and none of them can
/// change a confidence. That separation is the point: a fact may be true and harmful to inject,
/// and a system that folded the two together could express neither.
///
/// **Content stays out.** Queries are hashed, actions are hashed, and transcript spans are
/// referenced by cursor. The ledger records that something happened and where to look, never a
/// copy of what was said.
///
/// Columns for signals that do not exist yet — temporal, causal and trust — are deliberately
/// absent. They arrive with the milestone that computes them, because a column that is always
/// zero teaches a reader that the signal is worthless rather than that it is unimplemented.
const V2: &str = r#"
CREATE TABLE recall_run (
  id                  TEXT PRIMARY KEY,
  scope               TEXT NOT NULL,
  session             TEXT,
  query_hash          TEXT NOT NULL,
  requested_at        INTEGER NOT NULL,
  config_fingerprint  TEXT NOT NULL,
  vector_available    INTEGER NOT NULL DEFAULT 0,
  result_limit        INTEGER NOT NULL,
  latency_us          INTEGER NOT NULL
) STRICT;

CREATE INDEX recall_run_when ON recall_run(scope, requested_at DESC);

-- One row per memory the search considered, whether or not it was returned. `selected` is what
-- separates "this was in the candidate set" from "this was an answer", and without both there
-- is no way to ask why something was missed.
CREATE TABLE recall_candidate (
  recall_id    TEXT NOT NULL REFERENCES recall_run(id),
  memory_id    TEXT NOT NULL,
  rank         INTEGER NOT NULL,
  selected     INTEGER NOT NULL DEFAULT 0,
  score        REAL NOT NULL,
  semantic     REAL NOT NULL DEFAULT 0.0,
  lexical      REAL NOT NULL DEFAULT 0.0,
  entity       REAL NOT NULL DEFAULT 0.0,
  frecency     REAL NOT NULL DEFAULT 0.0,
  confidence   REAL NOT NULL DEFAULT 0.0,
  strength     REAL NOT NULL DEFAULT 0.0,
  scope_signal REAL NOT NULL DEFAULT 0.0,
  PRIMARY KEY (recall_id, memory_id)
) STRICT;

CREATE INDEX recall_candidate_of ON recall_candidate(memory_id, selected);

-- An assembled context handed to a caller. A recall may produce none.
CREATE TABLE injection (
  id           TEXT PRIMARY KEY,
  recall_id    TEXT REFERENCES recall_run(id),
  session      TEXT,
  created_at   INTEGER NOT NULL,
  token_count  INTEGER NOT NULL,
  remote       INTEGER NOT NULL DEFAULT 0,
  policy_name  TEXT NOT NULL DEFAULT 'balanced'
) STRICT;

CREATE INDEX injection_when ON injection(session, created_at DESC);

CREATE TABLE injection_memory (
  injection_id       TEXT NOT NULL REFERENCES injection(id),
  memory_id          TEXT NOT NULL,
  position           INTEGER NOT NULL,
  presentation_mode  TEXT NOT NULL DEFAULT 'asserted',
  PRIMARY KEY (injection_id, memory_id)
) STRICT;

CREATE INDEX injection_memory_of ON injection_memory(memory_id);

-- One action a caller reported. `action_hash` rather than the arguments: the ledger needs to
-- tell two actions apart, not to reproduce them.
CREATE TABLE action_use (
  id                TEXT PRIMARY KEY,
  injection_id      TEXT REFERENCES injection(id),
  session           TEXT,
  reported_at       INTEGER NOT NULL,
  tool              TEXT,
  action_hash       TEXT NOT NULL,
  attribution_kind  TEXT NOT NULL DEFAULT 'proximal'
) STRICT;

CREATE INDEX action_use_when ON action_use(session, reported_at DESC);

-- Which memories an action actually used. A separate table rather than a list column, so that
-- "what did this memory get used for" is an index lookup rather than a scan and a parse.
CREATE TABLE action_memory (
  action_id  TEXT NOT NULL REFERENCES action_use(id),
  memory_id  TEXT NOT NULL,
  PRIMARY KEY (action_id, memory_id)
) STRICT;

CREATE INDEX action_memory_of ON action_memory(memory_id);

-- How it went. `evaluator` names who said so, because a model's opinion and a compiler's exit
-- status are not the same kind of evidence and must stay distinguishable.
CREATE TABLE outcome (
  id              TEXT PRIMARY KEY,
  action_id       TEXT NOT NULL REFERENCES action_use(id),
  observed_at     INTEGER NOT NULL,
  kind            TEXT NOT NULL,
  score           REAL,
  evidence_cursor INTEGER,
  evaluator       TEXT NOT NULL DEFAULT 'caller',
  note            TEXT
) STRICT;

CREATE INDEX outcome_of ON outcome(action_id);
"#;

/// Where one piece of work ended and the next began.
///
/// Derived and rebuildable: the transcript is the authority, and these rows are one version of
/// the rules' opinion about it. `derivation` is what makes a change to those rules safe to roll
/// out — a new version can be computed beside the old one and compared before anything switches
/// over, rather than silently replacing a segmentation whose results are already being used.
///
/// No foreign key to `memory`. An episode is a span of transcript, and a memory distilled from
/// that span points at cursors rather than the other way round; a segment that could not be
/// rebuilt without first deleting memories would not be rebuildable.
const V3: &str = r#"
CREATE TABLE episode_segment (
  id              TEXT PRIMARY KEY,
  session         TEXT NOT NULL,
  start_cursor    INTEGER NOT NULL,
  end_cursor      INTEGER NOT NULL,
  started_at      INTEGER NOT NULL,
  ended_at        INTEGER NOT NULL,
  boundary_before TEXT NOT NULL,
  reason_before   TEXT NOT NULL,
  boundary_after  TEXT,
  reason_after    TEXT,
  method          TEXT NOT NULL,
  derivation      INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX episode_span ON episode_segment(session, derivation, start_cursor);
CREATE INDEX episode_of ON episode_segment(session, start_cursor);
"#;

/// Derived relationships, kept apart from the asserted ones in `link`.
///
/// Two tables rather than a `source` column on one, because the two have opposite lifecycles.
/// An asserted link is part of what a memory means and is never rebuilt; a derived edge is an
/// index over things the store already holds, and the ability to drop every one and recompute
/// is what makes changing a derivation something that can be tried.
///
/// `stale_at` rather than a delete: an edge whose derivation has been superseded stops being
/// traversed and stays readable, so a comparison between two versions has both to look at.
///
/// No weight floor in the schema. What is worth traversing is a policy question that changes
/// with the query, and baking a threshold into the table would decide it once for everything.
const V4: &str = r#"
CREATE TABLE relation_view (
  from_memory        TEXT NOT NULL,
  to_memory          TEXT NOT NULL,
  kind               TEXT NOT NULL,
  weight             REAL NOT NULL DEFAULT 1.0,
  source             TEXT NOT NULL,
  derivation_version INTEGER NOT NULL DEFAULT 1,
  evidence_cursor    INTEGER,
  created_at         INTEGER NOT NULL,
  stale_at           INTEGER,
  PRIMARY KEY (from_memory, to_memory, kind, derivation_version)
) STRICT;

-- Traversal reads outward from a memory, filtered by kind. Live edges only, which is what the
-- partial index is for: a store holding three derivation versions should not pay for the two
-- nobody is traversing.
CREATE INDEX relation_out ON relation_view(from_memory, kind) WHERE stale_at IS NULL;
CREATE INDEX relation_in  ON relation_view(to_memory, kind)   WHERE stale_at IS NULL;
CREATE INDEX relation_gen ON relation_view(source, derivation_version);
"#;

/// Where a witness's content came from, and how it arrived.
///
/// The process boundary and the information source are different questions, and conflating them
/// is how a persistent store turns untrusted text into durable instruction: a trusted local peer
/// can submit a web page it just fetched.
///
/// `domain` is the load-bearing one. It is what makes ten sessions quoting one document count
/// as one source rather than ten confirmations — the attack that witness diversity by session
/// alone cannot see, because all ten arrivals really are distinct runs. `NULL` means the session
/// is the domain, which is what every witness written before this meant and still means.
const V5: &str = r#"
ALTER TABLE witness ADD COLUMN channel TEXT NOT NULL DEFAULT 'peer-assertion';
ALTER TABLE witness ADD COLUMN domain TEXT;

-- "which memories came from this source" has to be answerable in one read, because it is the
-- question a purge of a poisoned origin starts from.
CREATE INDEX witness_domain ON witness(domain) WHERE domain IS NOT NULL;
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
