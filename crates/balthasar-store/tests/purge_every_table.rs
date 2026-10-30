//! A purge reaches every table that names a session, in both files, and no other session's rows.
//!
//! The tables are read out of the schema rather than listed here, so a table added later with a
//! session column and no purge fails this test instead of quietly keeping what was said.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::SessionId;
use balthasar_model::scratch::Scratch;
use balthasar_store::{Store, Transcript};
use rusqlite::Connection;

/// Every table with a column naming a session, and that column. `session` itself is keyed `id`.
fn keyed(db: &Connection) -> Vec<(String, String)> {
    let mut tables = db
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
             AND name NOT LIKE '%fts_%'",
        )
        .expect("tables");
    let names: Vec<String> = tables
        .query_map([], |r| r.get(0))
        .expect("rows")
        .collect::<Result<_, _>>()
        .expect("names");
    names
        .into_iter()
        .filter_map(|table| {
            let columns = columns(db, &table);
            if table == "session" {
                return Some((table, "id".to_owned()));
            }
            columns
                .iter()
                .any(|c| c.0 == "session")
                .then(|| (table, "session".to_owned()))
        })
        .collect()
}

/// `(name, declared type, not null, has default)` for each column.
fn columns(db: &Connection, table: &str) -> Vec<(String, String, bool, bool)> {
    let mut info = db
        .prepare(&format!(
            "SELECT name, type, \"notnull\", dflt_value FROM pragma_table_info('{table}')"
        ))
        .expect("info");
    info.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)? != 0,
            r.get::<_, Option<String>>(3)?.is_some(),
        ))
    })
    .expect("rows")
    .collect::<Result<_, _>>()
    .expect("columns")
}

/// One row in every keyed table for `session`, every other required column a placeholder.
fn fill(db: &Connection, session: &str, n: &mut i64) {
    for (table, key) in keyed(db) {
        let mut names = Vec::new();
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        for (name, kind, required, defaulted) in columns(db, &table) {
            if name == key {
                names.push(name);
                values.push(rusqlite::types::Value::Text(session.to_owned()));
            } else if required && !defaulted {
                *n += 1;
                names.push(name);
                values.push(match kind.to_uppercase().as_str() {
                    "INTEGER" => rusqlite::types::Value::Integer(*n),
                    "REAL" => rusqlite::types::Value::Real(0.5),
                    _ => rusqlite::types::Value::Text(format!("x{n}")),
                });
            }
        }
        let marks = vec!["?"; names.len()].join(", ");
        db.execute(
            &format!(
                "INSERT INTO {table} ({}) VALUES ({marks})",
                names.join(", ")
            ),
            rusqlite::params_from_iter(values),
        )
        .unwrap_or_else(|why| panic!("{table}: {why}"));
    }
}

fn left(db: &Connection, session: &str) -> Vec<String> {
    keyed(db)
        .into_iter()
        .filter(|(table, key)| {
            let held: i64 = db
                .query_row(
                    &format!("SELECT count(*) FROM {table} WHERE {key} = ?1"),
                    [session],
                    |r| r.get(0),
                )
                .expect("count");
            held > 0
        })
        .map(|(table, _)| table)
        .collect()
}

#[test]
fn purging_a_run_empties_every_session_keyed_table_in_the_scrollback() {
    let dir = Scratch::new("balthasar-purge-every", "scrollback");
    let path = dir.join("transcript.db");
    let scrollback = Transcript::open(&path).expect("open");
    let raw = Connection::open(&path).expect("raw");
    raw.pragma_update(None, "foreign_keys", false)
        .expect("placeholders point at nothing");
    let mut n = 0;
    fill(&raw, "gone", &mut n);
    fill(&raw, "kept", &mut n);
    raw.execute(
        "INSERT INTO turn_fts (text, session, cursor) VALUES ('secret', 'gone', 0)",
        [],
    )
    .expect("fts");

    balthasar_store::purge_run(&scrollback, &SessionId::new("gone")).expect("purge");

    assert_eq!(
        left(&raw, "gone"),
        Vec::<String>::new(),
        "rows survived the purge"
    );
    let kept = keyed(&raw).len();
    assert_eq!(
        kept - left(&raw, "kept").len(),
        0,
        "another run's rows went too"
    );
    let fts: i64 = raw
        .query_row(
            "SELECT count(*) FROM turn_fts WHERE session = 'gone'",
            [],
            |r| r.get(0),
        )
        .expect("fts");
    assert_eq!(fts, 0);
}

