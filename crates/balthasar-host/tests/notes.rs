//! Notes: extracted by a helper, pinned or deferred, logged, undone and reviewed.

use balthasar_host::{Answering, Door, Hooks, Keeping, answer_hooked};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{ScopeId, floor};
use balthasar_store::{Store, Transcript};
use serde_json::{Value, json};

const NOW: balthasar_model::Timestamp = 1_756_000_000;
const SESSION: &str = "01NOTE";

#[path = "notes/provenance.rs"]
mod provenance;

#[path = "notes/atomic.rs"]
mod atomic;

#[path = "notes/coverage.rs"]
mod coverage;

#[path = "notes/projection.rs"]
mod projection;

#[path = "notes/standing.rs"]
mod standing;

#[path = "notes/quoting.rs"]
mod quoting;

struct Keep(Keeping);
impl Hooks for Keep {
    fn memory(&self) -> Keeping {
        self.0.clone()
    }
}

struct Harness {
    store: Store,
    scrollback: Transcript,
    hooks: Keep,
}

impl Harness {
    fn new() -> Self {
        Self::keeping(Keeping::default())
    }

    fn keeping(keeping: Keeping) -> Self {
        Self {
            store: Store::ephemeral().expect("store"),
            scrollback: Transcript::ephemeral().expect("scrollback"),
            hooks: Keep(keeping),
        }
    }

    fn ask(&mut self, call: &str, second: Value) -> Reply {
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
            args: vec![json!(SESSION), second],
        };
        answer_hooked(&mut at, &Door::Owner, &request, &mut self.hooks)
    }

    fn one(&mut self, call: &str, second: Value) -> Value {
        let reply = self.ask(call, second);
        assert!(reply.ok, "{call}: {:?}", reply.error);
        reply.result.into_iter().next().unwrap_or(Value::Null)
    }

    fn rows(&mut self, call: &str, second: Value) -> Vec<Value> {
        let reply = self.ask(call, second);
        assert!(reply.ok, "{call}: {:?}", reply.error);
        reply.result
    }

    fn turn(&mut self, n: u64, said: &str) {
        for (cursor, role, text) in [(n * 2, "user", said), (n * 2 + 1, "assistant", "done")] {
            let reply = self.ask(
                "observe",
                json!({ "cursor": cursor, "role": role, "kind": role, "tokens": 20, "text": text }),
            );
            assert!(reply.ok);
        }
    }

    fn layout(&mut self, round: u64) -> Value {
        self.one(
            "layout",
            json!({ "round": round, "window": 200_000, "reply": 8_000, "query": "carry on",
                    "helpers": ["memory"] }),
        )
    }

    /// Answer the one job of `kind` in `jobs`.
    fn answer(&mut self, jobs: &Value, kind: &str, text: Value) {
        let job = jobs
            .as_array()
            .expect("jobs")
            .iter()
            .find(|j| j["kind"] == kind)
            .unwrap_or_else(|| panic!("no {kind} job in {jobs}"));
        self.one(
            "job_done",
            json!({ "id": job["id"], "text": text.to_string() }),
        );
    }
}

/// What a helper answers after being told "use uv, not pip".
fn learned() -> Value {
    json!({ "ops": [
        { "op": "add", "title": "Package manager", "text": "Use uv, not pip.", "pinned": true },
        { "op": "add", "title": "Parser",
          "text": "The parser lives in src/parse.rs.\nIt is hand-written." } ] })
}

