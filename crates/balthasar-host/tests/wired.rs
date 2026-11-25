//! End-to-end verification of wired host capabilities: entity search, embeddings, traversal,
//! context assembly, status backends, outcome hooks, and layout ledger attribution.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_host::{Answering, Door, Hooks, answer_hooked};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{
    AgentId, Body, Derivation, Memory, NoteKind, Relation, ScopeId, SessionId, Tier, Timestamp,
    View, Witness, WitnessId, WitnessKind, floor,
};
use balthasar_recall::{FEATURES, LAYOUT, Model, Section};
use balthasar_store::{Store, Transcript, mint};
use serde_json::{Value, json};

const NOW: Timestamp = 1_756_000_000;

struct WiredHooks {
    sections: Vec<Section>,
    used_events: Vec<Value>,
    classified: Option<String>,
    ranker: Option<Model>,
}

impl Default for WiredHooks {
    fn default() -> Self {
        Self {
            sections: vec![
                Section::read("facts", &json!({ "tiers": ["fact"], "weight": 1 })).expect("sec"),
            ],
            used_events: Vec::new(),
            classified: Some("succeeded".to_owned()),
            ranker: None,
        }
    }
}

impl Hooks for WiredHooks {
    fn embed(&self, text: &str) -> Option<Vec<f32>> {
        if text.trim().is_empty() {
            return None;
        }
        Some(if text.contains("alpha") {
            vec![1.0, 0.0]
        } else {
            vec![0.0, 1.0]
        })
    }

    fn sections(&self) -> Vec<Section> {
        self.sections.clone()
    }

    fn backends(&self) -> Option<Value> {
        Some(json!({
            "embedder": { "model": "test-embed", "note": null },
            "distiller": { "reachable": ["command:cat"], "unavailable": [] },
        }))
    }

    fn classify_outcome(&mut self, _event: &Value) -> Option<String> {
        self.classified.clone()
    }

    fn note_used(&mut self, action: &Value) {
        self.used_events.push(action.clone());
    }

    fn ranker(&self) -> Option<&Model> {
        self.ranker.as_ref()
    }
}

fn remember(
    store: &mut Store,
    scope: &ScopeId,
    text: &str,
    id_tag: &str,
) -> balthasar_model::MemoryId {
    let mem = Memory::new(
        mint(NOW),
        Tier::Fact,
        scope.clone(),
        Body::note(text, NoteKind::Claim),
        NOW,
    );
    let witness = Witness::new(
        WitnessId::new(format!("w-{id_tag}")),
        WitnessKind::Imperative,
        SessionId::new("s1"),
        scope.clone(),
        NOW,
    );
    store
        .remember(mem, witness, NOW)
        .expect("remember")
        .id()
        .clone()
}

fn call(
    store: &mut Store,
    scrollback: Option<&mut Transcript>,
    scope: &ScopeId,
    capture: bool,
    hooks: &mut dyn Hooks,
    verb: &str,
    args: Vec<Value>,
) -> Reply {
    let mut at = Answering {
        store,
        scrollback,
        scratch: None,
        scope: scope.clone(),
        agent: AgentId::main(),
        now: NOW,
        inject_floor: floor::INJECT,
        live_floor: floor::LIVE,
        capture,
    };
    answer_hooked(
        &mut at,
        &Door::Owner,
        &Request {
            call: verb.to_owned(),
            args,
        },
        hooks,
    )
}

#[test]
fn context_verb_assembles_configured_sections() {
    let mut store = Store::ephemeral().expect("store");
    let scope = ScopeId::new("/w/proj");
    remember(&mut store, &scope, "we deploy with fly.io", "1");

    let mut hooks = WiredHooks::default();
    let reply = call(
        &mut store,
        None,
        &scope,
        false,
        &mut hooks,
        "context",
        vec![json!({ "turn": "deploy", "budget": 500 })],
    );
    assert!(reply.ok, "{reply:?}");
    let body = &reply.result[0];
    assert!(
        body["text"].as_str().unwrap_or("").contains("fly.io"),
        "{body}"
    );
    assert_eq!(body["sections"].as_array().map(Vec::len), Some(1));
}

