//! Ties come back in one order, whichever process asks: against the real binary, twice.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::scratch::Scratch;
use balthasar_model::{
    Body, Memory, MemoryId, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId,
    WitnessKind,
};
use std::path::Path;
use std::process::Command;

const AT: Timestamp = 1_756_000_000;

/// Twelve memories a search for `QWTIE` scores alike, filed in no particular order.
fn tied(path: &Path) {
    let mut store = balthasar_store::Store::open(path).expect("store");
    for n in [7, 2, 11, 0, 9, 4, 1, 10, 5, 3, 8, 6] {
        let memory = Memory::new(
            MemoryId::new(format!("01TIE{n:02}")),
            Tier::Fact,
            ScopeId::new("/w/thing"),
            Body::note(format!("QWTIE item{n:02} keeps one shape"), NoteKind::Claim),
            AT,
        );
        let witness = Witness::new(
            WitnessId::new(format!("w-{n}")),
            WitnessKind::Imperative,
            SessionId::new("s1"),
            ScopeId::new("/w/thing"),
            AT,
        );
        store.remember(memory, witness, AT).expect("remembered");
    }
}

/// The ids a `recall` in a process of its own printed, in its order.
fn asked(store: &str) -> Vec<String> {
    let at = (AT + 1).to_string();
    let ran = Command::new(env!("CARGO_BIN_EXE_balthasar"))
        .args([
            "--store", store, "--at", &at, "recall", "QWTIE", "--limit", "8", "--json",
        ])
        .output()
        .expect("balthasar");
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let said: serde_json::Value = serde_json::from_slice(&ran.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&ran.stdout)));
    said["result"]
        .as_array()
        .expect("rows")
        .iter()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn same_store_same_query_same_order() {
    let at = Scratch::new("balthasar-order", "tied");
    let path = at.join("store.db");
    tied(&path);
    let store = path.to_string_lossy().into_owned();
    let first = asked(&store);
    assert_eq!(first.len(), 8, "{first:?}");
    for _ in 0..3 {
        assert_eq!(asked(&store), first);
    }
    let mut sorted = first.clone();
    sorted.sort();
    assert_eq!(first, sorted, "a tie is broken by id");
}
