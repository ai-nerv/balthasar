//! Relationship views end to end, held to §8.9 of the future plan.
//!
//! The acceptance criteria are mostly about what must keep working: SQLite only, three families
//! surviving with no embedder, rebuilds that leave memories alone, and bounds that hold on a
//! store dense enough to hurt.

use balthasar_distil::{Step, Thresholds, entities, overlap, repairs, temporal};
use balthasar_model::{
    Body, Derivation, Family, Memory, MemoryId, NoteKind, ScopeId, SessionId, Temporal, Tier, View,
    Witness, WitnessId, WitnessKind,
};
use balthasar_store::{Reach, Store, mint};

const NOW: balthasar_model::Timestamp = 1_756_000_000;

fn scope() -> ScopeId {
    ScopeId::new("/w/thing")
}

/// A store holding a failure, its repair, and an unrelated note.
fn a_repair_story() -> (Store, MemoryId, MemoryId, MemoryId) {
    let mut store = Store::ephemeral().expect("store");
    let mut ids = Vec::new();
    for (n, text) in [
        "cargo test failed: no such command",
        "make test worked",
        "the office wifi is flaky",
    ]
    .iter()
    .enumerate()
    {
        let at = NOW + n as i64 * 10;
        let mut held = Memory::new(
            mint(at),
            Tier::Fact,
            scope(),
            Body::note(*text, NoteKind::Claim),
            at,
        );
        held.temporal = Temporal::recalled(at, at);
        let id = held.id.clone();
        store
            .remember(
                held,
                Witness::new(
                    WitnessId::new(format!("w{n}")),
                    WitnessKind::Imperative,
                    SessionId::new("01RUN"),
                    scope(),
                    at,
                ),
                at,
            )
            .expect("remember");
        ids.push(id);
    }
    (store, ids[0].clone(), ids[1].clone(), ids[2].clone())
}

#[test]
fn a_causal_query_reaches_the_fix_from_the_failure() {
    // The question the whole family exists for: "why did this fail / what fixed it" has to
    // return the repair when the search only matched the failure.
    let (mut store, failure, fix, _) = a_repair_story();
    let edges = repairs(
        &[
            Step {
                memory: failure.clone(),
                episode: "e1".to_owned(),
                command: "cargo test".to_owned(),
                failed: true,
                cursor: Some(1),
            },
            Step {
                memory: fix.clone(),
                episode: "e1".to_owned(),
                command: "make test".to_owned(),
                failed: false,
                cursor: Some(2),
            },
        ],
        NOW,
    );
    store.relate(&edges).expect("relate");

    let found = store
        .traverse(
            std::slice::from_ref(&failure),
            &[Family::Causal],
            &Reach::default(),
        )
        .expect("walk");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(found[0].0, fix);
    assert_eq!(found[0].1.view, View::Caused);
    assert_eq!(found[0].1.source, Derivation::Structure);
}

#[test]
fn the_unrelated_memory_is_not_dragged_in() {
    // A traversal that returned everything would beat search by being indiscriminate.
    let (mut store, failure, fix, wifi) = a_repair_story();
    store
        .relate(&repairs(
            &[
                Step {
                    memory: failure.clone(),
                    episode: "e1".to_owned(),
                    command: "cargo test".to_owned(),
                    failed: true,
                    cursor: Some(1),
                },
                Step {
                    memory: fix,
                    episode: "e1".to_owned(),
                    command: "make test".to_owned(),
                    failed: false,
                    cursor: Some(2),
                },
            ],
            NOW,
        ))
        .expect("relate");

    let found = store
        .traverse(&[failure], &[Family::Causal], &Reach::default())
        .expect("walk");
    assert!(!found.iter().any(|(id, _)| *id == wifi));
}

#[test]
fn three_families_still_answer_with_no_embedder() {
    // §8.9: `MAGI_BALTHASAR_NO_EMBED=1` retains temporal, causal-rule and entity traversal. Nothing in
    // this test can reach an embedder, because none of these derivations can.
    let (mut store, failure, fix, wifi) = a_repair_story();
    let held = store.all().expect("all");

    store
        .relate(&temporal(&held, &Thresholds::default(), NOW))
        .expect("temporal");
    store
        .relate(&repairs(
            &[
                Step {
                    memory: failure.clone(),
                    episode: "e1".to_owned(),
                    command: "cargo test".to_owned(),
                    failed: true,
                    cursor: Some(1),
                },
                Step {
                    memory: fix.clone(),
                    episode: "e1".to_owned(),
                    command: "make test".to_owned(),
                    failed: false,
                    cursor: Some(2),
                },
            ],
            NOW,
        ))
        .expect("causal");
    store
        .relate(&entities(
            &[
                (failure.clone(), vec!["test".to_owned()]),
                (fix.clone(), vec!["test".to_owned()]),
            ],
            &|_| 0.7,
            &Thresholds::default(),
            NOW,
        ))
        .expect("entity");

    for family in [Family::Temporal, Family::Causal, Family::Entity] {
        assert!(family.needs_no_embedder());
        let found = store
            .traverse(std::slice::from_ref(&failure), &[family], &Reach::default())
            .expect("walk");
        assert!(!found.is_empty(), "{family} returned nothing");
    }
    let _ = wifi;
}

