//! M1: what fades, what does not, and what happens to it when it does.

use aeon_model::{
    Body, Importance, Memory, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
    floor,
};
use aeon_store::{Recall, Store, mint};

const MARCH: Timestamp = 1_710_000_000;
const DAY: Timestamp = 86_400;

fn store() -> Store {
    Store::ephemeral().expect("a store")
}

/// A fact stated once, weakly, in March.
fn stated_once(store: &mut Store, what: &str, importance: Importance, at: Timestamp) -> Memory {
    let mut memory = Memory::new(
        mint(at),
        Tier::Fact,
        ScopeId::global(),
        Body::fact("project", what, "something"),
        at,
    );
    memory.strength.importance = importance;
    let witness = Witness::new(
        WitnessId::new(format!("w-{what}")),
        WitnessKind::Distillation,
        SessionId::new("s1"),
        ScopeId::global(),
        at,
    );
    let landing = store.remember(memory, witness, at).expect("remember");
    store.get(landing.id()).expect("get").expect("there")
}

#[test]
fn a_thing_stated_once_is_findable_but_never_asserted() {
    // The two floors, end to end. One distillation is worth keeping and is not worth stating.
    let mut store = store();
    let memory = stated_once(&mut store, "guess", Importance::Normal, MARCH);

    assert!(
        !memory.is_assertable(floor::INJECT, MARCH, false),
        "confidence {} should be under the assertion floor",
        memory.confidence
    );
    assert!(
        memory.confidence >= floor::LIVE,
        "confidence {} should still be worth keeping",
        memory.confidence
    );

    let found = store.recall(&Recall::of("guess", MARCH)).expect("recall");
    assert_eq!(found.len(), 1, "it is still found by an explicit search");
}

#[test]
fn a_preview_writes_nothing() {
    // Forgetting is the one thing a person cannot undo by asking again, so the rehearsal has
    // to be a genuine rehearsal.
    let mut store = store();
    let memory = stated_once(&mut store, "passing", Importance::Low, MARCH);
    let later = MARCH + 400 * DAY;

    let preview = store.decay_preview(later).expect("preview");
    assert!(!preview.is_empty(), "there is something to fade");
    assert!(preview.preview);

    let after = store.get(&memory.id).expect("get").expect("there");
    assert_eq!(after.strength.value, memory.strength.value);
    assert!(after.archived_at.is_none());
}

#[test]
fn what_fades_past_the_floor_is_archived_rather_than_removed() {
    let mut store = store();
    let memory = stated_once(&mut store, "passing", Importance::Low, MARCH);
    let later = MARCH + 400 * DAY;

    let report = store.decay(later).expect("decay");
    assert_eq!(
        report.swept.len(),
        1,
        "a year of neglect at 7-day half-life"
    );

    let after = store.get(&memory.id).expect("get").expect("there");
    assert!(after.archived_at.is_some());
    assert_eq!(after.tier, Tier::Archive);
    // Nothing is deleted. Everything about it survives the sweep.
    assert_eq!(after.witnesses.len(), 1);
    assert_eq!(store.all().expect("export").len(), 1);
}

#[test]
fn a_pinned_memory_survives_any_amount_of_neglect() {
    let mut store = store();
    let memory = stated_once(&mut store, "identity", Importance::Low, MARCH);
    store.pin(&memory.id, true, MARCH).expect("pin");

    let report = store.decay(MARCH + 10_000 * DAY).expect("decay");
    assert!(report.is_empty());
    assert_eq!(report.pinned, 1);

    let after = store.get(&memory.id).expect("get").expect("there");
    assert!(after.archived_at.is_none());
    assert_eq!(after.strength.at(MARCH + 10_000 * DAY), 1.0);
}

#[test]
fn what_is_kept_reaching_for_outlives_what_is_not() {
    // Conceptual inertia, end to end: two facts of the same importance, one of them used.
    let mut store = store();
    let ignored = stated_once(&mut store, "ignored", Importance::Normal, MARCH);
    let used = stated_once(&mut store, "used", Importance::Normal, MARCH);

    for week in 1..40 {
        store
            .touch(&used.id, MARCH + week * 7 * DAY)
            .expect("touch");
    }

    let later = MARCH + 300 * DAY;
    let ignored = store.get(&ignored.id).expect("get").expect("there");
    let used = store.get(&used.id).expect("get").expect("there");
    assert!(
        used.strength.at(later) > ignored.strength.at(later),
        "used {} should outlast ignored {}",
        used.strength.at(later),
        ignored.strength.at(later)
    );
}

#[test]
fn a_second_pass_in_the_same_moment_does_nothing() {
    // A pass that reported every memory as "weakened by 0.00" would make the report useless
    // for the thing it exists for.
    let mut store = store();
    stated_once(&mut store, "thing", Importance::Normal, MARCH);
    let later = MARCH + 30 * DAY;

    assert!(!store.decay(later).expect("first").is_empty());
    assert!(store.decay(later).expect("second").is_empty());
}

#[test]
fn importance_decides_what_outlives_what() {
    let mut store = store();
    let passing = stated_once(&mut store, "passing", Importance::Low, MARCH);
    let build = stated_once(&mut store, "build", Importance::High, MARCH);

    store.decay(MARCH + 200 * DAY).expect("decay");
    let passing = store.get(&passing.id).expect("get").expect("there");
    let build = store.get(&build.id).expect("get").expect("there");

    assert!(passing.archived_at.is_some(), "a passing remark retires");
    assert!(
        build.archived_at.is_none(),
        "how the project builds does not"
    );
}