#[test]
fn a_finished_turn_is_read_for_notes_by_the_checklist() {
    let mut harness = Harness::new();
    harness.turn(0, "use uv, not pip");
    harness.turn(1, "carry on");
    let laid = harness.layout(0);
    let job = laid["jobs"]
        .as_array()
        .expect("jobs")
        .iter()
        .find(|j| j["kind"] == "extract")
        .expect("an extract job")
        .clone();
    assert_eq!(
        job["covers"],
        json!([0, 2]),
        "this prompt and the turns before it"
    );
    assert_eq!(job["role"], "memory");
    assert_eq!(job["fallback"], "skip");
    assert_eq!(job["schema"]["required"], json!(["ops"]));
    let instruction = job["instruction"].as_str().expect("text");
    assert!(instruction.contains("mistakes and corrections"));
    assert!(instruction.contains("Write absolute dates"));
    assert!(
        job["input"]
            .as_str()
            .expect("text")
            .contains("Today is 2025-08-24.")
    );
    assert!(
        job["input"]
            .as_str()
            .expect("text")
            .contains("[0] person: use uv, not pip")
    );

    let again = harness.layout(1);
    assert!(again["jobs"].as_array().expect("jobs").is_empty());
    assert!(
        harness
            .rows("jobs", json!({}))
            .iter()
            .all(|j| j["kind"] != "extract" || j["covers"] != json!([0, 2]))
    );
}

#[test]
fn notes_are_pinned_or_listed_and_a_new_rule_reaches_the_open_prompt() {
    let mut harness = Harness::new();
    harness.turn(0, "use uv, not pip");
    harness.turn(1, "carry on");
    let laid = harness.layout(0);
    harness.answer(&laid["jobs"], "extract", learned());

    let notes = harness.one("notes", json!({}));
    assert_eq!(notes["pinned"][0]["text"], "Use uv, not pip.");
    assert_eq!(notes["deferred"][0]["title"], "Parser");
    assert_eq!(
        notes["deferred"][0]["description"], "The parser lives in src/parse.rs.",
        "a description nobody wrote is the first line"
    );
    let same_prompt = harness.layout(1);
    let rule = &same_prompt["slots"][0];
    assert_eq!(
        rule["kind"], "rules",
        "a rule pinned mid-prompt is worth one cache miss"
    );
    assert_eq!(
        harness.layout(2)["slots"][0]["text"],
        rule["text"],
        "and after it the slot holds still"
    );

    harness.turn(2, "next thing");
    let next = harness.layout(0);
    let pinned = &next["slots"][0];
    assert_eq!(pinned["kind"], "rules");
    let text = pinned["text"].as_str().expect("text");
    assert!(text.contains("- Use uv, not pip."), "{text}");
    assert!(!text.contains("Parser"), "{text}");
    let index = next["slots"]
        .as_array()
        .expect("slots")
        .iter()
        .find(|s| s["kind"] == "observations")
        .expect("observation index");
    let index = index["text"].as_str().expect("text");
    assert!(
        index.contains("N-2")
            && index.contains("Parser")
            && index.contains("The parser lives in src/parse.rs."),
        "{index}"
    );
    let open = harness.one("note_open", json!({ "id": "N-2" }));
    assert_eq!(
        open["text"],
        "The parser lives in src/parse.rs.\nIt is hand-written."
    );
    assert!(!harness.ask("note_open", json!({ "id": "N-99" })).ok);
}

#[test]
fn every_change_is_logged_and_one_can_be_undone() {
    let mut harness = Harness::new();
    harness.turn(0, "use uv, not pip");
    harness.turn(1, "carry on");
    let laid = harness.layout(0);
    harness.answer(&laid["jobs"], "extract", learned());

    let log = harness.rows("changes", json!({ "limit": 10 }));
    assert_eq!(log.len(), 2);
    assert_eq!(log[0]["id"], "C-2", "newest first");
    assert_eq!(log[1]["op"], "add");
    assert_eq!(log[1]["note"], "N-1");
    assert_eq!(log[1]["after"]["text"], "Use uv, not pip.");
    assert!(log[1]["by"].as_str().expect("by").starts_with("J-"));

    let undone = harness.one("undo", json!({ "change": "C-1" }));
    assert_eq!(undone["undone"], "C-1");
    assert!(
        harness.one("notes", json!({}))["pinned"]
            .as_array()
            .expect("pinned")
            .is_empty()
    );
    let log = harness.rows("changes", json!({}));
    assert_eq!(log[0]["op"], "revert");
    assert_eq!(log[0]["by"], "undo C-1");
    assert_eq!(log[2]["state"], "undone");
    assert!(
        !harness.ask("undo", json!({ "change": "C-1" })).ok,
        "undone once"
    );

    // Undoing the undo brings the note back.
    harness.one("undo", json!({ "change": log[0]["id"] }));
    assert_eq!(harness.one("notes", json!({}))["pinned"][0]["id"], "N-1");
}