#[test]
fn status_verb_includes_embedder_and_distiller_reachability() {
    let mut store = Store::ephemeral().expect("store");
    let scope = ScopeId::new("/w/proj");
    let mut hooks = WiredHooks::default();
    let reply = call(
        &mut store,
        None,
        &scope,
        false,
        &mut hooks,
        "status",
        vec![],
    );
    assert!(reply.ok, "{reply:?}");
    assert_eq!(reply.result[0]["embedder"]["model"], "test-embed");
    assert_eq!(
        reply.result[0]["distiller"]["reachable"],
        json!(["command:cat"])
    );
}

#[test]
fn recall_uses_vectors_relations_and_learned_ranker() {
    let mut store = Store::ephemeral().expect("store");
    let scope = ScopeId::new("/w/proj");
    let a = remember(&mut store, &scope, "deployment target is fly.io", "a");
    let b = remember(&mut store, &scope, "rollback runs make revert", "b");
    store.embed(&a, &[1.0, 0.0], "test-embed").expect("embed a");
    store.embed(&b, &[0.0, 1.0], "test-embed").expect("embed b");
    store
        .relate(&[Relation {
            from: a.clone(),
            to: b.clone(),
            view: View::SameEntity,
            weight: 0.9,
            source: Derivation::Rule,
            derivation_version: 1,
            evidence_cursor: None,
            created_at: NOW,
        }])
        .expect("relate");

    let mut hooks = WiredHooks::default();
    let reply = call(
        &mut store,
        None,
        &scope,
        true,
        &mut hooks,
        "recall",
        vec![json!("alpha about fly.io")],
    );
    assert!(reply.ok, "{reply:?}");
    let rows = reply.result[0]["memories"].as_array().expect("memories");
    assert!(
        rows.iter().any(|r| r["id"] == b.to_string()),
        "entity traversal should reach related memory: {rows:?}"
    );

    let mut model = Model {
        weights: vec![0.0; FEATURES.len()],
        bias: 0.0,
        mean: vec![0.0; FEATURES.len()],
        scale: vec![1.0; FEATURES.len()],
        trained_on: 500,
        holdout_auc: 0.85,
        layout: LAYOUT,
    };
    model.weights[2] = -4.0;
    hooks.ranker = Some(model);
    let reranked = call(
        &mut store,
        None,
        &scope,
        false,
        &mut hooks,
        "recall",
        vec![json!("alpha")],
    );
    assert!(reranked.ok, "{reranked:?}");
    assert_eq!(reranked.result[0]["id"], b.to_string());
}

#[test]
fn used_and_outcome_invoke_configured_hooks() {
    let mut store = Store::ephemeral().expect("store");
    let scope = ScopeId::new("/w/proj");
    let id = remember(&mut store, &scope, "run `make test` before pushing", "u1");
    let mut hooks = WiredHooks::default();

    let recalled = call(
        &mut store,
        None,
        &scope,
        true,
        &mut hooks,
        "recall",
        vec![json!("make test")],
    );
    let injection = recalled.result[0]["injection"]
        .as_str()
        .expect("injection")
        .to_owned();

    let used = call(
        &mut store,
        None,
        &scope,
        true,
        &mut hooks,
        "used",
        vec![
            json!(injection),
            json!({ "tool": "shell", "memories": [id.to_string()] }),
        ],
    );
    assert!(used.ok, "{used:?}");
    assert_eq!(hooks.used_events.len(), 1);
    let action = used.result[0]["action"]
        .as_str()
        .expect("action")
        .to_owned();

    let out = call(
        &mut store,
        None,
        &scope,
        true,
        &mut hooks,
        "outcome",
        vec![json!(action), json!({ "tool": "shell", "exit_code": 0 })],
    );
    assert!(out.ok, "{out:?}");
    assert_eq!(out.result[0]["kind"], "succeeded");
}

#[test]
fn applied_layout_records_traceable_recall_run_when_capturing() {
    let mut store = Store::ephemeral().expect("store");
    let mut scrollback = Transcript::ephemeral().expect("scrollback");
    let scope = ScopeId::new("/w/proj");
    remember(&mut store, &scope, "we deploy with fly.io", "l1");
    let mut hooks = WiredHooks::default();

    let obs = call(
        &mut store,
        Some(&mut scrollback),
        &scope,
        true,
        &mut hooks,
        "observe",
        vec![
            json!("s1"),
            json!({ "cursor": 1, "role": "user", "kind": "user", "text": "deploy with fly.io" }),
        ],
    );
    assert!(obs.ok, "{obs:?}");

    let laid = call(
        &mut store,
        Some(&mut scrollback),
        &scope,
        true,
        &mut hooks,
        "layout",
        vec![
            json!("s1"),
            json!({ "query": "deploy fly.io", "window": 32_000 }),
        ],
    );
    assert!(laid.ok, "{laid:?}");
    let layout_id = laid.result[0]["id"].as_str().expect("layout id").to_owned();

    let applied = call(
        &mut store,
        Some(&mut scrollback),
        &scope,
        true,
        &mut hooks,
        "applied",
        vec![
            json!("s1"),
            json!({ "id": layout_id, "usage": { "input": 100 } }),
        ],
    );
    assert!(applied.ok, "{applied:?}");

    let traced = store
        .trace_of(&format!("recall-layout-{layout_id}"))
        .expect("trace");
    assert!(traced.is_some(), "layout delivery should link a RecallRun");
}

