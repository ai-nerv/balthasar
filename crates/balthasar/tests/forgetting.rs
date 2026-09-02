//! `balthasar forget --session` against the real binary, over the real file layout.
//!
//! The store-level test proves a purge closes all three of a run's hiding places. This proves
//! the command finds all three — which is a different thing, and the half that breaks when a
//! path is computed twice in two places.

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
    pad.of(session)
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
    let at = std::env::temp_dir().join(format!("balthasar-cli-forget-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&at);
    std::fs::create_dir_all(&at).expect("workspace");
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
    let _ = std::fs::remove_dir_all(&at);
}

#[test]
fn forgetting_a_run_nobody_has_heard_of_says_so() {
    // A typo must not report a successful purge of nothing: the next thing somebody does is
    // stop looking for the run they meant.
    let at = std::env::temp_dir().join(format!("balthasar-cli-typo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&at);
    std::fs::create_dir_all(&at).expect("workspace");
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
    let _ = std::fs::remove_dir_all(&at);
}