#[test]
fn a_contradiction_is_fixed_where_it_is_and_stale_notes_retire() {
    let mut harness = Harness::new();
    harness.turn(0, "use uv, not pip");
    harness.turn(1, "carry on");
    let laid = harness.layout(0);
    harness.answer(&laid["jobs"], "extract", learned());
    harness.turn(2, "Package manager: Use poetry; uv was dropped.");
    let later = harness.layout(0);
    harness.answer(
        &later["jobs"],
        "extract",
        json!({ "ops": [
            { "op": "add", "title": "package manager", "text": "Use poetry; uv was dropped." },
            { "op": "retire", "id": "N-2" } ] }),
    );
    let one = harness.one("note_open", json!({ "id": "N-1" }));
    assert_eq!(
        one["text"], "Use poetry; uv was dropped.",
        "the same note, not a second"
    );
    assert_eq!(one["pinned"], true);
    assert!(harness.one("note_open", json!({ "id": "N-2" }))["retired"].is_number());
    assert_eq!(harness.one("notes", json!({}))["deferred"], json!([]));
}

#[test]
fn with_review_on_changes_wait_for_the_main_model() {
    let mut harness = Harness::keeping(Keeping {
        review: true,
        ..Keeping::default()
    });
    harness.turn(0, "use uv, not pip");
    harness.turn(1, "carry on");
    let laid = harness.layout(0);
    harness.answer(&laid["jobs"], "extract", learned());
    assert!(
        harness.one("notes", json!({}))["pinned"]
            .as_array()
            .expect("pinned")
            .is_empty()
    );

    let waiting = harness.rows("jobs", json!({}));
    let review = waiting
        .iter()
        .find(|j| j["kind"] == "review")
        .expect("a review job");
    assert_eq!(review["role"], "main");
    assert!(review["input"].as_str().expect("text").contains("C-1: add"));
    harness.one(
        "job_done",
        json!({ "id": review["id"], "text": json!({ "approve": ["C-1"], "reject": [] }).to_string() }),
    );
    assert_eq!(
        harness.one("notes", json!({}))["pinned"][0]["title"],
        "Package manager"
    );

    let rejected = harness.one("reject", json!({ "changes": ["C-2"] }));
    assert_eq!(rejected["rejected"], json!(["C-2"]));
    assert_eq!(harness.one("notes", json!({}))["deferred"], json!([]));
    let again = harness.one("approve", json!({ "changes": ["C-2"] }));
    assert_eq!(
        again["approved"],
        json!([]),
        "a rejected change stays rejected"
    );
}

#[test]
fn notes_are_tidied_every_so_many_extractions() {
    let mut harness = Harness::keeping(Keeping {
        tidy_every: 1,
        ..Keeping::default()
    });
    harness.turn(0, "use uv, not pip");
    harness.turn(1, "carry on");
    let laid = harness.layout(0);
    harness.answer(&laid["jobs"], "extract", learned());
    let waiting = harness.rows("jobs", json!({}));
    let tidy = waiting
        .iter()
        .find(|j| j["kind"] == "tidy")
        .expect("a tidy job");
    assert!(
        tidy["input"]
            .as_str()
            .expect("text")
            .contains("Pinned notes cost about")
    );
}

