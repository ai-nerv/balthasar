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
            capture: false,
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