#[test]
fn purging_a_session_empties_every_session_keyed_table_in_the_memory_store() {
    let dir = Scratch::new("balthasar-purge-every", "memory");
    let path = dir.join("memory.db");
    let mut store = Store::open(&path).expect("open");
    let raw = Connection::open(&path).expect("raw");
    raw.pragma_update(None, "foreign_keys", false)
        .expect("placeholders point at nothing");
    let mut n = 0;
    fill(&raw, "gone", &mut n);
    fill(&raw, "kept", &mut n);

    balthasar_store::purge_session(&mut store, &SessionId::new("gone"), 0).expect("purge");

    assert_eq!(
        left(&raw, "gone"),
        Vec::<String>::new(),
        "rows survived the purge"
    );
    assert_eq!(
        keyed(&raw).len(),
        left(&raw, "kept").len(),
        "another run's rows went too"
    );
}

#[test]
fn applied_layouts_are_pruned_by_age_but_not_the_newest_or_one_still_owed_effects() {
    let scrollback = Transcript::ephemeral().expect("open");
    let session = SessionId::new("s");
    let body = serde_json::json!({ "slots": [] });
    let laid = |at| {
        scrollback
            .propose(&session, "m", 10, &body, &body, at)
            .expect("proposed")
    };
    let (old, owed, unsent, newest) = (laid(100), laid(200), laid(300), laid(400));
    for id in [&old, &owed, &newest] {
        assert!(scrollback.confirm(id, None, 500).expect("applied"));
    }
    scrollback
        .queue_layout_effect(&balthasar_store::LayoutEffect {
            layout: owed.clone(),
            session: session.clone(),
            run: session.clone(),
            agent: balthasar_model::AgentId::main(),
            scope: balthasar_model::ScopeId::new("/w"),
            cursor: 0,
            text: "owed".to_owned(),
            at: 500,
        })
        .expect("queued");

    let gone = balthasar_store::prune_layouts(&scrollback, 1_000).expect("pruned");
    assert_eq!(gone, 1);
    assert!(
        scrollback.proposal(&old).expect("read").is_none(),
        "old and sent"
    );
    assert!(
        scrollback.proposal(&owed).expect("read").is_some(),
        "its effect is still owed"
    );
    assert!(
        scrollback.proposal(&unsent).expect("read").is_some(),
        "never applied"
    );
    assert!(
        scrollback.proposal(&newest).expect("read").is_some(),
        "the run's newest"
    );
}

#[test]
fn a_retention_pass_empties_every_table_of_a_quiet_session_and_nothing_else() {
    const NOW: i64 = 1_800_000_000;
    let dir = Scratch::new("balthasar-purge-every", "retained");
    let path = dir.join("transcript.db");
    let scrollback = Transcript::open(&path).expect("open");
    let raw = Connection::open(&path).expect("raw");
    raw.pragma_update(None, "foreign_keys", false)
        .expect("placeholders point at nothing");
    let mut n = 0;
    for session in ["quiet", "live", "recent"] {
        fill(&raw, session, &mut n);
    }
    raw.execute(
        "INSERT INTO turn (session, cursor, at, role, kind) \
         VALUES ('recent', 9999, ?1, 'user', 'prose')",
        [NOW],
    )
    .expect("a turn this month");
    let every = keyed(&raw).len();

    let live = [SessionId::new("live")];
    let before = NOW - 30 * balthasar_store::DAY;
    let pruned =
        balthasar_store::prune_turns(&scrollback, None, None, before, &live).expect("pruned");

    assert_eq!(pruned.sessions, 1);
    assert_eq!(
        left(&raw, "quiet"),
        vec!["note_change".to_owned()],
        "only the notes' own change log outlives the session"
    );
    assert_eq!(left(&raw, "live").len(), every, "the live session");
    assert_eq!(
        left(&raw, "recent").len(),
        every,
        "a session active this month"
    );
}
