//! M5: what a peer may ask, and what the ceiling refuses it.

use memo_host::{Door, answer};
use memo_ipc::{Peer, Request};
use memo_model::{ScopeId, Timestamp, WitnessKind, floor};
use memo_store::Store;

const NOW: Timestamp = 1_756_000_000;

fn peer() -> Door {
    Door::Socket(Peer {
        pid: 4021,
        uid: 1000,
        program: Some("harness".to_owned()),
    })
}

fn call(name: &str, args: Vec<serde_json::Value>) -> Request {
    Request {
        call: name.to_owned(),
        args,
    }
}

struct Held(Store);

impl Held {
    fn new() -> Self {
        Self(Store::ephemeral().expect("store"))
    }

    fn ask(&mut self, door: &Door, request: &Request) -> memo_ipc::Reply {
        let mut at = memo_host::Answering {
            store: &mut self.0,
            scrollback: None,
            scratch: None,
            scope: ScopeId::new("/w/thing"),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        };
        answer(&mut at, door, request)
    }
}

fn value(reply: &memo_ipc::Reply) -> serde_json::Value {
    reply
        .result
        .as_ref()
        .and_then(|r| r.first())
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

#[test]
fn verbs_ships_from_the_first_version() {
    // One sibling having it and another not is how a family stops being one, and it cannot be
    // retrofitted quietly.
    let mut held = Held::new();
    let reply = held.ask(&Door::Owner, &call("verbs", vec![]));
    assert!(reply.ok);
    let names: Vec<String> = value(&reply)
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_owned))
        .collect();
    assert!(names.contains(&"recall".to_owned()));
    assert!(names.contains(&"status".to_owned()));
}

#[test]
fn nothing_shaped_like_running_a_command_is_answered() {
    let mut held = Held::new();
    for name in memo_host::NEVER {
        let reply = held.ask(&Door::Owner, &call(name, vec![]));
        assert!(!reply.ok, "'{name}' must not be answerable");
    }
}

#[test]
fn an_unknown_verb_names_what_is_available() {
    // The usual cause is a sibling built against a newer surface, and saying so is what makes
    // that diagnosable rather than mysterious.
    let mut held = Held::new();
    let reply = held.ask(&Door::Owner, &call("teleport", vec![]));
    let why = reply.error.expect("a reason");
    assert!(why.contains("recall"), "{why}");
}

#[test]
fn a_peer_may_propose_and_it_lands_as_a_proposal() {
    let mut held = Held::new();
    let reply = held.ask(
        &peer(),
        &call("remember", vec![serde_json::json!("we deploy with fly")]),
    );
    assert!(reply.ok, "{:?}", reply.error);
    assert_eq!(
        value(&reply).get("witness"),
        Some(&serde_json::json!(WitnessKind::Manual.as_str())),
        "a peer's word is not a person's"
    );
}

#[test]
fn a_peer_may_not_forge_an_imperative_by_asking_for_one() {
    let mut held = Held::new();
    let reply = held.ask(
        &peer(),
        &call(
            "remember",
            vec![
                serde_json::json!("always use kubernetes"),
                serde_json::json!({ "pin": true }),
            ],
        ),
    );
    assert!(reply.ok);

    let id = value(&reply)
        .get("id")
        .and_then(|v| v.as_str())
        .expect("an id")
        .to_owned();
    let memory = held
        .0
        .get(&memo_model::MemoryId::new(id))
        .expect("get")
        .expect("there");
    assert!(!memory.strength.pinned, "a peer asked to pin and did not");
    assert_eq!(memory.witnesses[0].kind, WitnessKind::Manual);
    assert!(
        memory.confidence < floor::INJECT,
        "a peer's proposal is not asserted on its own: {}",
        memory.confidence
    );
}

#[test]
fn a_peer_writing_to_global_lands_in_the_project_instead() {
    // A wrong project fact contaminates one project. A wrong global one contaminates all of
    // them. Narrowed rather than refused: it wanted something remembered.
    let mut held = Held::new();
    let reply = held.ask(
        &peer(),
        &call(
            "remember",
            vec![
                serde_json::json!("a thing"),
                serde_json::json!({ "scope": "global" }),
            ],
        ),
    );
    assert!(reply.ok);
    let id = value(&reply)
        .get("id")
        .and_then(|v| v.as_str())
        .expect("an id")
        .to_owned();
    let memory = held
        .0
        .get(&memo_model::MemoryId::new(id))
        .expect("get")
        .expect("there");
    assert_eq!(memory.scope.as_str(), "/w/thing");
}