#[test]
fn what_a_summary_will_replace_is_read_for_notes_first() {
    let mut harness = Harness::new();
    for n in 0..8 {
        for (cursor, role, tokens) in [(n * 2, "user", 40), (n * 2 + 1, "assistant", 2_000)] {
            let reply = harness.ask(
                "observe",
                json!({ "cursor": cursor, "role": role, "kind": role, "tokens": tokens,
                        "text": format!("{role} {n}") }),
            );
            assert!(reply.ok);
        }
    }
    let laid = harness.one(
        "layout",
        json!({ "round": 1, "window": 20_000, "reply": 2_000, "fixed": 1_000,
                "query": "carry on", "helpers": ["memory"] }),
    );
    let jobs = laid["jobs"].as_array().expect("jobs");
    let summary = jobs
        .iter()
        .find(|j| j["kind"] == "summarise")
        .expect("a summary");
    let notes = jobs
        .iter()
        .find(|j| j["kind"] == "extract")
        .expect("notes before the cut");
    assert_eq!(notes["covers"][0], 0);
    assert_eq!(notes["covers"][1], summary["covers"][1]);
}

#[test]
fn a_second_pinned_rule_has_the_notes_tidied_at_once() {
    // Every pinned note rides along with every request, so one that says an old rule in new words
    // is merged now rather than ten extractions later.
    let mut harness = Harness::new();
    harness.turn(0, "use uv, not pip");
    let laid = harness.layout(0);
    harness.answer(&laid["jobs"], "extract", learned());
    assert!(
        harness
            .rows("jobs", json!({}))
            .iter()
            .all(|j| j["kind"] != "tidy"),
        "one pinned rule has nothing to be merged with"
    );

    harness.turn(1, "a firm rule: we never push to main");
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [{ "op": "add", "title": "No pushing to main",
                          "text": "Never push to main.", "pinned": true }] }),
    );
    assert!(
        harness
            .rows("jobs", json!({}))
            .iter()
            .any(|j| j["kind"] == "tidy"),
        "a second pinned rule should have the notes tidied"
    );
}

#[test]
fn a_plain_request_leaves_a_pinned_rule_in_its_own_words() {
    // "Reading each file yourself" had an extract re-add the pinned rule under its title, reworded.
    let mut harness = Harness::new();
    harness.turn(0, "use uv, not pip");
    let laid = harness.layout(0);
    harness.answer(&laid["jobs"], "extract", learned());
    harness.turn(1, "now do the same for the parser, reading it yourself");
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [{ "op": "add", "title": "Package manager",
                          "text": "Install packages with uv, never with pip.", "pinned": true }] }),
    );
    let notes = harness.one("notes", json!({}));
    assert_eq!(notes["pinned"][0]["text"], "Use uv, not pip.", "{notes}");
}

#[test]
fn a_request_about_the_rules_lays_none_down() {
    // "List every standing rule…" had the request itself pinned into every later prompt.
    let mut harness = Harness::new();
    harness.turn(
        0,
        "Write docs/rules-check.md: list every standing rule or preference you were given, \
         then say for each doc whether it follows them.",
    );
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [{ "op": "add", "title": "Standing rules docs",
                          "text": "Write docs/rules-check.md listing every standing rule.",
                          "pinned": true }] }),
    );
    let notes = harness.one("notes", json!({}));
    assert!(
        notes["pinned"].as_array().is_none_or(Vec::is_empty),
        "pinned from a request: {notes}"
    );
}

