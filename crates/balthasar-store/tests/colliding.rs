//! What an arriving claim collides with: a habit is its trigger, a failed step replaces the habit
//! naming it, and every stored confidence is what its evidence argues.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::{
    Body, Memory, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
};
use balthasar_store::{Landing, Store, mint};

const MARCH: Timestamp = 1_710_000_000;
const AUGUST: Timestamp = 1_756_000_000;

fn store() -> Store {
    Store::ephemeral().expect("a store")
}

fn fact(subject: &str, predicate: &str, object: &str, at: Timestamp) -> Memory {
    Memory::new(
        mint(at),
        Tier::Fact,
        ScopeId::global(),
        Body::fact(subject, predicate, object),
        at,
    )
}

fn note(text: &str, at: Timestamp) -> Memory {
    Memory::new(
        mint(at),
        Tier::Fact,
        ScopeId::global(),
        Body::note(text, balthasar_model::NoteKind::Claim),
        at,
    )
}

/// A repair, as the rules write one: what failed, and what worked instead.
fn repaired(failed: &str, worked: &str, at: Timestamp) -> Memory {
    Memory::new(
        mint(at),
        Tier::Habit,
        ScopeId::global(),
        Body::habit(
            format!("when `{failed}` is what you would reach for"),
            vec![worked.to_owned()],
        ),
        at,
    )
}

fn cost(run: &str, at: Timestamp) -> Witness {
    Witness::new(
        WitnessId::new(format!("w-{run}")),
        WitnessKind::Cost,
        SessionId::new(run),
        ScopeId::global(),
        at,
    )
}

#[test]
fn a_repair_whose_failure_is_a_habits_step_replaces_that_habit() {
    let mut store = store();
    let old = store
        .remember(
            repaired("npm run deploy", "make deploy", MARCH),
            cost("s1", MARCH),
            MARCH,
        )
        .expect("first")
        .id()
        .clone();
    let landed = store
        .remember(
            repaired("make deploy", "just deploy", AUGUST),
            cost("s2", AUGUST),
            AUGUST,
        )
        .expect("second");
    assert!(matches!(landed, Landing::Superseded { .. }), "{landed:?}");
    let closed = store.get(&old).expect("get").expect("kept");
    assert!(
        closed.temporal.valid_to.is_some(),
        "the failed step is no longer current"
    );
}

#[test]
fn repairs_of_different_commands_are_different_habits() {
    let mut store = store();
    let tests = store
        .remember(
            repaired("cargo test", "make test", MARCH),
            cost("s1", MARCH),
            MARCH,
        )
        .expect("tests")
        .id()
        .clone();
    let landed = store
        .remember(
            repaired("cargo build", "make build", AUGUST),
            cost("s2", AUGUST),
            AUGUST,
        )
        .expect("build");
    assert!(matches!(landed, Landing::Added(_)), "{landed:?}");
    let held = store.get(&tests).expect("get").expect("kept");
    assert!(held.temporal.valid_to.is_none());
    assert_eq!(
        store.witnesses_of(&tests).expect("why").len(),
        1,
        "not reinforced"
    );
}

#[test]
fn every_stored_confidence_is_what_a_fresh_rescore_gives() {
    let mut store = store();
    let saw = |kind: WitnessKind, run: &str| {
        Witness::new(
            WitnessId::new(format!("w-{run}-{kind}")),
            kind,
            SessionId::new(run),
            ScopeId::global(),
            AUGUST,
        )
    };
    let codec = |value: &str| fact("transport", "codec", value, AUGUST);
    store
        .remember(
            codec("protobuf"),
            saw(WitnessKind::Imperative, "s1"),
            AUGUST,
        )
        .expect("1");
    store
        .remember(codec("cbor"), saw(WitnessKind::Correction, "s2"), AUGUST)
        .expect("2");
    let claim = note("we deploy with fly.io", AUGUST);
    let id = store
        .remember(claim, saw(WitnessKind::Imperative, "s3"), AUGUST)
        .expect("3")
        .id()
        .clone();
    store
        .attach(&id, saw(WitnessKind::Repetition, "s4"), AUGUST)
        .expect("4");
    store
        .remember(
            note("we deploy with render", AUGUST),
            saw(WitnessKind::Repetition, "s5"),
            AUGUST,
        )
        .expect("5");
    store
        .remember(
            repaired("cargo test", "make test", AUGUST),
            cost("s6", AUGUST),
            AUGUST,
        )
        .expect("6");
    store
        .remember(
            repaired("make test", "just test", AUGUST),
            cost("s7", AUGUST),
            AUGUST,
        )
        .expect("7");
    let gone = store
        .remember(
            note("the old box was 10.0.0.9", AUGUST),
            saw(WitnessKind::Imperative, "s8"),
            AUGUST,
        )
        .expect("8")
        .id()
        .clone();
    store.archive(&gone, AUGUST).expect("archived");
    balthasar_store::purge_session(&mut store, &SessionId::new("s4"), AUGUST).expect("purged");

    for memory in store.all().expect("all") {
        let stored = memory.confidence;
        let fresh = store.rescore(&memory.id, AUGUST).expect("rescore");
        assert!(
            (stored - fresh).abs() < 1e-9,
            "{}: {stored} stored, {fresh} argued",
            memory.text()
        );
    }
}
