//! Nothing closed, archived or forgotten reaches a prompt, by any path a layout or recall takes.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_host::{Answering, Door, Hooks, answer_hooked};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{
    Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
    floor,
};
use balthasar_store::{Store, Transcript, mint};
use serde_json::{Value, json};

const NOW: Timestamp = 1_756_000_000;
const DAY: Timestamp = 86_400;
const SESSION: &str = "01SERVED";
const SCOPE: &str = "/w/thing";
const ASKED: &str = "which transport codec do we use in crates/net/codec.rs";
const GONE: [&str; 3] = ["QWOLDVALUE", "QWARCHIVED", "QWFORGOTTEN"];

struct Plain;
impl Hooks for Plain {}

struct Held {
    store: Store,
    scrollback: Transcript,
    now: Timestamp,
    capture: bool,
}

fn saw(kind: WitnessKind, run: &str, at: Timestamp) -> Witness {
    Witness::new(
        WitnessId::new(format!("w-{run}-{at}")),
        kind,
        SessionId::new(run),
        ScopeId::new(SCOPE),
        at,
    )
}

fn memory(body: Body, at: Timestamp) -> Memory {
    Memory::new(mint(at), Tier::Fact, ScopeId::new(SCOPE), body, at)
}

impl Held {
    /// F1: a replaced pair, an archived memory and a forgotten one, all naming the codec.
    fn seeded() -> Self {
        let mut held = Self {
            store: Store::ephemeral().expect("store"),
            scrollback: Transcript::ephemeral().expect("scrollback"),
            now: NOW,
            capture: false,
        };
        let old = Body::fact("transport", "codec", "QWOLDVALUE");
        let new = Body::fact("transport", "codec", "QWNEWVALUE");
        held.remember(old, WitnessKind::Imperative, "s1", NOW - 30 * DAY);
        held.remember(new, WitnessKind::Correction, "s2", NOW - 2 * DAY);

        let archived = Body::note("the transport codec was QWARCHIVED", NoteKind::Claim);
        let id = held.remember(archived, WitnessKind::Imperative, "s3", NOW - 9 * DAY);
        held.store.archive(&id, NOW - DAY).expect("archived");

        let forgotten = Body::note(
            "the transport codec QWFORGOTTEN lives in crates/net/codec.rs",
            NoteKind::Claim,
        );
        let id = held.remember(forgotten, WitnessKind::Imperative, "s4", NOW - 8 * DAY);
        let reply = held.ask(&Door::Owner, "forget", vec![json!(id.to_string())]);
        assert!(reply.ok, "forget: {:?}", reply.error);
        held
    }

    fn remember(
        &mut self,
        body: Body,
        kind: WitnessKind,
        run: &str,
        at: Timestamp,
    ) -> balthasar_model::MemoryId {
        self.store
            .remember(memory(body, at), saw(kind, run, at), at)
            .expect("remembered")
            .id()
            .clone()
    }

    fn ask(&mut self, door: &Door, call: &str, args: Vec<Value>) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: Some(&mut self.scrollback),
            scratch: None,
            scope: ScopeId::new(SCOPE),
            agent: balthasar_model::AgentId::main(),
            now: self.now,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: self.capture,
        };
        let request = Request {
            call: call.into(),
            args,
        };
        answer_hooked(&mut at, door, &request, &mut Plain)
    }

    fn one(&mut self, call: &str, second: Value) -> Value {
        let reply = self.ask(&Door::Owner, call, vec![json!(SESSION), second]);
        assert!(reply.ok, "{call}: {:?}", reply.error);
        reply.result.into_iter().next().unwrap_or(Value::Null)
    }

    fn asked(&mut self) {
        let turn = json!({ "cursor": 0, "role": "user", "kind": "user", "tokens": 20,
                           "text": ASKED });
        let reply = self.ask(&Door::Owner, "observe", vec![json!(SESSION), turn]);
        assert!(reply.ok, "{:?}", reply.error);
    }
}

fn window() -> Value {
    json!({ "round": 0, "window": 200_000, "reply": 8_000,
            "fixed": { "system": 600, "tools": 400 }, "query": ASKED, "idle_s": 5,
            "helpers": [] })
}

