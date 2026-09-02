//! The scrollback, as a harness with no journal of its own would use it.

use memo_host::{Answering, Door, answer};
use memo_ipc::{Reply, Request};
use memo_model::{ScopeId, floor};
use memo_store::{Store, Transcript};

const NOW: memo_model::Timestamp = 1_756_000_000;
const S: &str = "01RUN";

struct Held {
    store: Store,
    scrollback: Transcript,
}

impl Held {
    fn new() -> Self {
        Self {
            store: Store::ephemeral().expect("store"),
            scrollback: Transcript::ephemeral().expect("scrollback"),
        }
    }

    fn ask(&mut self, name: &str, args: Vec<serde_json::Value>) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: Some(&mut self.scrollback),
            scratch: None,
            scope: ScopeId::new("/w/thing"),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        };
        answer(
            &mut at,
            &Door::Owner,
            &Request {
                call: name.to_owned(),
                args,
            },
        )
    }

    fn value(reply: &Reply) -> serde_json::Value {
        reply
            .result
            .as_ref()
            .and_then(|r| r.first())
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }
}

/// A turn in the shape a harness sends, carrying its own record.
fn turn(cursor: u64, role: &str, text: &str, raw: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "cursor": cursor, "role": role, "kind": "prose",
        "text": text, "at": NOW, "raw": raw,
    })
}

#[test]
fn what_a_harness_wrote_comes_back_byte_for_byte() {
    // The whole claim. memo stores the record and never parses it, so a harness gets back
    // exactly what it wrote in whatever shape it uses.
    let mut held = Held::new();
    let raw = serde_json::json!({
        "type": "tool", "id": "t1", "name": "shell",
        "args": "{\"command\":\"make test\"}",
        "result": { "output": "ok", "is_error": false }
    });
    held.ask(
        "observe",
        vec![serde_json::json!(S), turn(0, "tool", "ok", raw.clone())],
    );

    let back = Held::value(&held.ask("replay", vec![serde_json::json!(S)]));
    let turns = back.as_array().expect("a list");
    assert_eq!(turns.len(), 1);
    let stored: serde_json::Value =
        serde_json::from_str(turns[0]["raw"].as_str().expect("raw")).expect("json");
    assert_eq!(stored, raw, "the harness's own record, unchanged");
}

#[test]
fn the_same_turn_twice_is_two_turns() {
    // The memory store deduplicates within a session. A transcript must not.
    let mut held = Held::new();
    for cursor in 0..2 {
        held.ask(
            "observe",
            vec![
                serde_json::json!(S),
                turn(
                    cursor,
                    "user",
                    "carry on",
                    serde_json::json!({ "type": "user" }),
                ),
            ],
        );
    }
    let back = Held::value(&held.ask("replay", vec![serde_json::json!(S)]));
    assert_eq!(back.as_array().expect("a list").len(), 2);
}

#[test]
fn a_tool_call_is_revised_where_it_stands() {
    // How a harness actually writes one: the call when it is made, the result when it arrives.
    let mut held = Held::new();
    held.ask(
        "observe",
        vec![
            serde_json::json!(S),
            turn(
                4,
                "tool",
                "",
                serde_json::json!({ "type": "tool", "result": null }),
            ),
        ],
    );
    let reply = held.ask(
        "amend",
        vec![
            serde_json::json!(S),
            turn(
                4,
                "tool",
                "ok",
                serde_json::json!({ "type": "tool", "result": { "output": "ok" } }),
            ),
        ],
    );
    assert!(reply.ok, "{:?}", reply.error);

    let back = Held::value(&held.ask("replay", vec![serde_json::json!(S)]));
    let turns = back.as_array().expect("a list");
    assert_eq!(turns.len(), 1, "a revision is not a second turn");
    assert_eq!(turns[0]["revisions"], serde_json::json!(1));
    assert!(
        turns[0]["raw"]
            .as_str()
            .expect("raw")
            .contains("\"output\":\"ok\"")
    );
}

