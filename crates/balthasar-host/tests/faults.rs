//! F5: damaged stores and hostile conditions. Nothing panics, a failure is a reply, and a
//! replaced value never reaches a layout. `BALTHASAR_FAULT_REPS` repeats each case (default 3).

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_host::{Answering, Door, Hooks, answer_hooked};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{
    AgentId, Body, Memory, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
    floor, scratch::Scratch,
};
use balthasar_store::{Store, Transcript, mint};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::panic::{AssertUnwindSafe, catch_unwind};

const NOW: Timestamp = 1_756_000_000;
const DAY: Timestamp = 86_400;
const SESSION: &str = "01FAULT";
const SCOPE: &str = "/w/thing";

struct Plain;
impl Hooks for Plain {}

fn reps() -> usize {
    std::env::var("BALTHASAR_FAULT_REPS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(3)
}

fn ask(
    store: &mut Store,
    scrollback: &mut Transcript,
    now: Timestamp,
    call: &str,
    args: Value,
) -> Reply {
    let mut at = Answering {
        store,
        scrollback: Some(scrollback),
        scratch: None,
        scope: ScopeId::new(SCOPE),
        agent: AgentId::main(),
        now,
        inject_floor: floor::INJECT,
        live_floor: floor::LIVE,
        capture: false,
    };
    let args = match args {
        Value::Array(args) => args,
        one => vec![one],
    };
    answer_hooked(
        &mut at,
        &Door::Owner,
        &Request {
            call: call.into(),
            args,
        },
        &mut Plain,
    )
}

/// F1 in `store`: an old codec closed by a new one.
fn seed(store: &mut Store) {
    for (value, run, at, kind) in [
        ("QWOLDVALUE", "s1", NOW - 30 * DAY, WitnessKind::Imperative),
        ("QWNEWVALUE", "s2", NOW - 2 * DAY, WitnessKind::Correction),
    ] {
        let memory = Memory::new(
            mint(at),
            Tier::Fact,
            ScopeId::new(SCOPE),
            Body::fact("transport", "codec", value),
            at,
        );
        let witness = Witness::new(
            WitnessId::new(format!("w-{run}")),
            kind,
            SessionId::new(run),
            ScopeId::new(SCOPE),
            at,
        );
        store.remember(memory, witness, at).expect("seeded");
    }
}

/// Ask for a turn and a layout; whatever comes back, it is a reply and holds no closed value.
fn lay(store: &mut Store, scrollback: &mut Transcript, now: Timestamp) {
    let turn = json!({ "cursor": 0, "role": "user", "kind": "user", "tokens": 9,
                       "text": "which transport codec" });
    let _ = ask(store, scrollback, now, "observe", json!([SESSION, turn]));
    let laid = ask(
        store,
        scrollback,
        now,
        "layout",
        json!([SESSION, { "round": 0,
        "window": 200_000, "reply": 8_000, "fixed": { "system": 100, "tools": 100 },
        "query": "which transport codec", "helpers": [] }]),
    );
    let said = serde_json::to_string(&laid.result).expect("json");
    assert!(
        !said.contains("QWOLDVALUE"),
        "a closed value reached a layout: {said}"
    );
}

/// `case` run `reps` times, each in a fresh directory, never panicking.
fn hostile(name: &str, case: impl Fn(&std::path::Path)) {
    for n in 0..reps() {
        let dir = Scratch::new("balthasar-faults", &format!("{name}-{n}"));
        let ran = catch_unwind(AssertUnwindSafe(|| case(&dir)));
        assert!(ran.is_ok(), "{name} panicked on repetition {n}");
    }
}

#[test]
fn a_garbage_store_is_refused_not_panicked_on() {
    hostile("garbage", |dir| {
        let path = dir.join("memory.db");
        std::fs::write(&path, b"this is not a database at all").expect("garbage");
        assert!(Store::open(&path).is_err());
    });
}

#[test]
fn a_store_from_a_future_schema_is_refused() {
    hostile("future", |dir| {
        let path = dir.join("memory.db");
        drop(Store::open(&path).expect("made"));
        rusqlite::Connection::open(&path)
            .and_then(|raw| raw.pragma_update(None, "user_version", 99))
            .expect("future");
        let refused = Store::open(&path).err().expect("refused").to_string();
        assert!(refused.contains("99"), "{refused}");
    });
}

#[test]
fn a_read_only_store_answers_with_refusals() {
    hostile("readonly", |dir| {
        let path = dir.join("memory.db");
        let mut store = Store::open(&path).expect("made");
        seed(&mut store);
        drop(store);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).expect("ro");
        if let Ok(mut store) = Store::open(&path) {
            let mut scrollback = Transcript::ephemeral().expect("scrollback");
            lay(&mut store, &mut scrollback, NOW);
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("rw");
    });
}

#[test]
fn a_clock_moved_back_never_revives_a_closed_value() {
    hostile("clock", |_| {
        let mut store = Store::ephemeral().expect("store");
        seed(&mut store);
        let mut scrollback = Transcript::ephemeral().expect("scrollback");
        lay(&mut store, &mut scrollback, NOW - 40 * DAY);
    });
}

#[test]
fn two_handles_writing_one_store_stay_consistent() {
    hostile("two", |dir| {
        let path = dir.join("memory.db");
        let mut first = Store::open(&path).expect("first");
        let mut second = Store::open(&path).expect("second");
        seed(&mut first);
        seed(&mut second);
        let mut scrollback = Transcript::ephemeral().expect("scrollback");
        lay(&mut first, &mut scrollback, NOW);
        lay(&mut second, &mut scrollback, NOW);
    });
}

#[test]
fn a_writer_holding_the_lock_delays_but_does_not_break_a_layout() {
    hostile("locked", |dir| {
        let path = dir.join("memory.db");
        let mut store = Store::open(&path).expect("made");
        seed(&mut store);
        let holder = rusqlite::Connection::open(&path).expect("holder");
        holder.execute_batch("BEGIN IMMEDIATE").expect("locked");
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            holder.execute_batch("ROLLBACK").expect("released");
        });
        let mut scrollback = Transcript::ephemeral().expect("scrollback");
        lay(&mut store, &mut scrollback, NOW);
        release.join().expect("joined");
    });
}

#[test]
fn an_oversized_row_is_answered_and_the_next_layout_works() {
    hostile("oversized", |_| {
        let mut store = Store::ephemeral().expect("store");
        seed(&mut store);
        let mut scrollback = Transcript::ephemeral().expect("scrollback");
        let huge = json!({ "cursor": 5, "role": "tool", "kind": "tool_result", "tool": "shell",
                           "tokens": 5_000_000, "text": "x".repeat(20 * 1024 * 1024) });
        let _ = ask(
            &mut store,
            &mut scrollback,
            NOW,
            "observe",
            json!([SESSION, huge]),
        );
        lay(&mut store, &mut scrollback, NOW);
    });
}
