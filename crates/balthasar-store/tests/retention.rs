//! Turn retention: a quiet session loses every copy of its turns, and what stays still reads and
//! searches as one.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::scratch::Scratch;
use balthasar_model::{AgentId, Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp};
use balthasar_store::{DAY, Scratchpad, Store, Transcript, Turn, mint};
use rusqlite::Connection;
use std::path::Path;

const NOW: Timestamp = 1_800_000_000;
const OLD: Timestamp = NOW - 40 * DAY;
const SECRET: &str = "the staging password is hunter2-retained-forever";
const SHARED: &str = "we deploy on fridays";
const CITED: &str = "always run the linter first";

fn said(cursor: u64, at: Timestamp, role: &str, text: &str) -> Turn {
    Turn {
        cursor,
        at,
        role: role.into(),
        kind: "prose".into(),
        text: text.into(),
        raw: Some(serde_json::json!({ "said": text }).to_string()),
        ..Turn::default()
    }
}

/// The scratch copy `observe` keeps of what a run said.
fn mirror(store: &mut Store, run: &str, text: &str) {
    let mut held = Memory::new(
        mint(OLD),
        Tier::Scratch,
        ScopeId::new("/w/p"),
        Body::note(text, NoteKind::Observation),
        OLD,
    );
    held.session = Some(SessionId::new(run));
    store.keep_scratch(held).expect("mirrored");
}

fn texts(store: &Store) -> Vec<String> {
    store.all().expect("all").iter().map(Memory::text).collect()
}

fn spans(scrollback: &Transcript, word: &str) -> Vec<(String, u64)> {
    scrollback
        .spans_matching(word, 10)
        .expect("search")
        .into_iter()
        .map(|span| (span.session.as_str().to_owned(), span.cursor))
        .collect()
}

/// Whether the secret is anywhere under a path, in any file, as raw bytes.
fn anywhere(at: &Path) -> bool {
    if at.is_file() {
        return std::fs::read(at)
            .map(|bytes| bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()))
            .unwrap_or(false);
    }
    std::fs::read_dir(at)
        .map(|entries| entries.flatten().any(|e| anywhere(&e.path())))
        .unwrap_or(false)
}

