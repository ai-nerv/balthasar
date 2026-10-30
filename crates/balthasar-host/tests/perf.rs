//! `layout` and `recall` at the tail, against a store of 20,000 live memories written one
//! transaction at a time. Run by `oslo make perf`; `BALTHASAR_PERF_SCALE` multiplies the budgets
//! on a slower machine, and nothing else.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_host::{Answering, Door, Hooks, answer_hooked};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{
    AgentId, Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId,
    WitnessKind, floor, scratch::Scratch,
};
use balthasar_store::{Store, Transcript, mint};
use serde_json::{Value, json};
use std::time::Instant;

const NOW: Timestamp = 1_756_000_000;
const SCOPE: &str = "/w/thing";
const SESSION: &str = "01PERF";
const LIVE: usize = 20_000;
const CALLS: usize = 60;

/// The tail each call must stay under, in milliseconds.
const LAYOUT_P95_MS: f64 = 75.0;
const RECALL_P95_MS: f64 = 60.0;

const SUBJECTS: &[&str] = &[
    "billing",
    "webhooks",
    "telemetry",
    "invoices",
    "payouts",
    "scheduler",
    "indexer",
    "gateway",
    "ledger",
    "renderer",
    "uploader",
    "notifier",
    "sharder",
    "throttler",
    "compactor",
    "digester",
];

struct Plain;
impl Hooks for Plain {}

fn scale() -> f64 {
    std::env::var("BALTHASAR_PERF_SCALE")
        .ok()
        .and_then(|n| n.parse().ok())
        .filter(|n: &f64| *n > 0.0)
        .unwrap_or(1.0)
}

fn ask(store: &mut Store, scrollback: &mut Transcript, call: &str, args: Vec<Value>) -> Reply {
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
    answer_hooked(&mut at, &Door::Owner, &request, &mut Plain)
}

/// A store of [`LIVE`] memories under `dir`, each its own transaction.
fn filled(dir: &Scratch) -> (Store, Transcript) {
    let mut store = Store::open(&dir.join("memory.db")).expect("store");
    for n in 0..LIVE {
        let subject = SUBJECTS[n % SUBJECTS.len()];
        let text = format!(
            "the {subject} service {} listens on port {} and deploys with make {subject}-{n}",
            n / SUBJECTS.len(),
            4000 + n % 997
        );
        let at = NOW - (LIVE - n) as Timestamp * 60;
        let memory = Memory::new(
            mint(at),
            Tier::Fact,
            ScopeId::new(SCOPE),
            Body::note(text, NoteKind::Claim),
            at,
        );
        let witness = Witness::new(
            WitnessId::new(format!("w-{n}")),
            WitnessKind::Imperative,
            SessionId::new(format!("s{}", n % 50)),
            ScopeId::new(SCOPE),
            at,
        );
        store.remember(memory, witness, at).expect("remembered");
    }
    let scrollback = Transcript::open(&dir.join("transcript.db")).expect("scrollback");
    (store, scrollback)
}

fn p95(mut taken: Vec<f64>) -> f64 {
    taken.sort_by(f64::total_cmp);
    let at = ((taken.len() - 1) as f64 * 0.95).round() as usize;
    taken[at]
}

#[test]
#[ignore = "a measurement, run by `oslo make perf`"]
fn layout_and_recall_stay_inside_their_tail_at_twenty_thousand() {
    let dir = Scratch::new("balthasar-perf", "tail");
    let (mut store, mut scrollback) = filled(&dir);
    let turn = json!({ "cursor": 0, "role": "user", "kind": "user", "tokens": 20,
                       "text": "where does the billing service deploy from" });
    assert!(
        ask(
            &mut store,
            &mut scrollback,
            "observe",
            vec![json!(SESSION), turn]
        )
        .ok
    );

    let mut laid = Vec::new();
    let mut recalled = Vec::new();
    for round in 0..CALLS {
        let subject = SUBJECTS[round % SUBJECTS.len()];
        let query = format!("which port does the {subject} service listen on");
        let window = json!({ "round": round, "window": 200_000, "reply": 8_000,
                             "fixed": { "system": 600, "tools": 400 }, "query": query,
                             "idle_s": 5, "helpers": [] });
        let started = Instant::now();
        let reply = ask(
            &mut store,
            &mut scrollback,
            "layout",
            vec![json!(SESSION), window],
        );
        laid.push(started.elapsed().as_secs_f64() * 1000.0);
        assert!(reply.ok, "layout: {:?}", reply.error);

        let started = Instant::now();
        let reply = ask(&mut store, &mut scrollback, "recall", vec![json!(query)]);
        recalled.push(started.elapsed().as_secs_f64() * 1000.0);
        assert!(reply.ok, "recall: {:?}", reply.error);
    }
    let (layout, recall) = (p95(laid), p95(recalled));
    println!("perf: {LIVE} live, layout p95 {layout:.2} ms, recall p95 {recall:.2} ms");
    assert!(
        layout <= LAYOUT_P95_MS * scale(),
        "layout p95 {layout:.2} ms over {LAYOUT_P95_MS} ms"
    );
    assert!(
        recall <= RECALL_P95_MS * scale(),
        "recall p95 {recall:.2} ms over {RECALL_P95_MS} ms"
    );
}
