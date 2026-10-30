//! `balthasar forget --session` against the real binary, over the real file layout.
//!
//! The store-level test proves a purge closes all three of a run's hiding places. This proves
//! the command finds all three — which is a different thing, and the half that breaks when a
//! path is computed twice in two places.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::scratch::Scratch;

use balthasar_model::{Body, Memory, NoteKind, ScopeId, SessionId, Tier};
use balthasar_store::mint;
use std::path::{Path, PathBuf};
use std::process::Command;

const NOW: balthasar_model::Timestamp = 1_756_000_000;
const SECRET: &str = "the deploy token MAGI_BALTHASAR_DEPLOY_TOKEN is hunter2-nevershare";

/// The three files `--store` implies, laid out the way the CLI computes them.
fn planted(at: &Path, session: &SessionId) -> (PathBuf, PathBuf, PathBuf) {
    let memory = at.join("store.db");
    let transcript = at.join("store-transcript.db");
    let runs = at.join("store.runs");
    let scope = ScopeId::new("/w/thing");

    let mut store = balthasar_store::Store::open(&memory).expect("store");
    store
        .open_session(session, &scope, "/w/thing", "test", NOW)
        .expect("session");
    let mut owned = Memory::new(
        mint(NOW),
        Tier::Scratch,
        scope.clone(),
        Body::note(SECRET, NoteKind::Observation),
        NOW,
    );
    owned.session = Some(session.clone());
    store.keep_scratch(owned).expect("keep");

    let mut held = balthasar_store::Transcript::open(&transcript).expect("transcript");
    held.open_run(session, "/w/thing", "/w/thing", "test", NOW)
        .expect("run");
    held.write(
        session,
        &balthasar_store::Turn {
            cursor: 1,
            at: NOW,
            role: "user".into(),
            kind: "prose".into(),
            text: SECRET.to_owned(),
            ..balthasar_store::Turn::default()
        },
    )
    .expect("turn");

    let mut pad = balthasar_store::Scratchpad::at(&runs);
    let mut scratch = Memory::new(
        mint(NOW),
        Tier::Scratch,
        scope,
        Body::note(SECRET, NoteKind::Observation),
        NOW,
    );
    scratch.session = Some(session.clone());
    pad.of(session, &balthasar_model::AgentId::main())
        .expect("pad")
        .keep_scratch(scratch)
        .expect("keep");

    (memory, transcript, runs)
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
fn forgetting_a_run_leaves_none_of_it_on_disk() {
    let at = Scratch::new("balthasar-cli-forget", "one");
    let session = SessionId::new("01DOOMED");
    let (memory, transcript, runs) = planted(&at, &session);

    assert!(anywhere(&at), "the secret is on disk to begin with");

    let ran = Command::new(env!("CARGO_BIN_EXE_balthasar"))
        .args([
            "--store",
            &memory.to_string_lossy(),
            "forget",
            "01DOOMED",
            "--session",
            "--purge",
            "--yes",
        ])
        .output()
        .expect("balthasar forget");
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );

    for place in [&memory, &transcript, &runs] {
        assert!(
            !anywhere(place),
            "the secret survived in {}",
            place.display()
        );
    }
}

#[test]
fn forgetting_a_run_nobody_has_heard_of_says_so() {
    // A typo must not report a successful purge of nothing: the next thing somebody does is
    // stop looking for the run they meant.
    let at = Scratch::new("balthasar-cli-typo", "one");
    let (memory, _, _) = planted(&at, &SessionId::new("01REAL"));

    let ran = Command::new(env!("CARGO_BIN_EXE_balthasar"))
        .args([
            "--store",
            &memory.to_string_lossy(),
            "forget",
            "01TYPO",
            "--session",
            "--purge",
            "--yes",
        ])
        .output()
        .expect("balthasar forget");
    assert!(!ran.status.success(), "it refused");
    assert!(
        String::from_utf8_lossy(&ran.stderr).contains("no run called"),
        "and said why: {}",
        String::from_utf8_lossy(&ran.stderr)
    );
}

