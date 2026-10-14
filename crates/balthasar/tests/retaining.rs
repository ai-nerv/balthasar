//! Turn retention through the real binary: told by a coordinator, rehearsed by `consolidate`,
//! applied by `consolidate --now`, and nothing at all when nobody named a number of days.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::scratch::Scratch;
use balthasar_model::{SessionId, Timestamp};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const NOW: Timestamp = 1_800_000_000;
const DAY: Timestamp = 86_400;

/// One session that went quiet forty days before [`NOW`], in the files `--store` implies.
fn planted(at: &Path) -> String {
    let held = balthasar_store::Transcript::open(&at.join("store-transcript.db")).expect("open");
    let turn = balthasar_store::Turn {
        at: NOW - 40 * DAY,
        role: "user".into(),
        kind: "prose".into(),
        text: "an old thing to have said".into(),
        ..balthasar_store::Turn::default()
    };
    held.insert(&SessionId::new("quiet"), &turn).expect("turn");
    at.join("store.db").to_string_lossy().into_owned()
}

fn held(at: &Path) -> u64 {
    balthasar_store::Transcript::open(&at.join("store-transcript.db"))
        .expect("open")
        .count(&SessionId::new("quiet"))
        .expect("count")
}

/// One `balthasar` run, its configuration and its coordinator's directory under `at`.
fn balthasar(at: &Path, args: &[&str], stdin: &str) -> serde_json::Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_balthasar"))
        .args(args)
        .current_dir(at)
        .env("XDG_CONFIG_HOME", at.join("config"))
        .env("XDG_DATA_HOME", at.join("data"))
        .env("NERV_GIVEN", at.join("given"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("balthasar");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("told");
    let ran = child.wait_with_output().expect("ran");
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    serde_json::from_slice(&ran.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&ran.stdout)))
}

fn consolidate(at: &Path, store: &str, now: bool) -> serde_json::Value {
    let at_now = NOW.to_string();
    let mut args = vec!["--store", store, "--at", &at_now, "consolidate", "--json"];
    if now {
        args.push("--now");
    }
    balthasar(at, &args, "")["result"][0]["retired"].clone()
}

#[test]
fn with_no_number_of_days_named_every_turn_is_kept() {
    let at = Scratch::new("balthasar-cli-retain", "unset");
    let store = planted(&at);
    let retired = consolidate(&at, &store, true);
    assert_eq!(retired["sessions"], 0);
    assert_eq!(held(&at), 1);
}

#[test]
fn a_coordinators_number_of_days_is_rehearsed_and_then_applied() {
    let at = Scratch::new("balthasar-cli-retain", "told");
    let store = planted(&at);
    let told = balthasar(&at, &["configure", "--json"], "balthasar.retain_days = 30");
    assert_eq!(told["result"][0]["set"], serde_json::json!(["retain_days"]));

    let rehearsed = consolidate(&at, &store, false);
    assert_eq!(
        (rehearsed["sessions"].as_u64(), rehearsed["turns"].as_u64()),
        (Some(1), Some(1))
    );
    assert_eq!(held(&at), 1, "a rehearsal removes nothing");

    let applied = consolidate(&at, &store, true);
    assert_eq!(
        (applied["sessions"].as_u64(), applied["turns"].as_u64()),
        (Some(1), Some(1))
    );
    assert_eq!(held(&at), 0);
}
