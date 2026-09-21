//! What a harness says about a tool row, kept and brought forward.

use balthasar_model::SessionId;
use balthasar_store::{Transcript, Turn};

fn turn(cursor: u64, text: &str) -> Turn {
    Turn {
        cursor,
        at: 1_756_000_000,
        role: "user".into(),
        kind: "prose".into(),
        text: text.to_owned(),
        ..Turn::default()
    }
}

#[test]
fn what_a_tool_said_about_its_row_comes_back() {
    let mut t = Transcript::ephemeral().expect("a scrollback");
    let s = SessionId::new("s1");
    let written = Turn {
        role: "tool".into(),
        tool: Some("read".into()),
        group: Some(4),
        stub: Some("read src/main.rs (420 lines)".into()),
        handle: Some("read src/main.rs".into()),
        keep: true,
        error: true,
        ..turn(5, "fn main() {}")
    };
    t.write(&s, &written).expect("write");
    let back = t.at(&s, 5).expect("at").expect("there");
    assert_eq!(back.group, Some(4));
    assert_eq!(back.stub, written.stub);
    assert_eq!(back.handle, written.handle);
    assert!(back.keep && back.error);
    assert_eq!(back.unit(), 4);
    assert!(back.is_tool());
}

#[test]
fn a_scrollback_written_before_the_new_columns_is_brought_forward() {
    let dir = std::env::temp_dir().join(format!("balthasar-migrate-{}", std::process::id()));
    let path = dir.join("old-transcript.db");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    {
        let old = rusqlite::Connection::open(&path).expect("open");
        old.execute_batch(
            "CREATE TABLE turn (session TEXT NOT NULL, cursor INTEGER NOT NULL, \
             at INTEGER NOT NULL, role TEXT NOT NULL, kind TEXT NOT NULL, \
             text TEXT NOT NULL DEFAULT '', tool TEXT, raw TEXT, \
             revisions INTEGER NOT NULL DEFAULT 0, entry TEXT, tokens INTEGER, ok INTEGER, \
             ms INTEGER, args TEXT, state TEXT NOT NULL DEFAULT 'live', \
             pinned INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (session, cursor)) STRICT;
             INSERT INTO turn (session, cursor, at, role, kind, text) \
             VALUES ('s1', 0, 1, 'user', 'prose', 'from before');",
        )
        .expect("an old file");
    }
    let mut t = Transcript::open(&path).expect("reopened");
    let s = SessionId::new("s1");
    let old = t.at(&s, 0).expect("at").expect("kept");
    assert_eq!(old.text, "from before");
    assert!(old.group.is_none() && !old.keep);
    t.write(
        &s,
        &Turn {
            stub: Some("new".into()),
            ..turn(1, "after")
        },
    )
    .expect("the new columns take writes");
    drop(t);
    Transcript::open(&path).expect("and opening again is a no-op");
    let _ = std::fs::remove_dir_all(&dir);
}
