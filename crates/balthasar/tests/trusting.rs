//! `balthasar trust` names the channel each new memory's evidence arrived on: against the real
//! binary, for a run that was distilled and for a memory written by hand.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::SessionId;
use balthasar_model::scratch::Scratch;
use std::path::Path;
use std::process::Command;

const NOW: balthasar_model::Timestamp = 1_756_000_000;

fn said(store: &Path, args: &[&str]) -> serde_json::Value {
    let mut all = vec!["--store".to_owned(), store.to_string_lossy().into_owned()];
    all.extend(args.iter().map(|a| (*a).to_owned()));
    let ran = Command::new(env!("CARGO_BIN_EXE_balthasar"))
        .args(&all)
        .output()
        .expect("balthasar");
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    serde_json::from_slice(&ran.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&ran.stdout)))
}

/// The channels `trust` names for the memory a search for `query` finds holding `holding`.
fn channels(store: &Path, query: &str, holding: &str) -> Vec<String> {
    let found = said(store, &["recall", query, "--json"]);
    let id = found["result"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|row| {
            row["body"]["text"]
                .as_str()
                .is_some_and(|t| t.contains(holding))
        })
        .and_then(|row| row["id"].as_str())
        .unwrap_or_else(|| panic!("nothing holds {holding}: {found}"))
        .to_owned();
    said(store, &["trust", &id, "--json"])["result"][0]["channels"]
        .as_array()
        .expect("channels")
        .iter()
        .filter_map(|c| c.as_str().map(str::to_owned))
        .collect()
}

#[test]
fn a_distilled_instruction_names_the_person_who_gave_it() {
    let at = Scratch::new("balthasar-trust", "distilled");
    let store = at.join("store.db");
    let session = SessionId::new("01TRUST");
    let mut held =
        balthasar_store::Transcript::open(&at.join("store-transcript.db")).expect("transcript");
    held.open_run(&session, "/w/thing", "/w/thing", "test", NOW)
        .expect("run");
    let turn = balthasar_store::Turn {
        cursor: 1,
        at: NOW,
        role: "user".into(),
        kind: "prose".into(),
        text: "remember: we deploy with fly.io".into(),
        ..balthasar_store::Turn::default()
    };
    held.write(&session, &turn).expect("turn");
    drop(held);
    said(&store, &["distil", "--now", "--json"]);
    assert_eq!(channels(&store, "deploy", "fly.io"), ["user-instruction"]);
}

#[test]
fn a_memory_written_by_hand_says_so() {
    let at = Scratch::new("balthasar-trust", "manual");
    let store = at.join("store.db");
    said(
        &store,
        &["remember", "the staging box is at 10.0.0.7", "--json"],
    );
    assert_eq!(
        channels(&store, "staging box", "10.0.0.7"),
        ["manual-write"]
    );
}
