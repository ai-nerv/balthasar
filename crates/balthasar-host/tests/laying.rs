//! `layout`, `applied` and `overflowed`, end to end through the door.

use balthasar_host::{Answering, Door, Hooks, answer_hooked};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{ScopeId, SessionId, floor};
use balthasar_store::{State, Store, Transcript};
use serde_json::{Value, json};

const NOW: balthasar_model::Timestamp = 1_756_000_000;
const SESSION: &str = "01LAY";

#[path = "laying/atomic.rs"]
mod atomic;

struct Plain;
impl Hooks for Plain {}

struct Harness {
    store: Store,
    scrollback: Transcript,
}

impl Harness {
    fn new() -> Self {
        Self {
            store: Store::ephemeral().expect("store"),
            scrollback: Transcript::ephemeral().expect("scrollback"),
        }
    }

    fn ask_with(&mut self, hooks: &mut dyn Hooks, call: &str, args: Vec<Value>) -> Reply {
        self.ask_through(&Door::Owner, hooks, call, args)
    }

    fn ask_through(
        &mut self,
        door: &Door,
        hooks: &mut dyn Hooks,
        call: &str,
        args: Vec<Value>,
    ) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: Some(&mut self.scrollback),
            scratch: None,
            scope: ScopeId::new("/w/thing"),
            agent: balthasar_model::AgentId::main(),
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

    fn ask(&mut self, call: &str, args: Vec<Value>) -> Reply {
        self.ask_with(&mut Plain, call, args)
    }

    fn one(&mut self, call: &str, second: Value) -> Value {
        let reply = self.ask(call, vec![json!(SESSION), second]);
        assert!(reply.ok, "{call}: {:?}", reply.error);
        reply.result.into_iter().next().unwrap_or(Value::Null)
    }

    fn observe(&mut self, turn: Value) {
        let reply = self.ask("observe", vec![json!(SESSION), turn]);
        assert!(reply.ok, "{:?}", reply.error);
    }

    /// One prompt: the user, the model calling a tool, and the tool's result.
    fn turn(&mut self, n: u64, result: u64) {
        let base = n * 3;
        self.observe(json!({ "cursor": base, "role": "user", "kind": "user",
                             "tokens": 40, "text": format!("prompt {n}") }));
        self.observe(
            json!({ "cursor": base + 1, "role": "assistant", "kind": "assistant",
                             "tokens": 100, "group": base + 1, "text": "running the tests" }),
        );
        self.observe(
            json!({ "cursor": base + 2, "role": "tool", "kind": "tool_result",
                             "tool": "shell", "tokens": result, "group": base + 1,
                             "text": "test output ".repeat(300),
                             "stub": format!("`cargo test` run {n} exited 101") }),
        );
    }

    fn rows(&self) -> Vec<balthasar_store::Turn> {
        self.scrollback
            .replay(&SessionId::new(SESSION))
            .expect("replay")
    }
}

/// A small window: 17,000 of room, 15,640 of it for the conversation when nothing else is held.
fn small() -> Value {
    json!({ "round": 0, "window": 20_000, "reply": 2_000, "fixed": { "system": 600, "tools": 400 },
            "query": "prompt", "idle_s": 5, "helpers": [] })
}

