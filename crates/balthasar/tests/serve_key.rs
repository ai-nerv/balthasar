//! The key `serve` is told on its standard input when it starts, or mints and publishes beside its
//! socket when it is told none, and which every call that writes carries: against the real binary.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

mod serving;

use serde_json::json;
use serving::{KEY, Serving};
use std::path::{Path, PathBuf};

/// Where a key for the socket at `socket` is published.
fn key_file(socket: &Path) -> PathBuf {
    let mut named = socket.as_os_str().to_owned();
    named.push(".key");
    PathBuf::from(named)
}

#[test]
fn serve_told_a_malformed_key_refuses_to_start() {
    let upper = format!("{}\n", KEY.to_uppercase());
    for fed in ["not a key\n", KEY, upper.as_str()] {
        let said = Serving::fed("badkey", fed, &[])
            .err()
            .unwrap_or_else(|| panic!("{fed:?}: it started"));
        assert!(said.contains("64 lowercase hex"), "{fed:?}: {said}");
    }
}

#[test]
fn serve_told_its_key_publishes_none() {
    let serving = Serving::start("told");
    assert!(!key_file(&serving.socket).exists());
}

#[test]
fn serve_told_no_key_publishes_one_of_its_own_and_takes_it_with_it() {
    use std::os::unix::fs::PermissionsExt;
    let mut serving = Serving::fed("minted", "", &[]).expect("it starts");
    let file = key_file(&serving.socket);
    let published = std::fs::read_to_string(&file).expect("a key beside the socket");
    let mode = std::fs::metadata(&file).expect("stat").permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let minted = published.trim();
    assert_eq!(minted.len(), 64, "{minted:?}");

    let turn = json!({ "cursor": 1, "role": "user", "kind": "prose", "text": "hi", "raw": {} });
    let refused = serving.ask_with(None, "observe", json!(["s", turn]));
    assert_eq!(refused["error"], balthasar_ipc::KEY_REFUSED, "{refused}");
    let taken = serving.ask_with(Some(minted), "observe", json!(["s", turn]));
    assert_eq!(taken["ok"], true, "{taken}");

    let sent = std::process::Command::new("kill")
        .args(["-TERM", &serving.child.id().to_string()])
        .status()
        .expect("kill");
    assert!(sent.success());
    assert!(serving.child.wait().expect("reaped").success());
    assert!(
        !file.exists(),
        "the key outlived the serve that published it"
    );
}

#[test]
fn a_write_needs_the_key_and_a_read_does_not() {
    let serving = Serving::start("keys");
    let other = KEY.replace('5', "6");
    for key in [None, Some(other.as_str()), Some("")] {
        let told = serving.ask_with(key, "remember", json!(["a note"]));
        assert_eq!(told["fault"], "refused", "{key:?}: {told}");
        assert_eq!(told["error"], balthasar_ipc::KEY_REFUSED, "{key:?}: {told}");
    }
    let kept = serving.ask("remember", json!(["a note"]));
    assert_eq!(kept["ok"], true, "{kept}");
    for verb in ["verbs", "sessions", "status"] {
        let read = serving.ask_with(None, verb, json!([]));
        assert_eq!(read["ok"], true, "{verb}: {read}");
    }
}

#[test]
fn a_second_peer_of_the_same_user_cannot_write_into_the_session() {
    let serving = Serving::start("second");
    let turn = |cursor: u64, text: &str| json!({ "cursor": cursor, "role": "user", "kind": "prose", "text": text, "raw": {} });
    let ours = serving.ask("observe", json!(["s", turn(1, "what the session said")]));
    assert_eq!(ours["ok"], true, "{ours}");

    for (verb, said) in [
        ("observe", turn(2, "planted")),
        ("amend", turn(1, "rewritten")),
        (
            "observe",
            json!({ "cursor": 3, "role": "trace", "kind": "permission", "text": "x" }),
        ),
    ] {
        let told = serving.ask_with(None, verb, json!(["s", said]));
        assert_eq!(told["error"], balthasar_ipc::KEY_REFUSED, "{verb}: {told}");
    }

    let replayed = serving.ask_with(None, "replay", json!(["s"]));
    let rows = replayed["result"].as_array().expect("rows");
    assert_eq!(rows.len(), 1, "{replayed}");
    assert_eq!(rows[0]["text"], "what the session said");
    assert_eq!(rows[0]["revisions"], 0, "never amended");
}