#[test]
fn a_restarting_harness_is_told_where_it_was() {
    // With no journal of its own it has no other way to know, and guessing wrong overwrites a
    // turn nothing else holds a copy of.
    let mut held = Held::new();
    let fresh = Held::value(&held.ask("resume", vec![serde_json::json!(S)]));
    assert_eq!(fresh["next"], serde_json::json!(0));

    for cursor in 0..3 {
        held.ask(
            "observe",
            vec![
                serde_json::json!(S),
                turn(
                    cursor,
                    "user",
                    "a turn",
                    serde_json::json!({ "type": "user" }),
                ),
            ],
        );
    }
    let after = Held::value(&held.ask("resume", vec![serde_json::json!(S)]));
    assert_eq!(after["next"], serde_json::json!(3));
    assert_eq!(after["turns"], serde_json::json!(3));
}

#[test]
fn why_quotes_the_turn_a_witness_saw() {
    // The reason the scrollback earns its place in `why`: naming a cursor at somebody is not
    // the same as showing them what was said.
    let mut held = Held::new();
    held.ask(
        "observe",
        vec![
            serde_json::json!(S),
            turn(
                7,
                "user",
                "remember: we deploy to fly.io",
                serde_json::json!({ "type": "user" }),
            ),
        ],
    );

    // A memory whose witness points at that cursor.
    let mut memory = memo_model::Memory::new(
        memo_store::mint(NOW),
        memo_model::Tier::Fact,
        ScopeId::new("/w/thing"),
        memo_model::Body::note("we deploy to fly.io", memo_model::NoteKind::Claim),
        NOW,
    );
    memory.session = Some(memo_model::SessionId::new(S));
    let witness = memo_model::Witness::new(
        memo_model::WitnessId::new("w1"),
        memo_model::WitnessKind::Imperative,
        memo_model::SessionId::new(S),
        ScopeId::new("/w/thing"),
        NOW,
    )
    .at_cursor(7);
    let landing = held.store.remember(memory, witness, NOW).expect("remember");
    let id = landing.id().to_string();

    let why = Held::value(&held.ask("why", vec![serde_json::json!(id)]));
    let quoted = why["quoted"].as_array().expect("quoted");
    assert_eq!(quoted.len(), 1);
    assert_eq!(quoted[0]["cursor"], serde_json::json!(7));
    assert_eq!(
        quoted[0]["said"],
        serde_json::json!("remember: we deploy to fly.io")
    );
}

#[test]
fn a_host_with_no_scrollback_says_so_rather_than_pretending() {
    // A harness relying on memo for persistence must find out immediately, not discover on
    // restore that nothing was kept.
    let mut store = Store::ephemeral().expect("store");
    let mut at = Answering {
        store: &mut store,
        scrollback: None,
        scratch: None,
        scope: ScopeId::new("/w/thing"),
        now: NOW,
        inject_floor: floor::INJECT,
        live_floor: floor::LIVE,
        capture: false,
    };
    for verb in ["replay", "resume", "amend"] {
        let reply = answer(
            &mut at,
            &Door::Owner,
            &Request {
                call: verb.to_owned(),
                args: vec![serde_json::json!(S), serde_json::json!({})],
            },
        );
        assert!(!reply.ok, "'{verb}' must refuse rather than answer emptily");
        assert!(
            reply.error.expect("a reason").contains("scrollback"),
            "and say why"
        );
    }
}

#[test]
fn two_runs_keep_separate_scrollbacks() {
    let mut held = Held::new();
    for (session, text) in [("a", "one"), ("b", "two")] {
        held.ask(
            "observe",
            vec![
                serde_json::json!(session),
                turn(0, "user", text, serde_json::json!({ "type": "user" })),
            ],
        );
    }
    let a = Held::value(&held.ask("replay", vec![serde_json::json!("a")]));
    assert_eq!(
        a.as_array().expect("a list")[0]["text"],
        serde_json::json!("one")
    );
}
