//! Jobs: summaries and curated memory, asked of the harness and settled by its answers.

use balthasar_host::{Answering, Door, Hooks, answer_hooked};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{ScopeId, floor};
use balthasar_store::{Store, Transcript};
use serde_json::{Value, json};

const NOW: balthasar_model::Timestamp = 1_756_000_000;
const SESSION: &str = "01JOB";

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

    fn ask(&mut self, call: &str, args: Vec<Value>) -> Reply {
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
        answer_hooked(&mut at, &Door::Owner, &request, &mut Plain)
    }

    fn one(&mut self, call: &str, second: Value) -> Value {
        let reply = self.ask(call, vec![json!(SESSION), second]);
        assert!(reply.ok, "{call}: {:?}", reply.error);
        reply.result.into_iter().next().unwrap_or(Value::Null)
    }

    fn rows(&mut self, call: &str) -> Vec<Value> {
        let reply = self.ask(call, vec![json!(SESSION), json!({})]);
        assert!(reply.ok, "{call}: {:?}", reply.error);
        reply.result
    }

    fn observe(&mut self, turn: Value) {
        let reply = self.ask("observe", vec![json!(SESSION), turn]);
        assert!(reply.ok, "{:?}", reply.error);
    }

    /// A prompt and a long answer: 2,040 tokens, nothing a stub could shrink.
    fn talk(&mut self, n: u64, prompt: &str) {
        self.observe(
            json!({ "cursor": n * 2, "role": "user", "kind": "user", "tokens": 40,
                             "text": prompt }),
        );
        self.observe(
            json!({ "cursor": n * 2 + 1, "role": "assistant", "kind": "assistant",
                             "tokens": 2_000, "group": n * 2 + 1,
                             "text": format!("answer {n}: the parser lives in src/parse.rs") }),
        );
    }
}

/// 17,000 of room; a summary is wanted past 13,294 of conversation.
fn small(round: u64, helpers: Value) -> Value {
    json!({ "round": round, "window": 20_000, "reply": 2_000, "fixed": { "system": 1_000 },
            "query": "carry on", "helpers": helpers })
}

fn slot<'a>(laid: &'a Value, kind: &str) -> Option<&'a Value> {
    laid["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["kind"] == kind)
}

#[test]
fn a_summary_is_asked_for_once_and_its_answer_stands_in_for_the_span() {
    let mut harness = Harness::new();
    for n in 0..8 {
        harness.talk(n, "carry on");
    }
    let laid = harness.one("layout", small(0, json!([])));
    let jobs = laid["jobs"].as_array().expect("jobs");
    assert_eq!(jobs.len(), 1, "{}", laid["why"]);
    let job = &jobs[0];
    assert_eq!(job["kind"], "summarise");
    assert_eq!(job["role"], "memory");
    assert_eq!(job["fallback"], "main");
    assert_eq!(job["blocking"], false);
    assert!(
        job["instruction"]
            .as_str()
            .expect("text")
            .contains("Lookup keywords")
    );
    assert!(
        job["input"]
            .as_str()
            .expect("text")
            .contains("[1] assistant: answer 0")
    );
    let to = job["covers"][1].as_u64().expect("a span");

    let again = harness.one("layout", small(1, json!([])));
    assert_eq!(
        again["jobs"],
        json!([]),
        "not asked twice while it is on its way"
    );
    assert!(
        again["why"]
            .as_str()
            .expect("why")
            .contains("a summary is on its way")
    );

    harness.one(
        "job_done",
        json!({ "id": job["id"], "text": "They talked about the parser.",
                                     "usage": { "input": 900, "output": 40 }, "model": "m" }),
    );
    let summarised = harness.one("layout", small(2, json!([])));
    let summary = slot(&summarised, "summary").expect("a summary slot");
    assert_eq!(summary["covers"], json!([0, to]));
    assert_eq!(summary["text"], "They talked about the parser.");
    assert!(
        summarised["slots"]
            .as_array()
            .expect("slots")
            .iter()
            .filter_map(|s| s["cursor"].as_u64())
            .all(|c| c > to)
    );

    // The next one rolls forward: it covers from the start and folds the old summary in.
    for n in 8..16 {
        harness.talk(n, "keep going");
    }
    let later = harness.one("layout", small(0, json!([])));
    let job = &later["jobs"][0];
    assert_eq!(job["covers"][0], 0);
    assert!(
        job["input"]
            .as_str()
            .expect("text")
            .contains("Previous summary (turns 0–")
    );
}

