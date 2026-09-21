//! The two verbs a person settles a disagreement with, and who may call them.

use balthasar_host::{Answering, Door, answer};
use balthasar_ipc::{Peer, Reply, Request};
use balthasar_model::{
    Body, LinkRelation, Memory, MemoryId, NoteKind, ScopeId, SessionId, Tier, Witness, WitnessId,
    WitnessKind, floor,
};
use balthasar_store::{Store, mint};
use serde_json::json;

const NOW: balthasar_model::Timestamp = 1_756_000_000;

struct Held {
    store: Store,
}

impl Held {
    fn new() -> Self {
        Self {
            store: Store::ephemeral().expect("store"),
        }
    }

    fn ask(&mut self, door: &Door, name: &str, args: Vec<serde_json::Value>) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: None,
            scratch: None,
            scope: ScopeId::new("/w/thing"),
            agent: balthasar_model::AgentId::main(),
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

    fn owner(&mut self, name: &str, args: Vec<serde_json::Value>) -> Reply {
        self.ask(&Door::Owner, name, args)
    }

    fn peer(&mut self, name: &str, args: Vec<serde_json::Value>) -> Reply {
        let door = Door::Socket(Peer {
            pid: 42,
            uid: 1000,
            program: Some("/usr/bin/some-harness".to_owned()),
        });
        self.ask(&door, name, args)
    }

    fn claim(&mut self, text: &str) -> MemoryId {
        let held = Memory::new(
            mint(NOW),
            Tier::Fact,
            ScopeId::new("/w/thing"),
            Body::note(text, NoteKind::Claim),
            NOW,
        );
        let id = held.id.clone();
        self.store
            .remember(
                held,
                Witness::new(
                    WitnessId::new(format!("w-{text}")),
                    WitnessKind::Imperative,
                    SessionId::new("01RUN"),
                    ScopeId::new("/w/thing"),
                    NOW,
                ),
                NOW,
            )
            .expect("remember");
        id
    }

    fn disagreeing(&mut self) -> (MemoryId, MemoryId) {
        let a = self.claim("The tests run on CI.");
        let b = self.claim("The tests need a GPU that no developer has.");
        for (src, dst) in [(&a, &b), (&b, &a)] {
            self.store
                .link(src, dst, LinkRelation::Contradicts, NOW)
                .expect("link");
        }
        for id in [&a, &b] {
            self.store.rescore(id, NOW).expect("rescore");
        }
        (a, b)
    }
}

#[test]
fn what_is_open_is_said_where_a_person_looks_as_well_as_in_its_own_view() {
    let mut held = Held::new();
    held.disagreeing();
    let listed = held.owner("disagreements", vec![]);
    assert!(listed.ok);
    assert_eq!(listed.n, 1);
    assert!(listed.result[0]["a"]["text"].is_string());
    // A view nobody thinks to open is no use, so the count rides along with what a person
    // already reads.
    let status = held.owner("status", vec![]);
    assert_eq!(status.result[0]["disagreements"], 1);
}

#[test]
fn settling_is_the_persons_and_a_peer_is_refused() {
    let mut held = Held::new();
    let (a, b) = held.disagreeing();
    let refused = held.peer(
        "settle",
        vec![
            json!(a.to_string()),
            json!(b.to_string()),
            json!({"keep": a.to_string()}),
        ],
    );
    assert!(!refused.ok);
    assert!(
        refused
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("person"),
        "{refused:?}"
    );
    // And nothing moved.
    assert_eq!(held.owner("disagreements", vec![]).n, 1);
}

#[test]
fn keeping_one_retires_the_other_and_closes_the_disagreement() {
    let mut held = Held::new();
    let (a, b) = held.disagreeing();
    let done = held.owner(
        "settle",
        vec![
            json!(a.to_string()),
            json!(b.to_string()),
            json!({"keep": a.to_string()}),
        ],
    );
    assert!(done.ok, "{done:?}");
    assert_eq!(done.result[0]["kept"], a.to_string());
    assert_eq!(done.result[0]["retired"], b.to_string());
    assert_eq!(held.owner("disagreements", vec![]).n, 0);
}

#[test]
fn reconciling_closes_it_with_both_left_standing() {
    let mut held = Held::new();
    let (a, b) = held.disagreeing();
    let done = held.owner(
        "settle",
        vec![
            json!(a.to_string()),
            json!(b.to_string()),
            json!({"reconcile": true}),
        ],
    );
    assert!(done.ok, "{done:?}");
    assert_eq!(held.owner("disagreements", vec![]).n, 0);
    for id in [&a, &b] {
        let why = held.owner("why", vec![json!(id.to_string())]);
        assert_eq!(
            why.result[0]["against"].as_array().map(Vec::len),
            Some(0),
            "nothing should still be pulling against {id}"
        );
    }
}

#[test]
fn neither_being_right_retires_both_and_keeps_them_findable() {
    let mut held = Held::new();
    let (a, b) = held.disagreeing();
    let done = held.owner(
        "settle",
        vec![
            json!(a.to_string()),
            json!(b.to_string()),
            json!({"neither": true}),
        ],
    );
    assert!(done.ok, "{done:?}");
    assert_eq!(held.owner("disagreements", vec![]).n, 0);
    for id in [&a, &b] {
        let memory = held.store.get(id).expect("read").expect("memory");
        assert!(memory.archived_at.is_some(), "{id} is still asserted");
    }
}

#[test]
fn a_settlement_has_to_say_which_way_and_name_one_of_the_two() {
    let mut held = Held::new();
    let (a, b) = held.disagreeing();
    let both = held.owner(
        "settle",
        vec![
            json!(a.to_string()),
            json!(b.to_string()),
            json!({"keep": a.to_string(), "reconcile": true}),
        ],
    );
    assert!(!both.ok, "two answers at once is not an answer");
    let neither_said = held.owner(
        "settle",
        vec![json!(a.to_string()), json!(b.to_string()), json!({})],
    );
    assert!(!neither_said.ok);
    let stranger = held.owner(
        "settle",
        vec![
            json!(a.to_string()),
            json!(b.to_string()),
            json!({"keep": "01NOTAMEMORY"}),
        ],
    );
    assert!(!stranger.ok, "keeping something that is not one of the two");
    let itself = held.owner(
        "settle",
        vec![
            json!(a.to_string()),
            json!(a.to_string()),
            json!({"reconcile": true}),
        ],
    );
    assert!(!itself.ok);
    assert_eq!(held.owner("disagreements", vec![]).n, 1, "nothing moved");
}

#[test]
fn a_memory_this_project_does_not_hold_cannot_be_settled() {
    let mut held = Held::new();
    let (a, _) = held.disagreeing();
    let missing = held.owner(
        "settle",
        vec![
            json!(a.to_string()),
            json!("01M0000000000000000000NOPE"),
            json!({"keep": a.to_string()}),
        ],
    );
    assert!(!missing.ok);
    assert!(
        missing
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("no memory"),
        "{missing:?}"
    );
}
