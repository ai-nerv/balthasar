//! The whole loop, as a harness would drive it.
//!
//! Recall hands memories over and says which injection they were. The harness acts. It reports
//! what it ran and how it went. memo works out for itself whether the action followed anything
//! it was given — and only then is there a labelled example to learn from.
//!
//! This is the path that turns an empty `outcome` column into training data, so every step of
//! it is worth a test.

use memo_host::{Answering, Door, answer};
use memo_ipc::{Peer, Reply, Request};
use memo_model::{
    Body, Memory, NoteKind, OutcomeKind, ScopeId, SessionId, Tier, Witness, WitnessId, WitnessKind,
    floor,
};
use memo_store::{Store, mint};

const NOW: memo_model::Timestamp = 1_756_000_000;

struct Harness {
    store: Store,
    capture: bool,
}

impl Harness {
    fn new(capture: bool) -> Self {
        let mut store = Store::ephemeral().expect("store");
        for (n, text) in [
            "run the tests with `make test`, never `cargo test`",
            "the office wifi is flaky",
        ]
        .iter()
        .enumerate()
        {
            let held = Memory::new(
                mint(NOW + n as i64),
                Tier::Fact,
                ScopeId::new("/w/thing"),
                Body::note(*text, NoteKind::Claim),
                NOW,
            );
            store
                .remember(
                    held,
                    Witness::new(
                        WitnessId::new(format!("w{n}")),
                        WitnessKind::Imperative,
                        SessionId::new("01RUN"),
                        ScopeId::new("/w/thing"),
                        NOW,
                    ),
                    NOW,
                )
                .expect("remember");
        }
        Self { store, capture }
    }

    fn ask(&mut self, name: &str, args: Vec<serde_json::Value>) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: None,
            scratch: None,
            scope: ScopeId::new("/w/thing"),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: self.capture,
        };
        answer(
            &mut at,
            &Door::Socket(Peer {
                pid: 42,
                uid: 1000,
                program: Some("/usr/bin/some-harness".to_owned()),
            }),
            &Request {
                call: name.to_owned(),
                args,
            },
        )
    }

    fn field(reply: &Reply, name: &str) -> Option<String> {
        reply
            .result
            .as_ref()?
            .first()?
            .get(name)?
            .as_str()
            .map(str::to_owned)
    }
}

#[test]
fn a_recall_hands_back_the_injection_it_made() {
    // Without this the harness has nothing to report against, and every outcome it observes is
    // unattributable.
    let mut held = Harness::new(true);
    let reply = held.ask("recall", vec![serde_json::json!("how do we run tests")]);

    assert!(reply.ok, "{reply:?}");
    let injection = Harness::field(&reply, "injection").expect("an injection id");
    assert!(injection.starts_with("inject-"), "{injection}");
    assert!(
        !held.store.injected_in(&injection).expect("read").is_empty(),
        "and it holds what was handed over"
    );
}

#[test]
fn nothing_is_recorded_when_capture_is_off() {
    // The default. A memory layer that started keeping a trail of what somebody searched for
    // because a new version shipped is not one anybody should install.
    let mut held = Harness::new(false);
    let reply = held.ask("recall", vec![serde_json::json!("how do we run tests")]);

    assert!(reply.ok, "{reply:?}");
    assert!(Harness::field(&reply, "injection").is_none());
    // And the shape a caller sees is the old one: a bare list.
    assert!(
        reply
            .result
            .as_ref()
            .and_then(|v| v.first())
            .is_some_and(serde_json::Value::is_array),
        "the reply shape changed for callers that did not ask for a ledger"
    );
}

#[test]
fn an_action_that_followed_a_memory_attributes_itself() {
    // The centre of it. The harness says what it ran; memo works out that the memory naming
    // `make test` was followed, without the harness having to claim it.
    let mut held = Harness::new(true);
    let served = held.ask("recall", vec![serde_json::json!("how do we run tests")]);
    let injection = Harness::field(&served, "injection").expect("injection");

    let used = held.ask(
        "used",
        vec![
            serde_json::json!(injection),
            serde_json::json!({ "tool": "shell", "action": "make test" }),
        ],
    );
    assert!(used.ok, "{used:?}");
    let action = Harness::field(&used, "action").expect("action");

    let recorded = held
        .store
        .use_of(&action)
        .expect("read")
        .expect("an action");
    assert_eq!(
        recorded.attribution,
        memo_model::Attribution::Structural,
        "memo did not see the match for itself"
    );
    assert_eq!(recorded.memories.len(), 1, "and only the one it matched");
}