#[test]
fn a_plain_request_adds_facts_but_no_rule_of_its_own() {
    // The assistant fixing its own typo came back as "every document must use exact spelling".
    let mut harness = Harness::new();
    harness.turn(0, "now write the glossary");
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [
            { "op": "add", "title": "Fix typos",
              "text": "Every written document must use exact spelling." },
            { "op": "add", "title": "Test runner", "text": "Tests run with cargo nextest." }
        ] }),
    );
    let notes = harness.one("notes", json!({}));
    let titles: Vec<&str> = notes["deferred"]
        .as_array()
        .map(|all| all.iter().filter_map(|n| n["title"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(titles, ["Test runner"], "{notes}");
}

#[test]
fn a_note_that_says_the_request_back_is_no_note() {
    // "Read every Rust file … then write the doc" came back as a note in three runs running.
    let mut harness = Harness::new();
    harness.turn(
        0,
        "Read every Rust file under crates/parse, whole and yourself, then write docs/parse.md \
         with one section per file.",
    );
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [
            { "op": "add", "title": "Read Rust files whole",
              "text": "Read every Rust file under the given directories whole and yourself." },
            { "op": "add", "title": "Parser home", "text": "The parser lives in crates/parse." }
        ] }),
    );
    let notes = harness.one("notes", json!({}));
    let titles: Vec<&str> = notes["deferred"]
        .as_array()
        .map(|all| all.iter().filter_map(|n| n["title"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(titles, ["Parser home"], "{notes}");
}

#[test]
fn a_tidy_leaves_a_pinned_rule_in_its_own_words() {
    // Every request after a reworded pinned rule misses the cache, and nothing was gained by it.
    let mut harness = Harness::new();
    harness.turn(0, "use uv, not pip");
    let laid = harness.layout(0);
    harness.answer(&laid["jobs"], "extract", learned());
    harness.turn(1, "a firm rule: we never push to main");
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [{ "op": "add", "title": "No pushing to main",
                          "text": "Never push to main.", "pinned": true }] }),
    );
    let jobs = Value::Array(harness.rows("jobs", json!({})));
    harness.answer(
        &jobs,
        "tidy",
        json!({ "ops": [{ "op": "update", "id": "N-1", "title": "Python packages",
                          "text": "Always install Python packages with uv rather than pip." }] }),
    );
    let notes = harness.one("notes", json!({}));
    assert_eq!(notes["pinned"][0]["title"], "Package manager", "{notes}");
    assert_eq!(notes["pinned"][0]["text"], "Use uv, not pip.");
}

#[test]
fn a_request_for_one_piece_of_work_pins_nothing() {
    // "About 900 words, then stop" was pinned from the fifth chapter request as if it were a rule.
    let mut harness = Harness::new();
    harness.turn(0, "Write only chapter 5 now, about 900 words, then stop.");
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [{ "op": "add", "title": "Target about 900 words",
                          "text": "Each chapter should be about 900 words.", "pinned": true }] }),
    );
    let notes = harness.one("notes", json!({}));
    assert!(
        notes["pinned"].as_array().is_none_or(Vec::is_empty),
        "pinned from a plain request: {notes}"
    );
    assert!(
        notes["deferred"].as_array().is_none_or(Vec::is_empty),
        "a rule nobody laid down, kept unpinned: {notes}"
    );
}

#[test]
fn a_rule_said_again_in_other_words_is_not_kept_twice() {
    let mut harness = Harness::new();
    harness.turn(
        0,
        "a firm rule: every chapter ends with the single word Selah",
    );
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [{ "op": "add", "title": "Chapters end with Selah",
                          "text": "Every chapter ends with the single word Selah.", "pinned": true }] }),
    );
    harness.turn(1, "write chapter two");
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [{ "op": "add", "title": "Fixed rule for chapters",
                          "text": "Always a firm rule: every chapter ends with the single word Selah.",
                          "pinned": false }] }),
    );
    let notes = harness.one("notes", json!({}));
    let kept = notes["pinned"].as_array().map_or(0, Vec::len)
        + notes["deferred"].as_array().map_or(0, Vec::len);
    assert_eq!(kept, 1, "the same rule kept twice: {notes}");
    assert_eq!(notes["pinned"][0]["title"], "Chapters end with Selah");
}