fn kinds(laid: &Value) -> Vec<String> {
    laid["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .map(|s| s["kind"].as_str().expect("a kind").to_owned())
        .collect()
}

fn named(laid: &Value) -> Vec<u64> {
    laid["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .filter_map(|s| s["cursor"].as_u64())
        .collect()
}

#[test]
fn a_layout_answers_in_the_contracts_shape() {
    let mut harness = Harness::new();
    let reply = harness.ask(
        "remember",
        vec![json!("the deploy target is fly.io"), json!({})],
    );
    assert!(reply.ok, "{:?}", reply.error);
    harness.turn(0, 3_000);
    harness.observe(
        json!({ "cursor": 3, "role": "user", "kind": "user", "tokens": 12,
                            "text": "where does the deploy target live?" }),
    );

    let laid = harness.one(
        "layout",
        json!({ "round": 0, "window": 200_000, "reply": 32_000,
                "fixed": { "system": 4_100, "tools": 6_900 }, "live": [0, 1, 2, 3],
                "query": "where does the deploy target live?", "idle_s": 12, "helpers": [] }),
    );
    println!("{}", serde_json::to_string_pretty(&laid).expect("json"));

    assert!(laid["id"].as_str().expect("an id").starts_with("L-"));
    let budget = &laid["budget"];
    assert_eq!(budget["room"], 157_000);
    assert_eq!(budget["memory"], 7_850);
    assert_eq!(budget["pinned"], 4_710);
    assert_eq!(budget["summary"], 12_560);
    assert_eq!(budget["factor"], 1.0);
    assert_eq!(named(&laid), [0, 1, 2, 3], "ascending, from live");
    assert_eq!(kinds(&laid), ["item", "item", "item", "memory", "item"]);
    let memory = &laid["slots"][3];
    assert!(memory["text"].as_str().expect("text").contains("fly.io"));
    assert_eq!(memory["ids"].as_array().expect("ids").len(), 1);
    assert_eq!(laid["jobs"], json!([]));
    assert_eq!(laid["fits"], true);
    assert!(
        laid["why"]
            .as_str()
            .expect("why")
            .contains("everything word for word")
    );
}

#[test]
fn nothing_is_recorded_until_the_layout_is_applied() {
    let mut harness = Harness::new();
    for n in 0..10 {
        harness.turn(n, 1_900);
    }
    let laid = harness.one("layout", small());
    let stubbed: Vec<u64> = laid["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .filter(|s| s["kind"] == "stub")
        .filter_map(|s| s["cursor"].as_u64())
        .collect();
    assert!(!stubbed.is_empty(), "{}", laid["why"]);
    let stub = laid["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["kind"] == "stub")
        .expect("a stub");
    assert!(
        stub["text"].as_str().expect("text").contains("exited 101"),
        "casper's words"
    );
    assert!(
        harness.rows().iter().all(|t| t.state == State::Live),
        "answering records nothing"
    );

    let id = laid["id"].clone();
    let estimated = laid["budget"]["estimated_input"]
        .as_u64()
        .expect("an estimate");
    harness.one(
        "applied",
        json!({ "id": id, "usage": { "input": estimated * 11 / 10,
                                                        "cache_read": 0, "cache_write": 0 } }),
    );
    let rows = harness.rows();
    for cursor in &stubbed {
        let row = rows.iter().find(|t| t.cursor == *cursor).expect("a row");
        assert_eq!(
            row.state,
            State::Masked,
            "cursor {cursor} is recorded as stubbed"
        );
    }
    let again = harness.one("layout", small_with(json!({ "round": 1 })));
    let factor = again["budget"]["factor"].as_f64().expect("a factor");
    assert!(
        (factor - 1.1).abs() < 0.01,
        "the first count sets the factor: {factor}"
    );
    // Applying twice changes nothing more.
    harness.one("applied", json!({ "id": id }));

    // Six times the estimate is an estimate that missed what was sent: nothing is learned from it.
    let off = harness.one("layout", small_with(json!({ "round": 2 })));
    let estimated = off["budget"]["estimated_input"]
        .as_u64()
        .expect("an estimate");
    harness.one(
        "applied",
        json!({ "id": off["id"].clone(), "usage": { "input": estimated * 6,
                                                        "cache_read": 0, "cache_write": 0 } }),
    );
    let after = harness.one("layout", small_with(json!({ "round": 3 })));
    let kept = after["budget"]["factor"].as_f64().expect("a factor");
    assert!(
        (kept - 1.1).abs() < 0.01,
        "an implausible count moved the factor: {kept}"
    );
}

/// `small()` with some fields replaced.
fn small_with(extra: Value) -> Value {
    let mut asked = small();
    for (name, value) in extra.as_object().expect("an object") {
        asked[name] = value.clone();
    }
    asked
}

#[test]
fn applying_a_layout_nobody_proposed_is_refused() {
    let mut harness = Harness::new();
    harness.turn(0, 100);
    let reply = harness.ask("applied", vec![json!(SESSION), json!({ "id": "L-999" })]);
    assert!(!reply.ok);
}

#[test]
fn an_overflow_answers_a_tighter_layout() {
    let mut harness = Harness::new();
    for n in 0..8 {
        harness.turn(n, 1_900);
    }
    let first = harness.one("layout", small());
    let said = "prompt is too long: 23456 tokens > 20000 maximum";
    let tighter = harness.one("overflowed", json!({ "id": first["id"], "said": said }));
    assert_ne!(tighter["id"], first["id"]);
    assert!(tighter["budget"]["factor"].as_f64().expect("a factor") > 1.0);
    let raw = |laid: &Value| {
        laid["budget"]["used"].as_f64().expect("used")
            / laid["budget"]["factor"].as_f64().expect("f")
    };
    assert!(
        raw(&tighter) < raw(&first),
        "{} then {}",
        first["why"],
        tighter["why"]
    );
    assert!(tighter["why"].as_str().expect("why").contains("tightened"));
}

#[test]
fn a_stored_summary_stands_in_for_what_it_covers() {
    let mut harness = Harness::new();
    for n in 0..4 {
        harness.turn(n, 200);
    }
    harness
        .scrollback
        .keep_summary(&SessionId::new(SESSION), 0, 5, "Tests were run twice.", NOW)
        .expect("a summary");
    let laid = harness.one("layout", small());
    let summary = laid["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["kind"] == "summary")
        .expect("a summary slot");
    assert_eq!(summary["covers"], json!([0, 5]));
    assert!(named(&laid).iter().all(|c| *c > 5));
    assert_eq!(kinds(&laid)[0], "summary");
}

#[test]
fn memory_is_settled_once_per_prompt() {
    let mut harness = Harness::new();
    assert!(
        harness
            .ask(
                "remember",
                vec![json!("the prompt style is terse"), json!({})]
            )
            .ok
    );
    harness.turn(0, 100);
    let first = harness.one("layout", small());
    let french = json!("a prompt must be answered in French");
    assert!(harness.ask("remember", vec![french, json!({})]).ok);
    let same = harness.one("layout", small_with(json!({ "round": 1 })));
    let memory = |laid: &Value| {
        laid["slots"]
            .as_array()
            .expect("slots")
            .iter()
            .find(|s| s["kind"] == "memory")
            .map(|s| s["text"].clone())
    };
    assert!(memory(&first).is_some());
    assert_eq!(
        memory(&first),
        memory(&same),
        "stable for every round of a prompt"
    );
    harness.turn(1, 100);
    let next = harness.one("layout", small());
    assert!(
        memory(&next)
            .expect("memory")
            .as_str()
            .expect("text")
            .contains("French"),
        "a new prompt searches again"
    );
}

#[test]
fn only_what_the_harness_would_send_is_named() {
    let mut harness = Harness::new();
    for n in 0..3 {
        harness.turn(n, 100);
    }
    let laid = harness.one("layout", small_with(json!({ "live": [0, 1, 2, 6, 7, 8] })));
    assert_eq!(named(&laid), [0, 1, 2, 6, 7, 8]);
}

#[test]
fn a_long_conversation_is_warned_before_the_latest_prompt() {
    let mut harness = Harness::new();
    // 12,960 of 15,640: over warn_at, under compact_at, and every result too small to stub.
    for n in 0..9 {
        harness.turn(n, 1_300);
    }
    let laid = harness.one("layout", small());
    let kinds = kinds(&laid);
    let note = kinds.iter().position(|k| k == "note").expect("a note");
    assert_eq!(laid["slots"][note + 1]["cursor"], 24, "{kinds:?}");
}

struct Policy(Value);
impl Hooks for Policy {
    fn policy(&mut self, _budget: &Value, _items: &Value) -> Option<Value> {
        Some(self.0.clone())
    }
}

#[test]
fn a_configured_policy_lays_out_instead_when_it_keeps_the_rules() {
    let mut harness = Harness::new();
    for n in 0..2 {
        harness.turn(n, 100);
    }
    let mut keeps = Policy(json!({ "why": "only the latest prompt", "slots": [
        { "kind": "item", "cursor": 3 }, { "kind": "item", "cursor": 4 },
        { "kind": "stub", "cursor": 5, "text": "elided by policy" } ] }));
    let reply = harness.ask_with(&mut keeps, "layout", vec![json!(SESSION), small()]);
    let laid = reply.result.into_iter().next().expect("a layout");
    assert_eq!(laid["why"], "only the latest prompt");
    assert_eq!(named(&laid), [3, 4, 5]);

    let mut splits = Policy(json!({ "slots": [{ "kind": "item", "cursor": 4 }] }));
    let reply = harness.ask_with(&mut splits, "layout", vec![json!(SESSION), small()]);
    let laid = reply.result.into_iter().next().expect("a layout");
    assert!(laid["why"].as_str().expect("why").contains("unusable"));
    assert_eq!(named(&laid), [0, 1, 2, 3, 4, 5]);
}

#[test]
fn a_layout_for_a_session_nobody_observed_says_so() {
    let mut harness = Harness::new();
    let reply = harness.ask("layout", vec![json!("never"), small()]);
    assert!(!reply.ok);
    assert!(reply.error.expect("a reason").contains("stream turns"));
}

#[test]
fn a_rule_a_session_proposed_to_itself_is_not_laid_into_the_next_prompt() {
    let session = Door::Socket(balthasar_ipc::Peer {
        pid: 4021,
        uid: 1000,
        program: Some("harness".to_owned()),
    });
    let mut harness = Harness::new();
    for text in [
        "Convention: every function name must start with the prefix zq_.",
        "The zq_ storage module keeps its index in index.db.",
    ] {
        let kept = harness.ask_through(&session, &mut Plain, "remember", vec![json!(text)]);
        assert!(kept.ok, "{:?}", kept.error);
    }
    harness.turn(0, 100);
    let laid = harness.one(
        "layout",
        small_with(json!({ "query": "the zq_ prefix and the storage index" })),
    );
    let memory = laid["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["kind"] == "memory")
        .map(|s| s["text"].as_str().unwrap_or_default().to_owned())
        .unwrap_or_default();
    assert!(
        memory.contains("index.db"),
        "a doubtful fact is still offered: {memory}"
    );
    assert!(
        !memory.contains("must start"),
        "a doubtful rule is not: {memory}"
    );
    // Nor handed to the helper that chooses reminders, which would rewrite it without its doubt.
    harness.turn(1, 100);
    let curating = harness.one(
        "layout",
        small_with(json!({ "query": "the zq_ prefix and the storage index",
                           "helpers": ["memory"] })),
    );
    let jobs = serde_json::to_string(&curating["jobs"]).expect("json");
    assert!(
        jobs.contains("index.db"),
        "the helper is given the fact: {jobs}"
    );
    assert!(!jobs.contains("must start"), "and not the rule: {jobs}");
}

#[test]
fn a_helper_rewriting_a_doubted_memory_does_not_rewrite_the_doubt_away() {
    let session = Door::Socket(balthasar_ipc::Peer {
        pid: 4021,
        uid: 1000,
        program: Some("harness".to_owned()),
    });
    let mut harness = Harness::new();
    let fact = "The zq_ storage module keeps its index in index.db.";
    let kept = harness.ask_through(&session, &mut Plain, "remember", vec![json!(fact)]);
    assert!(kept.ok, "{:?}", kept.error);
    harness.turn(0, 100);
    let asked = || small_with(json!({ "query": "the storage index", "helpers": ["memory"] }));
    let first = harness.one("layout", asked());
    let job = first["jobs"]
        .as_array()
        .expect("jobs")
        .iter()
        .find(|j| j["kind"] == "curate")
        .expect("a curate job")
        .clone();
    let id = kept.result[0]["id"].clone();
    let said = json!({ "chosen": [id], "notes": ["REWRITTEN: the index is index.db."] });
    harness.one(
        "job_done",
        json!({ "id": job["id"], "text": said.to_string() }),
    );
    let again = harness.one("layout", asked());
    let memory = again["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["kind"] == "memory")
        .map(|s| s["text"].as_str().unwrap_or_default().to_owned())
        .expect("a memory slot");
    assert!(memory.contains("Also on record"), "{memory}");
    assert!(memory.contains(fact), "{memory}");
    assert!(!memory.contains("REWRITTEN"), "{memory}");
}