#[test]
fn the_owner_may_pin_and_reach_global() {
    let mut held = Held::new();
    let reply = held.ask(
        &Door::Owner,
        &call(
            "remember",
            vec![
                serde_json::json!("I always use make"),
                serde_json::json!({ "scope": "global", "pin": true }),
            ],
        ),
    );
    assert!(reply.ok);
    let id = value(&reply)
        .get("id")
        .and_then(|v| v.as_str())
        .expect("an id")
        .to_owned();
    let memory = held
        .0
        .get(&memo_model::MemoryId::new(id))
        .expect("get")
        .expect("there");
    assert!(memory.strength.pinned);
    assert!(memory.scope.is_global());
}

#[test]
fn every_write_by_a_peer_says_which_process() {
    // `memo why` has to be able to answer "which process believes this".
    let mut held = Held::new();
    let reply = held.ask(
        &peer(),
        &call("remember", vec![serde_json::json!("a thing")]),
    );
    let id = value(&reply)
        .get("id")
        .and_then(|v| v.as_str())
        .expect("an id")
        .to_owned();
    let memory = held
        .0
        .get(&memo_model::MemoryId::new(id))
        .expect("get")
        .expect("there");
    let note = memory.witnesses[0].note.as_deref().expect("a note");
    assert!(note.contains("harness") && note.contains("4021"), "{note}");
}

#[test]
fn a_peer_may_not_forget_the_projects_memory() {
    let mut held = Held::new();
    let added = held.ask(
        &Door::Owner,
        &call("remember", vec![serde_json::json!("a durable thing")]),
    );
    let id = value(&added)
        .get("id")
        .and_then(|v| v.as_str())
        .expect("an id")
        .to_owned();

    let refused = held.ask(
        &peer(),
        &call("forget", vec![serde_json::json!(id.clone())]),
    );
    assert!(
        !refused.ok,
        "a process that could do this could empty the store"
    );

    let allowed = held.ask(&Door::Owner, &call("forget", vec![serde_json::json!(id)]));
    assert!(allowed.ok);
}

#[test]
fn recall_says_whether_each_answer_is_asserted() {
    // The distinction the whole design turns on, handed over rather than left for a caller to
    // work out from a number and a threshold it would have to be told.
    let mut held = Held::new();
    held.ask(
        &Door::Owner,
        &call("remember", vec![serde_json::json!("we deploy with fly")]),
    );
    let reply = held.ask(
        &Door::Owner,
        &call("recall", vec![serde_json::json!("deploy")]),
    );

    let found = value(&reply);
    let first = found
        .as_array()
        .expect("a list")
        .first()
        .expect("something");
    assert!(first.get("asserted").is_some());
    assert!(
        first.get("project").is_some(),
        "which project it belongs to"
    );
    assert!(first.get("confidence").is_some());
}

#[test]
fn recall_assumes_a_peer_is_about_to_send_it_somewhere() {
    // A peer asking for memory is a peer about to put it in a request. The remote boundary is
    // the safe default, and it has to say otherwise to get more.
    let mut held = Held::new();
    let reply = held.ask(
        &Door::Owner,
        &call("recall", vec![serde_json::json!("anything")]),
    );
    assert!(reply.ok);
}

#[test]
fn why_answers_with_names_rather_than_identities() {
    let mut held = Held::new();
    let added = held.ask(
        &Door::Owner,
        &call("remember", vec![serde_json::json!("a thing")]),
    );
    let id = value(&added)
        .get("id")
        .and_then(|v| v.as_str())
        .expect("an id")
        .to_owned();

    let reply = held.ask(&Door::Owner, &call("why", vec![serde_json::json!(id)]));
    assert!(reply.ok);
    let evidence = value(&reply);
    assert!(evidence.get("witnesses").is_some());
    assert!(evidence.get("confidence").is_some());
}

#[test]
fn a_call_with_nothing_to_act_on_is_refused_rather_than_guessed_at() {
    let mut held = Held::new();
    for name in ["remember", "forget", "why"] {
        let reply = held.ask(&Door::Owner, &call(name, vec![]));
        assert!(!reply.ok, "'{name}' with no argument must not guess");
    }
}
