//! What `--json` means on the command line: the family's reply shape, whatever happened.
//!
//! The floor verbs have answered in it since the contract existed; the data verbs printed bare
//! rows, and a failure printed a sentence on stderr and exited 1 — which is the one answer a
//! sibling cannot tell from balthasar not being installed.

use balthasar_model::scratch::Scratch;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A store with two memories in it, at the path `--store` implies.
fn planted(at: &Path) -> PathBuf {
    let memory = at.join("store.db");
    for text in ["we deploy with fly", "we deploy the worker separately"] {
        let ran = balthasar(&memory, &["remember", text]);
        assert!(ran.status.success(), "seeding failed");
    }
    memory
}

fn balthasar(store: &Path, args: &[&str]) -> Output {
    let mut all = vec!["--store".to_owned(), store.to_string_lossy().into_owned()];
    all.extend(args.iter().map(|a| (*a).to_owned()));
    Command::new(env!("CARGO_BIN_EXE_balthasar"))
        .args(&all)
        .output()
        .expect("balthasar")
}

/// What it printed on standard output, decoded.
fn said(store: &Path, args: &[&str]) -> serde_json::Value {
    let ran = balthasar(store, args);
    let text = String::from_utf8_lossy(&ran.stdout).into_owned();
    serde_json::from_str(&text).unwrap_or_else(|why| {
        panic!(
            "balthasar {args:?} did not answer in the reply shape: {why}\nstdout: {text}\nstderr: {}",
            String::from_utf8_lossy(&ran.stderr)
        )
    })
}

#[test]
fn a_listing_verb_answers_with_its_rows() {
    // `n` is how many were found. Bare rows, one JSON object per line, is not a reply at all:
    // a caller decoding stdout gets the first memory and never learns there were others.
    let at = Scratch::new("balthasar-framed", "rows");
    let store = planted(&at);

    let reply = said(&store, &["recall", "deploy", "--json"]);
    assert_eq!(reply["ok"], serde_json::json!(true), "{reply}");
    assert_eq!(reply["family"], serde_json::json!(1), "{reply}");
    let rows = reply["result"].as_array().expect("result is a list");
    assert_eq!(reply["n"].as_u64().expect("a count"), rows.len() as u64);
    assert_eq!(rows.len(), 2, "both memories, each its own row: {reply}");
    for row in rows {
        assert!(!row.is_array(), "a row is a memory, not the listing: {row}");
        assert!(row.get("id").is_some(), "{row}");
    }
}

#[test]
fn a_verb_that_answers_with_one_record_still_frames_it() {
    let at = Scratch::new("balthasar-framed", "record");
    let store = planted(&at);

    let reply = said(&store, &["decay", "--json"]);
    assert_eq!(reply["ok"], serde_json::json!(true), "{reply}");
    assert_eq!(reply["n"], serde_json::json!(1), "{reply}");
    assert!(reply["result"][0].is_object(), "{reply}");
}

#[test]
fn a_refusal_is_a_reply_on_stdout_at_exit_zero() {
    // The whole reason the shape exists. "exited 1, a sentence on stderr" is what a missing
    // binary looks like, and a sibling has to be able to tell the two apart.
    let at = Scratch::new("balthasar-framed", "refused");
    let store = planted(&at);

    let ran = balthasar(&store, &["why", "no-such-memory", "--json"]);
    assert!(
        ran.status.success(),
        "a refusal exits zero; this exited {:?} with {}",
        ran.status.code(),
        String::from_utf8_lossy(&ran.stderr)
    );
    let reply: serde_json::Value =
        serde_json::from_slice(&ran.stdout).expect("a refusal is a reply on stdout");
    assert_eq!(reply["ok"], serde_json::json!(false), "{reply}");
    assert_eq!(reply["n"], serde_json::json!(0), "{reply}");
    assert_eq!(reply["result"], serde_json::json!([]), "{reply}");
    assert!(
        reply["error"]
            .as_str()
            .expect("a reason")
            .contains("no-such-memory"),
        "and it names what was asked for: {reply}"
    );
}

#[test]
fn without_the_flag_a_person_still_gets_what_a_person_reads() {
    // The bare output is nobody's wire and is left alone. Framing it would have turned every
    // `balthasar recall` in a terminal into a wall of JSON.
    let at = Scratch::new("balthasar-framed", "bare");
    let store = planted(&at);

    let ran = balthasar(&store, &["recall", "deploy"]);
    let text = String::from_utf8_lossy(&ran.stdout);
    assert!(text.contains("we deploy with fly"), "{text}");
    assert!(!text.contains("\"ok\":true"), "{text}");
}

#[test]
fn the_export_pair_stays_the_corpus_that_import_reads() {
    // The deliberate exception, and the reason it is one: `export` writes what `import` reads
    // back, one memory per line. A reply shape between them would be a format nothing consumes.
    let at = Scratch::new("balthasar-framed", "corpus");
    let store = planted(&at);

    let dumped = balthasar(&store, &["export", "--json"]);
    let text = String::from_utf8_lossy(&dumped.stdout);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2, "one memory per line: {text}");
    for line in &lines {
        let row: serde_json::Value = serde_json::from_str(line).expect("a memory per line");
        assert!(row.get("id").is_some(), "{row}");
    }
}