#[test]
fn peer_remember_promotes_only_verbatim_whole_sentences_from_user_turns() {
    let mut store = Store::ephemeral().expect("store");
    let mut scrollback = Transcript::ephemeral().expect("scrollback");
    let scope = ScopeId::new("/w/proj");
    let mut hooks = WiredHooks::default();

    for (cursor, role, kind, text) in [
        (
            1,
            "user",
            "user",
            "Never force push to main. Let's use zstd for archives.",
        ),
        (
            2,
            "assistant",
            "prose",
            "Always deploy straight to production.",
        ),
        (3, "user", "from", "Sibling says use redis everywhere."),
        (
            4,
            "user",
            "user",
            "Here is a snippet:\n```\nUse sqlite in-memory only.\n```",
        ),
    ] {
        let obs = call(
            &mut store,
            Some(&mut scrollback),
            &scope,
            false,
            &mut hooks,
            "observe",
            vec![
                json!("s1"),
                json!({ "cursor": cursor, "role": role, "kind": kind, "text": text }),
            ],
        );
        assert!(obs.ok, "{obs:?}");
    }

    let peer = Door::Socket(balthasar_ipc::Peer {
        pid: 42,
        uid: 1000,
        program: Some("magi".into()),
        agent: None,
    });
    let ask_peer = |store: &mut Store,
                    scrollback: &mut Transcript,
                    hooks: &mut WiredHooks,
                    statement: &str| {
        let mut at = Answering {
            store,
            scrollback: Some(scrollback),
            scratch: None,
            scope: scope.clone(),
            agent: AgentId::default(),
            now: NOW,
            inject_floor: 0.35,
            live_floor: 0.15,
            capture: false,
        };
        answer_hooked(
            &mut at,
            &peer,
            &Request {
                call: "remember".into(),
                args: vec![json!(statement), json!({ "from_session": "s1" })],
            },
            hooks,
        )
    };

    let exact = ask_peer(
        &mut store,
        &mut scrollback,
        &mut hooks,
        "Let's use zstd for archives",
    );
    assert!(exact.ok, "{exact:?}");
    assert_eq!(exact.result[0]["witness"], "choice");
    assert_eq!(exact.result[0]["verified"], true);
    assert_eq!(exact.result[0]["cursor"], 1);
    let exact_id = balthasar_model::MemoryId::new(exact.result[0]["id"].as_str().expect("id"));
    let kept = store.get(&exact_id).expect("get").expect("memory");
    assert!(
        kept.confidence >= balthasar_model::floor::INJECT,
        "verified choice should cross inject floor: {}",
        kept.confidence
    );
    assert!(kept.is_assertable(balthasar_model::floor::INJECT, NOW, true));
    assert_eq!(kept.witnesses[0].cursor, Some(1));

    for unverified in [
        "force push to main",
        "Always deploy straight to production.",
        "Sibling says use redis everywhere.",
        "Use sqlite in-memory only.",
        "Fabricated rule the user never typed.",
    ] {
        let reply = ask_peer(&mut store, &mut scrollback, &mut hooks, unverified);
        assert!(reply.ok, "{reply:?}");
        assert_eq!(reply.result[0]["witness"], "manual", "{unverified}");
        assert_eq!(reply.result[0]["verified"], false, "{unverified}");
        let id = balthasar_model::MemoryId::new(reply.result[0]["id"].as_str().expect("id"));
        let mem = store.get(&id).expect("get").expect("memory");
        assert!(
            mem.confidence < balthasar_model::floor::INJECT,
            "unverified peer proposal must stay below inject floor: {}",
            mem.confidence
        );
    }
}
