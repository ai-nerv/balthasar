//! Saying a thing again is confirming it; fading is measured from when it was last needed.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::{
    Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
    floor,
};
use balthasar_store::{Recall, Store, mint};

const T0: Timestamp = 1_700_000_000;
const DAY: Timestamp = 86_400;

fn scope() -> ScopeId {
    ScopeId::new("/w/thing")
}

fn saw(run: &str, at: Timestamp) -> Witness {
    Witness::new(
        WitnessId::new(format!("w-{run}")),
        WitnessKind::Imperative,
        SessionId::new(run),
        scope(),
        at,
    )
}

fn held(store: &mut Store, text: &str, at: Timestamp) -> balthasar_model::MemoryId {
    let memory = Memory::new(
        mint(at),
        Tier::Fact,
        scope(),
        Body::note(text, NoteKind::Claim),
        at,
    );
    store
        .remember(memory, saw("first", at), at)
        .expect("remembered")
        .id()
        .clone()
}

#[test]
fn a_fact_confirmed_weekly_stays_asserted_past_a_year() {
    let mut store = Store::ephemeral().expect("store");
    let id = held(&mut store, "we release from the main branch", T0);
    let mut at = T0;
    for week in 1..=60 {
        at = T0 + week * 7 * DAY;
        store
            .attach(&id, saw(&format!("week{week}"), at), at)
            .expect("confirmed");
    }
    let memory = store.get(&id).expect("read").expect("there");
    assert!(at - T0 > 400 * DAY);
    assert!(
        memory.is_assertable(floor::INJECT, at + DAY, false),
        "confidence {}, confirmed {:?}",
        memory.confidence,
        memory.temporal.confirmed_at
    );
}

#[test]
fn a_fact_never_confirmed_goes_stale_after_a_year() {
    let mut store = Store::ephemeral().expect("store");
    let id = held(&mut store, "we release from the main branch", T0);
    let memory = store.get(&id).expect("read").expect("there");
    assert!(!memory.is_assertable(floor::INJECT, T0 + 400 * DAY, false));
}

#[test]
fn the_same_run_saying_it_again_confirms_nothing() {
    let mut store = Store::ephemeral().expect("store");
    let id = held(&mut store, "we release from the main branch", T0);
    let mut again = saw("first", T0 + 30 * DAY);
    again.id = WitnessId::new("w-again");
    store.attach(&id, again, T0 + 30 * DAY).expect("again");
    let memory = store.get(&id).expect("read").expect("there");
    assert_eq!(memory.temporal.confirmed_at, None);
}

#[test]
fn decay_does_not_move_last_accessed() {
    let mut store = Store::ephemeral().expect("store");
    let id = held(&mut store, "the cache lives in target", T0);
    store.decay(T0 + 40 * DAY).expect("decayed");
    store.decay(T0 + 80 * DAY).expect("decayed again");
    let memory = store.get(&id).expect("read").expect("there");
    assert_eq!(memory.strength.last_accessed, T0);
    let fresh = Memory::new(
        mint(T0),
        Tier::Fact,
        scope(),
        Body::note("unused", NoteKind::Claim),
        T0,
    );
    let expected = fresh.strength.at_tier(Tier::Fact, T0 + 80 * DAY);
    let stored = memory.strength.at_tier(Tier::Fact, T0 + 80 * DAY);
    assert!(
        (stored - expected).abs() < 1e-9,
        "{stored} against {expected}"
    );
}

#[test]
fn recall_strength_matches_decay_strength() {
    let mut store = Store::ephemeral().expect("store");
    held(&mut store, "the cache lives in target", T0);
    let at = T0 + 90 * DAY;
    let hit = store
        .recall(&Recall::of("cache", at))
        .expect("recall")
        .into_iter()
        .next()
        .expect("found");
    let expected = hit.memory.strength.at_tier(hit.memory.tier, at);
    assert!((hit.strength - expected).abs() < 1e-12);
}

#[test]
fn health_sees_a_missing_index_row_and_reindex_mends_it() {
    let dir = balthasar_model::scratch::Scratch::new("balthasar-health", "index");
    let path = dir.join("memory.db");
    let mut store = Store::open(&path).expect("store");
    held(&mut store, "the cache lives in target", T0);
    let checks = balthasar_store::health(&store, None).expect("health");
    assert!(
        balthasar_store::health_summary(&checks).ends_with("ok"),
        "{checks:?}"
    );
    rusqlite::Connection::open(&path)
        .and_then(|raw| raw.execute("DELETE FROM memory_fts", []))
        .expect("index row gone");
    let checks = balthasar_store::health(&store, None).expect("health");
    let summary = balthasar_store::health_summary(&checks);
    assert!(summary.contains("RED"), "{summary}");
    assert!(summary.contains("balthasar reindex --text"), "{summary}");
    store.reindex_text().expect("reindexed");
    let checks = balthasar_store::health(&store, None).expect("health");
    assert!(!balthasar_store::health_summary(&checks).contains("RED"));
}
