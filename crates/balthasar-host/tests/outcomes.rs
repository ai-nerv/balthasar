//! The outcome verbs across the two doors.
//!
//! The security half of §6.10: a peer may report how its own action went, and may not sign that
//! report as the person's judgment or promote its own attribution.

use balthasar_host::{Answering, Door, answer};
use balthasar_ipc::{Peer, Reply, Request};
use balthasar_model::{
    Body, Memory, MemoryId, NoteKind, Presentation, ScopeId, SessionId, Tier, Witness, WitnessId,
    WitnessKind, floor,
};
use balthasar_store::{Injection, Store, mint};

const NOW: balthasar_model::Timestamp = 1_756_000_000;

struct Held {
    store: Store,
    memory: MemoryId,
}

impl Held {
    fn new() -> Self {
        let mut store = Store::ephemeral().expect("store");
        let held = Memory::new(
            mint(NOW),
            Tier::Fact,
            ScopeId::new("/w/thing"),
            Body::note("the deploy target is fly.io", NoteKind::Claim),
            NOW,
        );
        let memory = held.id.clone();
        store
            .remember(
                held,
                Witness::new(
                    WitnessId::new("w1"),
                    WitnessKind::Imperative,
                    SessionId::new("01RUN"),
                    ScopeId::new("/w/thing"),
                    NOW,
                ),
                NOW,
            )
            .expect("remember");
        store
            .note_injection(
                &Injection {
                    id: "i1".to_owned(),
                    recall: None,
                    session: Some(SessionId::new("01RUN")),
                    created_at: NOW,
                    token_count: 12,
                    remote: false,
                    policy: "balanced".to_owned(),
                },
                &[(memory.clone(), Presentation::Asserted)],
            )
            .expect("inject");
        Self { store, memory }
    }

    fn ask(&mut self, door: &Door, name: &str, args: Vec<serde_json::Value>) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: None,
            scratch: None,
            scope: ScopeId::new("/w/thing"),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        };
        answer(
            &mut at,
            door,
            &Request {
                call: name.to_owned(),
                args,
            },
        )
    }

    fn field(reply: &Reply, name: &str) -> Option<serde_json::Value> {
        reply
            .result
            .as_ref()
            .and_then(|values| values.first())
            .and_then(|v| v.get(name))
            .cloned()
    }
}

fn peer() -> Door {
    Door::Socket(Peer {
        pid: 4242,
        uid: 1000,
        program: Some("/usr/bin/some-harness".to_owned()),
    })
}

#[test]
fn a_peer_cannot_sign_its_report_as_the_persons_judgment() {
    // A user evaluation and a peer's self-report carry different weight in every policy that
    // reads them. A peer that could forge the stronger one could manufacture its own authority.
    let mut held = Held::new();
    let used = held.ask(
        &peer(),
        "used",
        vec![
            serde_json::json!("i1"),
            serde_json::json!({ "memories": [held.memory.to_string()], "attribution": "explicit" }),
        ],
    );
    let action = Held::field(&used, "action").expect("an action");
    let action = action.as_str().expect("a string").to_owned();

    let reply = held.ask(
        &peer(),
        "outcome",
        vec![
            serde_json::json!(action),
            serde_json::json!({ "kind": "succeeded", "evaluator": "user" }),
        ],
    );
    assert!(reply.ok, "{reply:?}");

    let evaluator = held
        .store
        .outcome_of(&action)
        .expect("read back")
        .expect("a verdict")
        .evaluator;
    assert_ne!(evaluator, "user", "the peer signed as the user");
    assert!(
        evaluator.contains("some-harness"),
        "it signed as itself: {evaluator}"
    );
}

#[test]
fn the_owners_door_may_record_the_persons_judgment() {
    // The other half: the ceiling has to let the real thing through, or nothing can ever be a
    // user evaluation and the distinction is decoration.
    let mut held = Held::new();
    let used = held.ask(
        &Door::Owner,
        "used",
        vec![
            serde_json::json!("i1"),
            serde_json::json!({ "memories": [held.memory.to_string()] }),
        ],
    );
    let action = Held::field(&used, "action").expect("an action");
    let action = action.as_str().expect("a string").to_owned();

    held.ask(
        &Door::Owner,
        "outcome",
        vec![
            serde_json::json!(action),
            serde_json::json!({ "kind": "succeeded", "evaluator": "user" }),
        ],
    );
    let evaluator = held
        .store
        .outcome_of(&action)
        .expect("read back")
        .expect("a verdict")
        .evaluator;
    assert_eq!(evaluator, "user");
}

#[test]
fn an_action_that_names_no_memory_is_proximal_however_it_was_asked_for() {
    // Claiming to have followed nothing in particular explicitly is not a claim balthasar can act
    // on, and treating it as one would credit whatever happened to be injected.
    let mut held = Held::new();
    let used = held.ask(
        &Door::Owner,
        "used",
        vec![
            serde_json::json!("i1"),
            serde_json::json!({ "attribution": "explicit" }),
        ],
    );
    let action = Held::field(&used, "action").expect("an action");
    let action = action.as_str().expect("a string").to_owned();
    held.ask(
        &Door::Owner,
        "outcome",
        vec![
            serde_json::json!(action),
            serde_json::json!({ "kind": "succeeded" }),
        ],
    );

    let utility = held.store.utility_of(&held.memory).expect("utility");
    assert!(
        !utility.is_verified(),
        "it credited a memory it did not name"
    );
}

#[test]
fn the_ledger_does_not_keep_what_the_action_was() {
    // The caller sends its command; the ledger keeps a digest. Otherwise the table that
    // promises to hold no arguments is the table holding every argument.
    let mut held = Held::new();
    let used = held.ask(
        &Door::Owner,
        "used",
        vec![
            serde_json::json!("i1"),
            serde_json::json!({
                "memories": [held.memory.to_string()],
                "action": "flyctl deploy --token SECRET-VALUE-HERE",
            }),
        ],
    );
    let action = Held::field(&used, "action").expect("an action");
    let hash = held
        .store
        .use_of(action.as_str().expect("a string"))
        .expect("read back")
        .expect("an action")
        .action_hash;
    assert!(!hash.contains("SECRET"), "the token was stored: {hash}");
    assert!(!hash.contains("flyctl"), "the command was stored: {hash}");
}

#[test]
fn a_trace_walks_from_a_recall_to_what_it_led_to() {
    let mut held = Held::new();
    let reply = held.ask(
        &Door::Owner,
        "utility",
        vec![serde_json::json!(held.memory.to_string())],
    );
    assert!(reply.ok, "{reply:?}");
    // Injected, never acted on: unknown, and no verdict either way.
    assert_eq!(Held::field(&reply, "unknown"), Some(serde_json::json!(1)));
    assert_eq!(
        Held::field(&reply, "helpfulness"),
        Some(serde_json::Value::Null)
    );
    assert_eq!(
        Held::field(&reply, "verified_harmful"),
        Some(serde_json::json!(0))
    );
}

#[test]
fn every_new_verb_is_on_the_published_surface() {
    // A verb a peer can call but cannot discover is one nobody audits.
    let mut held = Held::new();
    let reply = held.ask(&Door::Owner, "verbs", Vec::new());
    let listed = serde_json::to_string(&reply.result).expect("json");
    for name in ["used", "outcome", "trace", "utility"] {
        assert!(
            listed.contains(&format!("\"{name}\"")),
            "{name} is not listed"
        );
    }
}