#[test]
fn a_rule_with_words_added_in_the_middle_is_not_kept_twice() {
    // A fresh session re-added the pinned Sources rule as "Every section *you write* in docs/…".
    let mut harness = Harness::new();
    harness.turn(
        0,
        "a firm rule: every section in docs/ ends with a Sources line listing the files read for it",
    );
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [{ "op": "add", "title": "Docs end with Sources",
                          "text": "Every section in docs/ ends with a Sources line listing the files read for it.",
                          "pinned": true }] }),
    );
    harness.turn(1, "write the glossary");
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [
            { "op": "add", "title": "One rule per section",
              "text": "Every section you write in docs/ ends with a Sources line listing the files read for it." },
            { "op": "add", "title": "Test runner", "text": "Tests run with cargo nextest." }
        ] }),
    );
    let notes = harness.one("notes", json!({}));
    let deferred = notes["deferred"].as_array().cloned().unwrap_or_default();
    assert_eq!(notes["pinned"].as_array().map_or(0, Vec::len), 1, "{notes}");
    assert_eq!(
        deferred.len(),
        1,
        "the rule kept twice, or a new note dropped: {notes}"
    );
    assert_eq!(deferred[0]["title"], "Test runner");
}

#[test]
fn a_plain_request_never_unpins_a_rule() {
    // An update from a chapter request carried `pinned: true`; turning every pin off there
    // unpinned the rule the person had laid down in the first prompt.
    let mut harness = Harness::new();
    harness.turn(
        0,
        "a firm rule: every chapter ends with the single word Selah",
    );
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [{ "op": "add", "title": "Chapters end with Selah",
                          "text": "Every chapter ends with the single word Selah.", "pinned": true }] }),
    );
    harness.turn(1, "write chapter two");
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [{ "op": "update", "id": "N-1",
                          "description": "Selah closes every chapter.", "pinned": true }] }),
    );
    let notes = harness.one("notes", json!({}));
    assert_eq!(
        notes["pinned"][0]["title"], "Chapters end with Selah",
        "a plain request unpinned the rule: {notes}"
    );
}

#[test]
fn an_empty_answer_to_a_stated_rule_is_asked_again() {
    // A rule said once and answered with an empty list was never recorded: nothing after it says
    // the rule again.
    let mut harness = Harness::new();
    harness.turn(0, "a firm rule: indent with tabs, never spaces");
    let laid = harness.layout(0);
    harness.answer(&laid["jobs"], "extract", json!({ "ops": [] }));
    let again = harness.rows("jobs", json!({}));
    let retry = again
        .iter()
        .find(|j| j["kind"] == "extract")
        .expect("the extraction asked once more")
        .clone();
    harness.answer(&json!([retry]), "extract", json!({ "ops": [] }));
    assert!(
        harness
            .rows("jobs", json!({}))
            .iter()
            .all(|j| j["kind"] != "extract"),
        "asked a third time"
    );
}

#[test]
fn a_plain_request_never_unpins_a_rule_by_its_title() {
    // An extraction over builders' reports answered with an add that reused the rule's title; forced
    // unpinned, it landed on the rule and unpinned it.
    let mut harness = Harness::new();
    harness.turn(
        0,
        "a firm rule: every Python file starts with a module docstring",
    );
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [{ "op": "add", "title": "Module docstrings required",
                          "text": "Every Python file starts with a module docstring.", "pinned": true }] }),
    );
    harness.turn(1, "carry on with the store");
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({ "ops": [{ "op": "add", "id": "N-9", "title": "Module docstrings required",
                          "text": "Every Python file starts with a module docstring.",
                          "description": "Module docstrings required.", "pinned": true }] }),
    );
    let notes = harness.one("notes", json!({}));
    assert_eq!(
        notes["pinned"][0]["title"], "Module docstrings required",
        "a plain request unpinned the rule by its title: {notes}"
    );
}
