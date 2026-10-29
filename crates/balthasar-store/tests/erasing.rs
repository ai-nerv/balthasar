//! A purge reaches every copy: the search indexes' own pages, the notes made of a run, and the
//! confidence of what the purged evidence supported.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::scratch::Scratch;
use balthasar_model::{
    Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
};
use balthasar_store::{Store, Transcript, Turn, mint};
use std::path::Path;

const NOW: Timestamp = 1_756_000_000;

fn scope() -> ScopeId {
    ScopeId::new("/w/thing")
}

fn witnessed(run: &str, n: u32) -> Witness {
    Witness::new(
        WitnessId::new(format!("w-{run}-{n}")),
        WitnessKind::Imperative,
        SessionId::new(run),
        scope(),
        NOW,
    )
}

fn fact(text: &str) -> Memory {
    Memory::new(
        mint(NOW),
        Tier::Fact,
        scope(),
        Body::note(text, NoteKind::Claim),
        NOW,
    )
}

/// Whether any page of `index` still holds `fragment`, read from the file by its own connection.
fn holds(path: &Path, index: &str, fragment: &str) -> bool {
    let db = rusqlite::Connection::open(path).expect("open");
    let mut pages = db
        .prepare(&format!("SELECT block FROM {index}_data"))
        .expect("pages");
    let blocks: Vec<Vec<u8>> = pages
        .query_map([], |r| r.get::<_, Option<Vec<u8>>>(0))
        .expect("read")
        .filter_map(Result::ok)
        .flatten()
        .collect();
    blocks.iter().any(|block| {
        block
            .windows(fragment.len())
            .any(|w| w == fragment.as_bytes())
    })
}

#[test]
fn purge_leaves_no_token_in_any_fts() {
    let dir = Scratch::new("balthasar-erase", "fts");
    let memory_path = dir.join("memory.db");
    let mut store = Store::open(&memory_path).expect("store");
    for n in 0..40 {
        store
            .remember(
                fact(&format!("filler fact number {n} about builds")),
                witnessed("keep", n),
                NOW,
            )
            .expect("filler");
    }
    let doomed = store
        .remember(
            fact("the token is qwzyxwvutsr"),
            witnessed("doomed", 0),
            NOW,
        )
        .expect("remembered")
        .id()
        .clone();
    assert!(
        holds(&memory_path, "memory_fts", "zyxwvutsr"),
        "indexed first"
    );
    balthasar_store::purge(&mut store, &doomed).expect("purge");
    assert!(
        !holds(&memory_path, "memory_fts", "zyxwvutsr"),
        "the memory index"
    );

    let turns_path = dir.join("turns.db");
    let mut held = Transcript::open(&turns_path).expect("scrollback");
    let session = SessionId::new("doomed");
    held.open_run(&session, "/w/thing", "/w/thing", "test", NOW)
        .expect("run");
    let turn = Turn {
        cursor: 1,
        at: NOW,
        role: "user".into(),
        kind: "prose".into(),
        text: "the token is qwzyxwvutsr".into(),
        ..Turn::default()
    };
    held.write(&session, &turn).expect("turn");
    assert!(holds(&turns_path, "turn_fts", "zyxwvutsr"), "indexed first");
    balthasar_store::purge_run(&held, &session).expect("purge run");
    assert!(
        !holds(&turns_path, "turn_fts", "zyxwvutsr"),
        "the turn index"
    );
}

#[test]
fn purge_session_rescores() {
    let mut store = Store::ephemeral().expect("store");
    let shared = store
        .remember(fact("we deploy with fly.io"), witnessed("kept", 0), NOW)
        .expect("remembered")
        .id()
        .clone();
    store
        .attach(&shared, witnessed("doomed", 1), NOW)
        .expect("a second run agrees");
    let confidence = |store: &Store| store.get(&shared).expect("get").expect("kept").confidence;
    let agreed = confidence(&store);

    balthasar_store::purge_session(&mut store, &SessionId::new("doomed"), NOW).expect("purge");
    let after = confidence(&store);
    assert!(
        after < agreed,
        "the purged run no longer vouches: {agreed} -> {after}"
    );
    let mut fresh = Store::ephemeral().expect("store");
    let alone = fresh
        .remember(fact("we deploy with fly.io"), witnessed("kept", 0), NOW)
        .expect("remembered")
        .id()
        .clone();
    let expected = fresh.get(&alone).expect("get").expect("held").confidence;
    assert!(
        (after - expected).abs() < 1e-9,
        "scored as if it had never been said there"
    );
}

#[test]
fn a_run_purge_takes_the_notes_made_of_it() {
    let held = Transcript::ephemeral().expect("scrollback");
    let cites = |runs: &[&str]| {
        let sources: Vec<_> = runs
            .iter()
            .map(|run| serde_json::json!({"session": run, "cursor": 1}))
            .collect();
        serde_json::json!({"title": "t", "text": "x", "description": "d",
                           "evidence": {"sources": sources}})
    };
    let alone = held
        .put_note(None, Some(&cites(&["doomed"])), NOW)
        .expect("note")
        .expect("id");
    let shared = held
        .put_note(None, Some(&cites(&["doomed", "kept"])), NOW)
        .expect("note")
        .expect("id");
    let other = held
        .put_note(None, Some(&cites(&["kept"])), NOW)
        .expect("note")
        .expect("id");

    balthasar_store::purge_run(&held, &SessionId::new("doomed")).expect("purge");

    assert!(
        held.note(&alone).expect("read").is_none(),
        "made only of the run"
    );
    let kept = held.note(&shared).expect("read").expect("still there");
    assert_eq!(
        kept.evidence["sources"],
        serde_json::json!([{"session": "kept", "cursor": 1}]),
        "without the run's citations"
    );
    assert!(held.note(&other).expect("read").is_some());
}