#[test]
fn a_quiet_session_loses_every_copy_of_its_turns_and_what_stays_reads_whole() {
    let dir = Scratch::new("balthasar-retention", "copies");
    let path = dir.join("transcript.db");
    let mut scrollback = Transcript::open(&path).expect("open");
    let mut pad = Scratchpad::at(dir.join("runs"));
    let mut project = Store::open(&dir.join("memory.db")).expect("store");
    let (quiet, recent, live) = (
        SessionId::new("quiet"),
        SessionId::new("recent"),
        SessionId::new("live"),
    );

    // Two sessions of one run, and a live one of its own.
    scrollback.bind_run(&quiet, Some("R")).expect("bound");
    scrollback.bind_run(&recent, Some("R")).expect("bound");
    for (cursor, role, text) in [
        (0, "user", SECRET),
        (1, "assistant", "on it"),
        (2, "user", SHARED),
        (3, "user", CITED),
    ] {
        scrollback
            .insert(&quiet, &said(cursor, OLD, role, text))
            .expect("said");
    }
    scrollback
        .write(&quiet, &said(1, OLD, "assistant", "on it, done"))
        .expect("amended");
    scrollback
        .notice(&quiet, 0, OLD, "permission", SECRET)
        .expect("trace");
    let body = serde_json::json!({ "slots": [{ "kind": "turn", "text": SECRET }] });
    let layout = scrollback
        .propose(&quiet, "m", 10, &body, &body, OLD)
        .expect("laid");
    scrollback.confirm(&layout, None, OLD).expect("applied");
    scrollback
        .insert(&recent, &said(0, OLD, "user", SHARED))
        .expect("said");
    scrollback
        .insert(&recent, &said(1, NOW, "user", "fresh"))
        .expect("said");
    scrollback
        .insert(&live, &said(0, OLD, "user", "live words"))
        .expect("said");

    let run = pad.of(&SessionId::new("R"), &AgentId::main()).expect("pad");
    for text in [SECRET, "on it", SHARED, CITED] {
        mirror(run, "R", text);
    }
    mirror(
        pad.of(&live, &AgentId::main()).expect("pad"),
        "live",
        "live words",
    );
    mirror(&mut project, "R", SECRET);

    let raw = Connection::open(&path).expect("raw");
    let evidence = serde_json::json!({ "version": 1, "kind": "user-rule",
        "sources": [{ "session": "quiet", "cursor": 3, "fingerprint": "f" }] });
    raw.execute(
        "INSERT INTO note (title, text, description, pinned, created, updated, evidence) \
         VALUES ('lint', ?1, 'lint first', 1, ?2, ?2, ?3)",
        rusqlite::params![CITED, OLD, evidence.to_string()],
    )
    .expect("a note citing turn 3");
    assert!(anywhere(&dir), "the secret is on disk to begin with");
    let before_recent = scrollback.replay(&recent).expect("replay");
    let before_live = scrollback.replay(&live).expect("replay");

    let previewed =
        balthasar_store::expired(&scrollback, NOW - 30 * DAY, std::slice::from_ref(&live))
            .expect("preview");
    assert_eq!(previewed.len(), 1);
    assert_eq!(
        (
            previewed[0].session.as_str(),
            previewed[0].turns,
            previewed[0].cited
        ),
        ("quiet", 4, 1)
    );
    assert_eq!(
        scrollback.count(&quiet).expect("count"),
        4,
        "a preview writes nothing"
    );

    let pruned = balthasar_store::prune_turns(
        &scrollback,
        Some(&mut pad),
        Some(&mut project),
        NOW - 30 * DAY,
        std::slice::from_ref(&live),
    )
    .expect("pruned");
    assert_eq!((pruned.sessions, pruned.turns, pruned.mirrors), (1, 3, 3));

    // The cited turn alone is left of the quiet session, and the others are as they were.
    let left: Vec<String> = scrollback
        .replay(&quiet)
        .expect("replay")
        .into_iter()
        .map(|t| t.text)
        .collect();
    assert_eq!(left, [CITED]);
    assert_eq!(scrollback.replay(&recent).expect("replay"), before_recent);
    assert_eq!(scrollback.replay(&live).expect("replay"), before_live);
    assert!(
        scrollback
            .revisions_of(&quiet, 1)
            .expect("revisions")
            .is_empty()
    );
    assert!(
        scrollback
            .noticed(&quiet, 0, 10)
            .expect("traces")
            .is_empty()
    );
    assert!(scrollback.proposal(&layout).expect("layout").is_none());

    // The search index holds exactly what the turns hold, before a rebuild and after one.
    assert!(spans(&scrollback, "hunter2").is_empty());
    assert_eq!(spans(&scrollback, "linter"), [("quiet".to_owned(), 3)]);
    assert_eq!(spans(&scrollback, "fridays"), [("recent".to_owned(), 0)]);
    let indexed = |raw: &Connection| -> (i64, i64, i64) {
        raw.query_row(
            "SELECT (SELECT count(*) FROM turn_fts), (SELECT count(*) FROM turn), \
                    (SELECT count(*) FROM turn_fts f WHERE NOT EXISTS (SELECT 1 FROM turn t \
                     WHERE t.session = f.session AND t.cursor = f.cursor))",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("counts")
    };
    assert_eq!(indexed(&raw), (4, 4, 0));
    assert_eq!(scrollback.reindex().expect("reindex"), 4);
    assert_eq!(indexed(&raw), (4, 4, 0));
    assert_eq!(spans(&scrollback, "linter"), [("quiet".to_owned(), 3)]);

    // A scratch copy goes with the last turn of its run that said it.
    let run = pad.of(&SessionId::new("R"), &AgentId::main()).expect("pad");
    let mut kept = texts(run);
    kept.sort();
    assert_eq!(kept, [CITED, SHARED]);
    assert_eq!(
        texts(pad.of(&live, &AgentId::main()).expect("pad")),
        ["live words"]
    );
    assert!(texts(&project).is_empty());

    drop((raw, scrollback, pad, project));
    assert!(!anywhere(&dir), "the secret is still on disk");
}

#[test]
fn a_session_that_is_live_or_spoke_this_month_keeps_everything() {
    let scrollback = Transcript::ephemeral().expect("open");
    let (live, recent) = (SessionId::new("live"), SessionId::new("recent"));
    scrollback
        .insert(&live, &said(0, OLD, "user", SECRET))
        .expect("said");
    scrollback
        .insert(&recent, &said(0, OLD, "user", SECRET))
        .expect("said");
    scrollback
        .notice(&recent, 0, NOW - DAY, "retry", "")
        .expect("noticed this month");

    let live_only = std::slice::from_ref(&live);
    let pruned = balthasar_store::prune_turns(&scrollback, None, None, NOW - 30 * DAY, live_only)
        .expect("pruned");

    assert_eq!(pruned, balthasar_store::Pruned::default());
    assert_eq!(scrollback.count(&live).expect("count"), 1);
    assert_eq!(scrollback.count(&recent).expect("count"), 1);
}