/// A fact holding [`SECRET`], and one turn of `session` quoting it, under `at`.
fn quoted(at: &Path, session: &SessionId) -> (PathBuf, PathBuf, String) {
    let memory = at.join("store.db");
    let transcript = at.join("store-transcript.db");
    let scope = ScopeId::new("/w/thing");
    let mut store = balthasar_store::Store::open(&memory).expect("store");
    let fact = Memory::new(
        mint(NOW),
        Tier::Fact,
        scope.clone(),
        Body::note(SECRET, NoteKind::Claim),
        NOW,
    );
    let witness = balthasar_model::Witness::new(
        balthasar_model::WitnessId::new("w-1"),
        balthasar_model::WitnessKind::Imperative,
        session.clone(),
        scope,
        NOW,
    );
    let id = store
        .remember(fact, witness, NOW)
        .expect("remembered")
        .id()
        .to_string();
    let mut held = balthasar_store::Transcript::open(&transcript).expect("transcript");
    held.open_run(session, "/w/thing", "/w/thing", "test", NOW)
        .expect("run");
    let text = format!("remember: {SECRET}");
    let turn = balthasar_store::Turn {
        cursor: 1,
        at: NOW,
        role: "user".into(),
        kind: "prose".into(),
        text: text.clone(),
        raw: Some(serde_json::json!({ "text": text }).to_string()),
        ..balthasar_store::Turn::default()
    };
    held.write(session, &turn).expect("turn");
    (memory, transcript, id)
}

fn forget(memory: &Path, id: &str, extra: &[&str]) -> std::process::Output {
    let store = memory.to_string_lossy();
    let mut args = vec![
        "--store", &store, "forget", id, "--purge", "--yes", "--json",
    ];
    args.extend(extra);
    Command::new(env!("CARGO_BIN_EXE_balthasar"))
        .args(&args)
        .output()
        .expect("balthasar forget")
}

#[test]
fn purging_a_memory_names_the_turns_still_quoting_it() {
    let at = Scratch::new("balthasar-cli-quoted", "kept");
    let (memory, transcript, id) = quoted(&at, &SessionId::new("01SAID"));
    let ran = forget(&memory, &id, &[]);
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let said: serde_json::Value = serde_json::from_slice(&ran.stdout).expect("json");
    let row = &said["result"][0];
    assert_eq!(
        row["quoted_by"],
        serde_json::json!([{"session": "01SAID", "cursor": 1}])
    );
    assert_eq!(row["redacted"], 0);
    assert!(anywhere(&transcript), "a purge alone leaves what was said");
    assert!(!anywhere(&memory), "and takes the memory");
}

#[test]
fn redacting_takes_the_text_out_of_every_turn_quoting_it() {
    let at = Scratch::new("balthasar-cli-quoted", "redacted");
    let session = SessionId::new("01SAID");
    let (memory, transcript, id) = quoted(&at, &session);
    let ran = forget(&memory, &id, &["--redact-turns"]);
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let said: serde_json::Value = serde_json::from_slice(&ran.stdout).expect("json");
    assert_eq!(said["result"][0]["redacted"], 1);
    assert!(!anywhere(&at), "nowhere on disk");
    let held = balthasar_store::Transcript::open(&transcript).expect("transcript");
    let turns = held.replay(&session).expect("replay");
    assert_eq!(
        turns[0].text,
        format!("remember: {}", balthasar_store::REDACTED)
    );
}

#[test]
fn redacting_without_purging_is_refused() {
    let at = Scratch::new("balthasar-cli-quoted", "alone");
    let (memory, _, id) = quoted(&at, &SessionId::new("01SAID"));
    let ran = Command::new(env!("CARGO_BIN_EXE_balthasar"))
        .args([
            "--store",
            &memory.to_string_lossy(),
            "forget",
            &id,
            "--redact-turns",
        ])
        .output()
        .expect("balthasar forget");
    assert!(!ran.status.success());
}