/// Every word a layout puts in front of a model.
fn said(laid: &Value) -> String {
    laid["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .filter_map(|slot| slot["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_served(text: &str) {
    for gone in GONE {
        assert!(!text.contains(gone), "{gone} reached a prompt:\n{text}");
    }
    assert!(
        text.contains("QWNEWVALUE"),
        "the current answer is offered:\n{text}"
    );
}

#[test]
fn a_closed_memory_is_never_packed() {
    let mut held = Held::seeded();
    held.asked();
    let laid = held.one("layout", window());
    assert_served(&said(&laid));
}

#[test]
fn nor_after_a_decay_pass() {
    let mut held = Held::seeded();
    held.store.decay(NOW).expect("decayed");
    held.asked();
    assert_served(&said(&held.one("layout", window())));
}

#[test]
fn nor_with_the_clock_moved_back() {
    let mut held = Held::seeded();
    held.now = NOW - 3 * DAY;
    held.asked();
    assert_served(&said(&held.one("layout", window())));
}

#[test]
fn nor_after_applied_or_overflowed() {
    let mut held = Held::seeded();
    held.asked();
    let laid = held.one("layout", window());
    held.one(
        "applied",
        json!({ "id": laid["id"], "usage": { "input": 900, "output": 10 } }),
    );
    assert_served(&said(&held.one(
        "layout",
        json!({ "round": 1, "window": 200_000,
        "reply": 8_000, "fixed": { "system": 600, "tools": 400 }, "query": ASKED,
        "idle_s": 5, "helpers": [] }),
    )));
    let tighter = held.one(
        "overflowed",
        json!({ "id": laid["id"], "said": "prompt is too long: 250000 tokens" }),
    );
    assert_served(&said(&tighter));
}

#[test]
fn a_peer_recall_finds_only_open_memories() {
    let mut held = Held::seeded();
    let peer = Door::Socket(balthasar_ipc::Peer {
        pid: 4242,
        uid: 1000,
        program: Some("/usr/bin/some-harness".to_owned()),
        agent: None,
    });
    let reply = held.ask(
        &peer,
        "recall",
        vec![json!(ASKED), json!({ "limit": 50, "history": true })],
    );
    assert!(reply.ok, "{:?}", reply.error);
    let text = serde_json::to_string(&reply.result).expect("json");
    assert_served(&text);
}

#[test]
fn the_owner_may_ask_for_history_and_sees_the_replaced_answer() {
    let mut held = Held::seeded();
    let reply = held.ask(
        &Door::Owner,
        "recall",
        vec![
            json!("transport codec"),
            json!({ "limit": 50, "history": true }),
        ],
    );
    assert!(reply.ok, "{:?}", reply.error);
    let text = serde_json::to_string(&reply.result).expect("json");
    assert!(text.contains("QWOLDVALUE"), "{text}");
    assert!(text.contains("QWNEWVALUE"), "{text}");
}

#[test]
fn a_delivered_memory_gains_access_count() {
    let mut held = Held::seeded();
    held.asked();
    let laid = held.one("layout", window());
    let delivered: Vec<String> = laid["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .filter(|slot| slot["kind"] == "memory")
        .flat_map(|slot| slot["ids"].as_array().cloned().unwrap_or_default())
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect();
    assert!(!delivered.is_empty(), "something was delivered: {laid}");
    held.one(
        "applied",
        json!({ "id": laid["id"], "usage": { "input": 900, "output": 10 } }),
    );
    for id in delivered {
        let memory = held
            .store
            .get(&balthasar_model::MemoryId::new(id))
            .expect("read")
            .expect("there");
        assert_eq!(memory.strength.access_count, 1, "{}", memory.text());
    }
}

/// The memory slot's text, from a layout.
fn memory_slot(laid: &Value) -> String {
    laid["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .filter(|slot| slot["kind"] == "memory")
        .filter_map(|slot| slot["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_packed_line_has_a_handle_why_can_answer() {
    let mut held = Held::seeded();
    held.asked();
    let text = memory_slot(&held.one("layout", window()));
    let handles: Vec<String> = text
        .lines()
        .filter_map(|line| line.strip_prefix("- ["))
        .filter_map(|rest| rest.split(']').next())
        .map(str::to_owned)
        .collect();
    assert!(!handles.is_empty(), "{text}");
    for handle in handles {
        let reply = held.ask(&Door::Owner, "why", vec![json!(handle)]);
        assert!(reply.ok, "why {handle}: {:?}", reply.error);
    }
    assert!(
        text.contains("(you corrected, "),
        "origin and date are shown: {text}"
    );
}

#[test]
fn slot_bytes_are_identical_across_requests_without_changes() {
    let mut held = Held::seeded();
    held.asked();
    let first = memory_slot(&held.one("layout", window()));
    let again = memory_slot(&held.one(
        "layout",
        json!({ "round": 1, "window": 200_000, "reply": 8_000,
                "fixed": { "system": 600, "tools": 400 }, "query": ASKED, "idle_s": 5,
                "helpers": [] }),
    ));
    assert_eq!(first, again);
}

/// The catalog slot's text, from a layout.
fn catalog_slot(laid: &Value) -> String {
    laid["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .filter(|slot| slot["kind"] == "catalog")
        .filter_map(|slot| slot["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn catalog_lists_what_stands_and_names_only_the_handle_of_what_it_replaced() {
    let mut held = Held::seeded();
    held.asked();
    let text = catalog_slot(&held.one("layout", window()));
    assert!(text.contains("QWNEWVALUE"), "{text}");
    assert!(text.contains("· replaces ["), "{text}");
    for gone in GONE {
        assert!(!text.contains(gone), "{gone} in the catalog: {text}");
    }
    assert!(text.ends_with("That is everything on record."), "{text}");
}

#[test]
fn catalog_says_when_it_is_truncated() {
    let mut held = Held::seeded();
    for n in 0..200 {
        let body = Body::fact(
            "service",
            format!("port_of_worker_{n}"),
            format!("{}", 9000 + n),
        );
        held.remember(body, WitnessKind::Imperative, &format!("r{n}"), NOW - DAY);
    }
    held.asked();
    let text = catalog_slot(&held.one(
        "layout",
        json!({ "round": 0, "window": 40_000, "reply": 2_000,
                "fixed": { "system": 600, "tools": 400 }, "query": ASKED, "idle_s": 5,
                "helpers": [] }),
    ));
    assert!(text.contains("may still be held"), "{text}");
}

#[test]
fn catalog_bytes_are_stable_without_changes() {
    let mut held = Held::seeded();
    held.asked();
    let first = catalog_slot(&held.one("layout", window()));
    let again = catalog_slot(&held.one(
        "layout",
        json!({ "round": 1, "window": 200_000, "reply": 8_000,
                "fixed": { "system": 600, "tools": 400 }, "query": "something else",
                "idle_s": 5, "helpers": [] }),
    ));
    assert!(!first.is_empty());
    assert_eq!(first, again);
}

#[test]
fn the_ledger_records_what_a_layout_delivered_and_how() {
    let mut held = Held::seeded();
    held.capture = true;
    held.asked();
    let laid = held.one("layout", window());
    let ids = |kind: &str| -> Vec<String> {
        laid["slots"]
            .as_array()
            .expect("slots")
            .iter()
            .filter(|slot| slot["kind"] == kind)
            .flat_map(|slot| slot["ids"].as_array().cloned().unwrap_or_default())
            .filter_map(|id| id.as_str().map(str::to_owned))
            .collect()
    };
    let (memory, catalog) = (ids("memory"), ids("catalog"));
    assert!(!memory.is_empty() && !catalog.is_empty(), "{laid}");
    held.one(
        "applied",
        json!({ "id": laid["id"], "usage": { "input": 900, "output": 10 } }),
    );

    let id = laid["id"].as_str().expect("an id");
    let ledger = held
        .store
        .presented_in(&format!("applied-{id}"))
        .expect("ledger");
    for memory in &memory {
        let (_, mode) = ledger
            .iter()
            .find(|(held, _)| held.as_str() == memory)
            .expect("each delivered memory is recorded");
        assert_ne!(*mode, balthasar_model::Presentation::Listed);
    }
    for listed in catalog.iter().filter(|id| !memory.contains(id)) {
        assert!(
            ledger.iter().any(|(held, mode)| held.as_str() == listed
                && *mode == balthasar_model::Presentation::Listed),
            "{listed} was listed"
        );
    }
}

#[test]
fn with_the_ledger_off_nothing_delivered_is_recorded() {
    let mut held = Held::seeded();
    held.asked();
    let laid = held.one("layout", window());
    held.one(
        "applied",
        json!({ "id": laid["id"], "usage": { "input": 900, "output": 10 } }),
    );
    let id = laid["id"].as_str().expect("an id");
    assert!(
        held.store
            .presented_in(&format!("applied-{id}"))
            .expect("ledger")
            .is_empty()
    );
}

/// A layout for a prompt that says only `query`, after the turn at `cursor`.
fn said_next(held: &mut Held, cursor: u64, query: &str, round: u64) -> Value {
    let turn = json!({ "cursor": cursor, "role": "user", "kind": "user", "tokens": 2,
                       "text": query });
    let reply = held.ask(&Door::Owner, "observe", vec![json!(SESSION), turn]);
    assert!(reply.ok, "{:?}", reply.error);
    held.one(
        "layout",
        json!({ "round": round, "window": 200_000, "reply": 8_000,
                "fixed": { "system": 600, "tools": 400 }, "query": query, "idle_s": 5,
                "helpers": [] }),
    )
}

#[test]
fn an_acknowledgement_keeps_the_memory_slot_byte_for_byte() {
    let mut held = Held::seeded();
    held.asked();
    let first = memory_slot(&held.one("layout", window()));
    assert!(first.contains("QWNEWVALUE"), "{first}");
    for (cursor, said) in [(1, "ok"), (2, "go on"), (3, "Thanks!")] {
        let next = memory_slot(&said_next(&mut held, cursor, said, 0));
        assert_eq!(next, first, "{said:?} changed the slot");
    }
}

#[test]
fn an_acknowledgement_after_a_failure_looks_again() {
    let mut held = Held::seeded();
    held.asked();
    let first = memory_slot(&held.one("layout", window()));
    let failed = json!({ "cursor": 1, "role": "tool", "kind": "tool_result", "text": "boom",
                         "error": true });
    let reply = held.ask(&Door::Owner, "observe", vec![json!(SESSION), failed]);
    assert!(reply.ok, "{:?}", reply.error);
    let next = memory_slot(&said_next(&mut held, 2, "ok", 0));
    assert_ne!(next, first, "a failure can make something else relevant");
}

#[test]
fn a_short_request_is_not_an_acknowledgement() {
    let mut held = Held::seeded();
    held.asked();
    let first = memory_slot(&held.one("layout", window()));
    let next = memory_slot(&said_next(&mut held, 1, "fix the parser", 0));
    assert_ne!(
        next, first,
        "three words asking for something are a question"
    );
}