#[test]
fn the_semantic_floor_needs_no_vectors_either() {
    // Not a family that survives — a family that has a floor. Exact overlap is what answers
    // when cosine is unavailable, so nothing downstream has to branch on the embedder.
    let (mut store, failure, fix, _) = a_repair_story();
    store
        .relate(&overlap(
            &[
                (failure.clone(), "make test is how we run tests".to_owned()),
                (fix.clone(), "run tests with make test".to_owned()),
            ],
            &Thresholds::default(),
            NOW,
        ))
        .expect("overlap");

    let found = store
        .traverse(&[failure], &[Family::Semantic], &Reach::default())
        .expect("walk");
    assert_eq!(found.len(), 1, "{found:#?}");
    assert_eq!(
        found[0].1.source,
        Derivation::Rule,
        "no embedder was involved"
    );
}

#[test]
fn rebuilding_a_derivation_leaves_every_memory_untouched() {
    // §8.9: derived relations can be rebuilt without changing memories or witnesses.
    let (mut store, failure, fix, _) = a_repair_story();
    let before: Vec<(MemoryId, f64, usize)> = store
        .all()
        .expect("all")
        .into_iter()
        .map(|m| {
            let witnesses = store.witnesses_of(&m.id).expect("witnesses").len();
            (m.id, m.confidence, witnesses)
        })
        .collect();

    store
        .relate(&entities(
            &[
                (failure, vec!["test".to_owned()]),
                (fix, vec!["test".to_owned()]),
            ],
            &|_| 0.7,
            &Thresholds::default(),
            NOW,
        ))
        .expect("relate");
    store
        .retire_relations(
            Derivation::Rule,
            balthasar_distil::RELATION_DERIVATION,
            NOW + 1,
        )
        .expect("retire");

    let after: Vec<(MemoryId, f64, usize)> = store
        .all()
        .expect("all")
        .into_iter()
        .map(|m| {
            let witnesses = store.witnesses_of(&m.id).expect("witnesses").len();
            (m.id, m.confidence, witnesses)
        })
        .collect();
    assert_eq!(before, after, "a rebuild moved a memory");
}

/// A store with `edges` edges out of one hub, and the hub.
fn a_hub_of(edges: usize) -> (Store, MemoryId) {
    let mut store = Store::ephemeral().expect("store");
    let hub = MemoryId::new("hub");
    let spokes: Vec<balthasar_model::Relation> = (0..edges)
        .map(|n| balthasar_model::Relation {
            from: hub.clone(),
            to: MemoryId::new(format!("m{n}")),
            view: View::SameEntity,
            weight: 0.9,
            source: Derivation::Rule,
            derivation_version: 1,
            evidence_cursor: None,
            created_at: NOW,
        })
        .collect();
    store.relate(&spokes).expect("relate");
    (store, hub)
}

/// The quickest of three walks out of a hub of `edges` edges, in microseconds.
fn walking_a_hub_of(edges: usize) -> f64 {
    let (store, hub) = a_hub_of(edges);
    // Once first, so the page cache and the prepared statement are not part of the measurement.
    store
        .traverse(
            std::slice::from_ref(&hub),
            &[Family::Entity],
            &Reach::default(),
        )
        .expect("warm");
    (0..3)
        .map(|_| {
            let started = std::time::Instant::now();
            let found = store
                .traverse(
                    std::slice::from_ref(&hub),
                    &[Family::Entity],
                    &Reach::default(),
                )
                .expect("walk");
            assert!(
                found.len() <= Reach::default().fan_out,
                "{} came back",
                found.len()
            );
            started.elapsed().as_secs_f64() * 1e6
        })
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_dense_store_stays_within_its_bounds() {
    // §8.9: relationship candidates stay bounded however dense the store is. The bound is what
    // comes back, and it is `fan_out` whether one memory has eight edges or eight thousand.
    let (store, hub) = a_hub_of(2000);
    let found = store
        .traverse(&[hub], &[Family::Entity], &Reach::default())
        .expect("walk");
    assert!(
        found.len() <= Reach::default().fan_out,
        "{} came back",
        found.len()
    );
}

#[test]
fn a_walk_costs_what_it_returns_rather_than_what_the_hub_holds() {
    // A ratio rather than a wall-clock budget: a loaded machine inflates both measurements, so
    // it cancels. Walking every edge would land near 8; the bar sits well under it.
    let few = walking_a_hub_of(500);
    let many = walking_a_hub_of(4000);

    let grew = many / few;
    assert!(
        grew < 3.0,
        "eight times the edges cost {grew:.2}x the walk ({few:.0}us -> {many:.0}us); \
         the traversal is reading edges it does not return"
    );
}

#[test]
#[ignore = "wall-clock; grades the machine, not the code. Run it to read the number."]
fn a_walk_stays_inside_its_declared_budget() {
    let took = walking_a_hub_of(2000);
    assert!(took < 50_000.0, "took {took:.0}us on a dense store");
}

#[test]
fn the_census_says_what_the_index_holds() {
    let (mut store, failure, fix, _) = a_repair_story();
    store
        .relate(&entities(
            &[
                (failure, vec!["test".to_owned()]),
                (fix, vec!["test".to_owned()]),
            ],
            &|_| 0.7,
            &Thresholds::default(),
            NOW,
        ))
        .expect("relate");

    let census = store.relation_census().expect("census");
    let entity = census.iter().find(|(view, _)| *view == View::SameEntity);
    assert_eq!(
        entity.map(|(_, n)| *n),
        Some(2),
        "both directions: {census:?}"
    );
}
