//! `memo distil` against the real binary: a live run teaches the project something.
//!
//! The gap this closes end to end. A session that came in over the socket lands in memo's own
//! scrollback, and until now no rule ever read it — the extractors only ran on `memo ingest`,
//! which walks a *harness's* journal files. This plants a run the way `observe` would and asks
//! the shipped binary whether the project learned from it.

use memo_model::SessionId;
use std::path::{Path, PathBuf};
use std::process::Command;

const NOW: memo_model::Timestamp = 1_756_000_000;

/// A run in the scrollback, at the path `--store` implies.
fn planted(at: &Path, session: &SessionId) -> PathBuf {
    let memory = at.join("store.db");
    let transcript = at.join("store-transcript.db");

    let mut held = memo_store::Transcript::open(&transcript).expect("transcript");
    held.open_run(session, "/w/thing", "/w/thing", "test", NOW)
        .expect("run");

    let turns = [
        (
            "user",
            "prose",
            "remember: we deploy with fly.io",
            None,
            None,
        ),
        ("assistant", "prose", "noted", None, None),
        (
            "tool",
            "tool_result",
            "failed",
            Some(false),
            Some(r#"{"command":"cargo test"}"#),
        ),
        (
            "tool",
            "tool_result",
            "ok",
            Some(true),
            Some(r#"{"command":"make test"}"#),
        ),
    ];
    for (cursor, (role, kind, text, ok, args)) in turns.iter().enumerate() {
        held.write(
            session,
            &memo_store::Turn {
                cursor: cursor as u64 + 1,
                at: NOW,
                role: (*role).to_owned(),
                kind: (*kind).to_owned(),
                text: (*text).to_owned(),
                tool: (*role == "tool").then(|| "shell".to_owned()),
                ok: *ok,
                args: args.map(str::to_owned),
                ..memo_store::Turn::default()
            },
        )
        .expect("turn");
    }
    memory
}

/// Run the shipped binary and hand back what it printed.
fn memo(store: &Path, args: &[&str]) -> String {
    let mut all = vec!["--store".to_owned(), store.to_string_lossy().into_owned()];
    all.extend(args.iter().map(|a| (*a).to_owned()));
    let ran = Command::new(env!("CARGO_BIN_EXE_memo"))
        .args(&all)
        .output()
        .expect("memo");
    assert!(
        ran.status.success(),
        "memo {args:?} failed: {}",
        String::from_utf8_lossy(&ran.stderr)
    );
    String::from_utf8_lossy(&ran.stdout).into_owned()
}

#[test]
fn a_run_streamed_into_memo_teaches_the_project() {
    let at = std::env::temp_dir().join(format!("memo-distil-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&at);
    std::fs::create_dir_all(&at).expect("workspace");
    let session = SessionId::new("01LIVE");
    let store = planted(&at, &session);

    // A rehearsal first: it must say what would cross and write nothing.
    let dry = memo(&store, &["distil", "--json"]);
    assert!(dry.contains("\"dry_run\":true"), "{dry}");
    assert!(
        !memo(&store, &["recall", "deploy"]).contains("fly.io"),
        "a rehearsal wrote nothing"
    );

    let done = memo(&store, &["distil", "--now", "--json"]);
    assert!(done.contains("\"runs\":1"), "{done}");

    let found = memo(&store, &["recall", "deploy"]);
    assert!(
        found.contains("fly.io"),
        "the project learned what the run said: {found}"
    );

    // Reading again is free and teaches nothing new.
    let again = memo(&store, &["distil", "--now", "--json"]);
    assert!(again.contains("\"runs\":0"), "{again}");

    let _ = std::fs::remove_dir_all(&at);
}

#[test]
fn consolidating_reads_the_runs_before_looking_for_what_they_share() {
    // The pass runs on a timer, so this is the path that actually fires in practice. Nobody
    // should have to know `memo distil` exists for a live run to be read.
    let at = std::env::temp_dir().join(format!("memo-sleep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&at);
    std::fs::create_dir_all(&at).expect("workspace");
    let store = planted(&at, &SessionId::new("01SLEEP"));

    let done = memo(&store, &["consolidate", "--now", "--json"]);
    assert!(done.contains("\"read\":1"), "{done}");

    let found = memo(&store, &["recall", "deploy"]);
    assert!(found.contains("fly.io"), "{found}");
    let _ = std::fs::remove_dir_all(&at);
}
