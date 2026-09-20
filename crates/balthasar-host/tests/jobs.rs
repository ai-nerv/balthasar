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
    assert_eq!(job["role"], "summary");
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
            .starts_with("Today is "),
        "a summary told to write absolute dates is told the date"
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

    // A blocking job shaped one request, which has gone without it: never handed out again.
    let _ = harness.ask(
        "remember",
        vec![json!("the deploy target is fly.io"), json!({})],
    );
    harness.talk(8, "how do we deploy?");
    let curating = harness.one("layout", small(0, json!(["notes", "curate"])));
    let blocking = curating["jobs"]
        .as_array()
        .expect("jobs")
        .iter()
        .find(|j| j["blocking"] == true)
        .map(|j| j["id"].clone());
    if let Some(blocking) = blocking {
        harness.one("job_done", json!({ "id": blocking, "failed": "too slow" }));
        assert!(
            harness.rows("jobs").iter().all(|j| j["id"] != blocking),
            "a blocking job came round again"
        );
    }
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
    let laid = harness.one("layout", small(0, json!(["notes", "curate"])));
    let ids: Vec<&str> = laid["jobs"]
        .as_array()
        .expect("jobs")
        .iter()
        .map(|j| j["id"].as_str().expect("an id"))
        .collect();
    assert_eq!(
        ids.len(),
        3,
        "a summary, notes before its cut, and a curate"
    );
    let mut distinct = ids.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), ids.len(), "{ids:?}");
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
    let first = harness.one("layout", ask(0, json!(["notes", "curate"])));
    let job = first["jobs"]
        .as_array()
        .expect("jobs")
        .iter()
        .find(|j| j["kind"] == "curate")
        .expect("a curate job")
        .clone();
    assert_eq!(job["blocking"], true);
    assert_eq!(job["timeout_ms"], 5_000);
    assert_eq!(job["fallback"], "skip");
    assert_eq!(job["schema"]["required"], json!(["chosen", "notes"]));
    let ids = slot(&first, "memory").expect("plain ranking meanwhile")["ids"].clone();
    let chosen = ids[0].as_str().expect("an id").to_owned();

    let said = json!({ "chosen": [chosen], "notes": ["Deploy with `fly deploy --remote-only`."] });
    harness.one(
        "job_done",
        json!({ "id": job["id"], "text": said.to_string() }),
    );
    let again = harness.one("layout", ask(0, json!(["notes", "curate"])));
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

#[test]
fn a_long_answer_reaches_its_summary_whole() {
    // A chapter's payoff is at its end: a summary that read only its first few thousand
    // characters kept the cast and lost the story.
    let mut harness = Harness::new();
    let chapter = format!(
        "{}the lamp was lit by Isolde at last.",
        "The sea kept its counsel. ".repeat(350)
    );
    for n in 0..8 {
        harness.observe(
            json!({ "cursor": n * 2, "role": "user", "kind": "user", "tokens": 40,
                                "text": "carry on" }),
        );
        harness.observe(
            json!({ "cursor": n * 2 + 1, "role": "assistant", "kind": "assistant",
                                "tokens": 2_000, "group": n * 2 + 1, "text": chapter }),
        );
    }
    let laid = harness.one("layout", small(0, json!([])));
    let job = laid["jobs"]
        .as_array()
        .expect("jobs")
        .iter()
        .find(|j| j["kind"] == "summarise")
        .expect("a summary job")
        .clone();
    assert!(
        job["input"]
            .as_str()
            .expect("text")
            .contains("the lamp was lit by Isolde at last."),
        "the end of a long answer was cut"
    );
    assert_eq!(job["timeout_ms"], 60_000);
}

#[test]
fn a_summary_carries_the_calls_that_failed_in_what_it_covers() {
    let mut harness = Harness::new();
    // A call that failed, early enough to be summarised away.
    harness.observe(
        json!({ "cursor": 0, "role": "user", "kind": "user", "tokens": 40,
                            "text": "read the parser" }),
    );
    harness.observe(
        json!({ "cursor": 1, "role": "assistant", "kind": "assistant",
                            "tokens": 20, "group": 1, "text": "" }),
    );
    harness.observe(
        json!({ "cursor": 2, "role": "tool", "kind": "tool_result", "tool": "read",
                            "tokens": 20, "group": 1, "error": true,
                            "args": "{\"path\":\"missing.rs\"}",
                            "text": "no such file: missing.rs" }),
    );
    for n in 2..10 {
        harness.talk(n, "carry on");
    }
    let laid = harness.one("layout", small(0, json!([])));
    let job = laid["jobs"]
        .as_array()
        .expect("jobs")
        .iter()
        .find(|j| j["kind"] == "summarise")
        .expect("a summarise job")
        .clone();
    assert!(
        job["covers"][0].as_u64().expect("from") <= 2,
        "{}",
        job["covers"]
    );
    assert!(
        job["covers"][1].as_u64().expect("to") >= 2,
        "{}",
        job["covers"]
    );

    // A helper that says nothing of it, as one is free to.
    harness.one(
        "job_done",
        json!({ "id": job["id"], "text": "Goal: carry on. State: the parser is in src/parse.rs." }),
    );
    let kept = harness
        .scrollback
        .summary(&balthasar_model::SessionId::new(SESSION))
        .expect("read")
        .expect("a summary");
    assert!(kept.text.starts_with("Goal: carry on."), "{}", kept.text);
    for part in [
        "Calls that failed",
        "[2] read",
        "missing.rs",
        "no such file",
    ] {
        assert!(kept.text.contains(part), "{part}: {}", kept.text);
    }
    // Once, however often the helper copies the last summary into the next.
    assert_eq!(kept.text.matches("Calls that failed").count(), 1);
    let sent = harness.one("layout", small(1, json!([])));
    let summary = slot(&sent, "summary").expect("a summary slot");
    assert!(
        summary["text"]
            .as_str()
            .expect("text")
            .contains("missing.rs")
    );

    // Rolled forward by a helper that copies the last summary whole, failures and all.
    for n in 10..18 {
        harness.talk(n, "carry on");
    }
    let later = harness.one("layout", small(0, json!([])));
    let next = later["jobs"]
        .as_array()
        .expect("jobs")
        .iter()
        .find(|j| j["kind"] == "summarise")
        .expect("a second summarise job")
        .clone();
    let copied = format!("{}\n\nAnd then more was carried on.", kept.text);
    harness.one("job_done", json!({ "id": next["id"], "text": copied }));
    let rolled = harness
        .scrollback
        .summary(&balthasar_model::SessionId::new(SESSION))
        .expect("read")
        .expect("a summary");
    assert!(rolled.to > kept.to, "{} then {}", kept.to, rolled.to);
    assert_eq!(
        rolled.text.matches("Calls that failed").count(),
        1,
        "{}",
        rolled.text
    );
    assert!(rolled.text.contains("[2] read"), "{}", rolled.text);
    assert!(
        rolled.text.contains("And then more was carried on."),
        "{}",
        rolled.text
    );
}