#[test]
fn an_unrelated_action_credits_nothing() {
    // The other half. Running something the memories never mentioned must not attribute to
    // whatever happened to be injected.
    let mut held = Harness::new(true);
    let served = held.ask("recall", vec![serde_json::json!("how do we run tests")]);
    let injection = Harness::field(&served, "injection").expect("injection");

    let used = held.ask(
        "used",
        vec![
            serde_json::json!(injection),
            serde_json::json!({ "tool": "shell", "action": "git push --force" }),
        ],
    );
    let action = Harness::field(&used, "action").expect("action");
    let recorded = held
        .store
        .use_of(&action)
        .expect("read")
        .expect("an action");

    assert!(recorded.memories.is_empty(), "it credited something");
    assert_eq!(recorded.attribution, memo_model::Attribution::Proximal);
}

#[test]
fn the_loop_produces_a_labelled_example() {
    // What all of this is for: an exported row with an outcome in it, which is the thing the
    // trainer cannot work without.
    let mut held = Harness::new(true);
    let served = held.ask("recall", vec![serde_json::json!("how do we run tests")]);
    let injection = Harness::field(&served, "injection").expect("injection");

    let used = held.ask(
        "used",
        vec![
            serde_json::json!(injection),
            serde_json::json!({ "tool": "shell", "action": "make test" }),
        ],
    );
    let action = Harness::field(&used, "action").expect("action");
    let ended = held.ask(
        "outcome",
        vec![
            serde_json::json!(action),
            serde_json::json!({ "kind": "succeeded" }),
        ],
    );
    assert!(ended.ok, "{ended:?}");

    let rows = held.store.training_rows(100).expect("export");
    let labelled: Vec<_> = rows.iter().filter(|r| r.outcome.is_some()).collect();
    assert!(
        !labelled.is_empty(),
        "no labelled row came out of a closed loop"
    );
    assert_eq!(labelled[0].outcome.as_deref(), Some("succeeded"));
    assert_eq!(labelled[0].attribution.as_deref(), Some("structural"));
}

#[test]
fn a_failure_is_a_label_too() {
    // Half the training signal. A loop that only recorded successes would teach a model that
    // everything works.
    let mut held = Harness::new(true);
    let served = held.ask("recall", vec![serde_json::json!("how do we run tests")]);
    let injection = Harness::field(&served, "injection").expect("injection");
    let used = held.ask(
        "used",
        vec![
            serde_json::json!(injection),
            serde_json::json!({ "tool": "shell", "action": "make test" }),
        ],
    );
    let action = Harness::field(&used, "action").expect("action");
    held.ask(
        "outcome",
        vec![
            serde_json::json!(action),
            serde_json::json!({ "kind": "failed" }),
        ],
    );

    let memory = held.store.injected_in(&injection).expect("read")[0].clone();
    let utility = held.store.utility_of(&memory).expect("utility");
    assert_eq!(utility.verified_harmful, 1);
    assert_eq!(utility.helpfulness(), Some(0.0));
}

#[test]
fn closing_the_loop_never_moves_confidence() {
    // The separation, checked at the level a harness actually drives. Utility flows; truth does
    // not, however the acting went.
    let mut held = Harness::new(true);
    let before: Vec<f64> = held
        .store
        .all()
        .expect("all")
        .iter()
        .map(|m| m.confidence)
        .collect();

    for kind in [OutcomeKind::Succeeded, OutcomeKind::Failed] {
        let served = held.ask("recall", vec![serde_json::json!("how do we run tests")]);
        let injection = Harness::field(&served, "injection").expect("injection");
        let used = held.ask(
            "used",
            vec![
                serde_json::json!(injection),
                serde_json::json!({ "tool": "shell", "action": "make test" }),
            ],
        );
        let action = Harness::field(&used, "action").expect("action");
        held.ask(
            "outcome",
            vec![
                serde_json::json!(action),
                serde_json::json!({ "kind": kind.as_str() }),
            ],
        );
    }

    let after: Vec<f64> = held
        .store
        .all()
        .expect("all")
        .iter()
        .map(|m| m.confidence)
        .collect();
    assert_eq!(
        before, after,
        "acting on a memory moved what is believed about it"
    );
}
