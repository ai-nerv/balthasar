//! A layout is never held up by one bad effect, a peer's recall is redacted like the memory slot,
//! and a window that cannot hold the reply is refused rather than given no room.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_host::{Answering, Door, Hooks, answer_hooked};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{
    AgentId, Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId,
    WitnessKind, floor,
};
use balthasar_store::{LayoutEffect, Store, Transcript, mint};
use serde_json::{Value, json};

const NOW: Timestamp = 1_756_000_000;
const SESSION: &str = "01ROBUST";
const SCOPE: &str = "/w/thing";

struct Plain;
impl Hooks for Plain {}

/// Rewrites every memory a peer is handed.
struct Masking;
impl Hooks for Masking {
    fn redact(&mut self, _text: &str, _memory: &Memory) -> Option<String> {
        Some("[redacted]".to_owned())
    }
}

fn ask(
    store: &mut Store,
    scrollback: &mut Transcript,
    door: &Door,
    hooks: &mut dyn Hooks,
    call: &str,
    args: Vec<Value>,
) -> Reply {
    let mut at = Answering {
        store,
        scrollback: Some(scrollback),
        scratch: None,
        scope: ScopeId::new(SCOPE),
        agent: AgentId::main(),
        now: NOW,
        inject_floor: floor::INJECT,
        live_floor: floor::LIVE,
        capture: false,
    };
    let request = Request {
        call: call.into(),
        args,
    };
    answer_hooked(&mut at, door, &request, hooks)
}

fn peer() -> Door {
    Door::Socket(balthasar_ipc::Peer {
        pid: 4242,
        uid: 1000,
        program: Some("/usr/bin/some-harness".to_owned()),
        agent: None,
    })
}

fn turn() -> Value {
    json!({ "cursor": 0, "role": "user", "kind": "user", "tokens": 20, "text": "the codec" })
}

#[test]
fn an_effect_that_cannot_be_applied_is_set_aside_and_the_layout_goes_on() {
    let mut store = Store::ephemeral().expect("store");
    let mut scrollback = Transcript::ephemeral().expect("scrollback");
    let observed = ask(
        &mut store,
        &mut scrollback,
        &Door::Owner,
        &mut Plain,
        "observe",
        vec![json!(SESSION), turn()],
    );
    assert!(observed.ok, "{:?}", observed.error);
    scrollback
        .queue_layout_effect(&LayoutEffect {
            layout: "L-x".into(),
            session: SessionId::new(SESSION),
            run: SessionId::new("another-run"),
            agent: AgentId::main(),
            scope: ScopeId::new(SCOPE),
            cursor: 0,
            text: "the codec".into(),
            at: NOW,
        })
        .expect("queued");
    let laid = ask(
        &mut store,
        &mut scrollback,
        &Door::Owner,
        &mut Plain,
        "layout",
        vec![
            json!(SESSION),
            json!({ "round": 0, "window": 200_000, "reply": 8_000,
            "fixed": { "system": 100, "tools": 100 }, "query": "codec", "helpers": [] }),
        ],
    );
    assert!(laid.ok, "{:?}", laid.error);
    assert_eq!(scrollback.failed_layout_effects().expect("count"), 1);
}

#[test]
fn a_peer_recall_passes_through_redaction() {
    let mut store = Store::ephemeral().expect("store");
    let mut scrollback = Transcript::ephemeral().expect("scrollback");
    let memory = Memory::new(
        mint(NOW),
        Tier::Fact,
        ScopeId::new(SCOPE),
        Body::note("the codec is zstd", NoteKind::Claim),
        NOW,
    );
    let witness = Witness::new(
        WitnessId::new("w-1"),
        WitnessKind::Imperative,
        SessionId::new("s1"),
        ScopeId::new(SCOPE),
        NOW,
    );
    store.remember(memory, witness, NOW).expect("remembered");
    let reply = ask(
        &mut store,
        &mut scrollback,
        &peer(),
        &mut Masking,
        "recall",
        vec![json!("codec"), json!({ "limit": 5 })],
    );
    assert!(reply.ok, "{:?}", reply.error);
    let text = serde_json::to_string(&reply.result).expect("json");
    assert!(
        text.contains("[redacted]") && !text.contains("zstd"),
        "{text}"
    );
}

#[test]
fn a_window_that_cannot_hold_the_reply_is_refused() {
    let mut store = Store::ephemeral().expect("store");
    let mut scrollback = Transcript::ephemeral().expect("scrollback");
    let observed = ask(
        &mut store,
        &mut scrollback,
        &Door::Owner,
        &mut Plain,
        "observe",
        vec![json!(SESSION), turn()],
    );
    assert!(observed.ok, "{:?}", observed.error);
    let laid = ask(
        &mut store,
        &mut scrollback,
        &Door::Owner,
        &mut Plain,
        "layout",
        vec![
            json!(SESSION),
            json!({ "round": 0, "window": 1_000, "reply": 2_000,
            "fixed": { "system": 10, "tools": 10 }, "query": "codec", "helpers": [] }),
        ],
    );
    assert!(!laid.ok, "refused rather than given no room");
    assert!(
        laid.error
            .as_deref()
            .is_some_and(|e| e.contains("no request fits")),
        "{:?}",
        laid.error
    );
}