#[test]
fn a_failed_job_is_tried_once_more_and_then_left() {
    let mut harness = Harness::new();
    for n in 0..8 {
        harness.talk(n, "carry on");
    }
    let laid = harness.one("layout", small(0, json!([])));
    let id = laid["jobs"][0]["id"].clone();
    harness.one(
        "job_done",
        json!({ "id": id, "failed": "no helper answered" }),
    );
    let retried = harness.rows("jobs");
    assert_eq!(retried.len(), 1);
    assert_eq!(retried[0]["id"], id, "the same job, handed out again");
    harness.one("job_done", json!({ "id": id, "failed": "still nothing" }));
    assert!(harness.rows("jobs").is_empty(), "twice is enough");
    // A late answer to a failed job changes nothing.
    harness.one("job_done", json!({ "id": id, "text": "too late" }));
    assert!(
        harness
            .scrollback
            .summary(&balthasar_model::SessionId::new(SESSION))
            .expect("read")
            .is_none()
    );
}

#[test]
fn every_job_has_its_own_id() {
    let mut harness = Harness::new();
    let reply = harness.ask(
        "remember",
        vec![json!("carry on means keep the plan"), json!({})],
    );
    assert!(reply.ok);
    for n in 0..8 {
        harness.talk(n, "carry on");
    }
    let laid = harness.one("layout", small(0, json!(["memory"])));
    let ids: Vec<&str> = laid["jobs"]
        .as_array()
        .expect("jobs")
        .iter()
        .map(|j| j["id"].as_str().expect("an id"))
        .collect();
    assert_eq!(ids.len(), 2, "a summary and a curate");
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn memory_is_curated_once_per_prompt_by_a_helper_that_can() {
    let mut harness = Harness::new();
    for fact in [
        "the deploy target is fly.io",
        "deploys run `fly deploy --remote-only`",
        "the deploy key lives in 1password",
    ] {
        assert!(harness.ask("remember", vec![json!(fact), json!({})]).ok);
    }
    harness.observe(
        json!({ "cursor": 0, "role": "user", "kind": "user", "tokens": 10,
                            "text": "how do we deploy?" }),
    );
    let ask = |round: u64, helpers: Value| {
        json!({ "round": round, "window": 200_000, "reply": 8_000, "query": "how do we deploy?",
                "helpers": helpers })
    };

    let plain = harness.one("layout", ask(0, json!([])));
    assert!(
        plain["jobs"].as_array().expect("jobs").is_empty(),
        "no helper, no curate"
    );

    harness.observe(
        json!({ "cursor": 1, "role": "user", "kind": "user", "tokens": 10,
                            "text": "how do we deploy?" }),
    );
    let first = harness.one("layout", ask(0, json!(["memory"])));
    let job = first["jobs"]
        .as_array()
        .expect("jobs")
        .iter()
        .find(|j| j["kind"] == "curate")
        .expect("a curate job")
        .clone();
    assert_eq!(job["blocking"], true);
    assert_eq!(job["timeout_ms"], 2_000);
    assert_eq!(job["fallback"], "skip");
    assert_eq!(job["schema"]["required"], json!(["chosen", "notes"]));
    let ids = slot(&first, "memory").expect("plain ranking meanwhile")["ids"].clone();
    let chosen = ids[0].as_str().expect("an id").to_owned();

    let said = json!({ "chosen": [chosen], "notes": ["Deploy with `fly deploy --remote-only`."] });
    harness.one(
        "job_done",
        json!({ "id": job["id"], "text": said.to_string() }),
    );
    let again = harness.one("layout", ask(0, json!(["memory"])));
    let memory = slot(&again, "memory").expect("a memory slot");
    assert!(
        memory["text"]
            .as_str()
            .expect("text")
            .contains("Deploy with `fly deploy")
    );
    assert_eq!(memory["ids"], json!([chosen]));
    assert!(
        again["jobs"]
            .as_array()
            .expect("jobs")
            .iter()
            .all(|j| j["kind"] != "curate"),
        "once per prompt"
    );
}
