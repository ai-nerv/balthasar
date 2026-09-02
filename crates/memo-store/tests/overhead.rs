//! What the ledger costs, and what a shadow policy is not allowed to touch.
//!
//! §6.11 asks for ledger overhead to be measured and bounded rather than assumed, and §11.6 for
//! shadow policies to be unable to move strength or access counts. Both are properties nobody
//! notices breaking until a memory has quietly been reinforced by an experiment.

use memo_model::{
    Attribution, Body, Memory, NoteKind, Presentation, ScopeId, SessionId, Tier, Witness,
    WitnessId, WitnessKind,
};
use memo_store::{Candidate, Injection, Recall, RecallRun, Signals, Store, Use, mint};

const NOW: memo_model::Timestamp = 1_756_000_000;

fn scope() -> ScopeId {
    ScopeId::new("/w/thing")
}

fn a_store_of(n: usize) -> (Store, Vec<memo_model::MemoryId>) {
    let mut store = Store::ephemeral().expect("store");
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let held = Memory::new(
            mint(NOW + i as i64),
            Tier::Fact,
            scope(),
            Body::note(
                format!("memory number {i} about deploying things"),
                NoteKind::Claim,
            ),
            NOW,
        );
        let id = held.id.clone();
        store
            .remember(
                held,
                Witness::new(
                    WitnessId::new(format!("w{i}")),
                    WitnessKind::Imperative,
                    SessionId::new("01RUN"),
                    scope(),
                    NOW,
                ),
                NOW,
            )
            .expect("remember");
        ids.push(id);
    }
    (store, ids)
}

#[test]
fn the_ledger_costs_a_bounded_amount_of_room() {
    // §6.11. One row per candidate per search is the design; what matters is that it stays
    // proportional to searches rather than growing with the store.
    let (mut store, ids) = a_store_of(50);
    let empty = store.bytes().expect("bytes");

    for n in 0..100 {
        let candidates: Vec<Candidate> = ids
            .iter()
            .take(10)
            .enumerate()
            .map(|(rank, memory)| Candidate {
                memory: memory.clone(),
                rank,
                selected: rank < 5,
                score: 0.5,
                signals: Signals::default(),
            })
            .collect();
        store
            .note_recall(
                &RecallRun {
                    id: format!("r{n}"),
                    scope: scope(),
                    session: Some(SessionId::new("01RUN")),
                    query_hash: "hash".to_owned(),
                    requested_at: NOW + n,
                    config_fingerprint: "cfg".to_owned(),
                    vector_available: false,
                    result_limit: 10,
                    latency_us: 100,
                },
                &candidates,
            )
            .expect("note");
    }

    let full = store.bytes().expect("bytes");
    let per_search = (full - empty) as f64 / 100.0;
    assert!(
        per_search < 4096.0,
        "a search costs {per_search:.0} bytes of ledger — the bound is one page"
    );
}

#[test]
fn recording_a_search_does_not_reinforce_anything() {
    // The load-bearing one. If writing the ledger touched strength, then instrumenting a system
    // would change the thing being instrumented, and every measurement after that is about the
    // measuring.
    let (mut store, ids) = a_store_of(3);
    let before: Vec<(f64, f64)> = ids
        .iter()
        .map(|id| {
            let held = store.get(id).expect("get").expect("there");
            (held.strength.value, held.confidence)
        })
        .collect();

    for n in 0..20 {
        store
            .note_recall(
                &RecallRun {
                    id: format!("r{n}"),
                    scope: scope(),
                    session: Some(SessionId::new("01RUN")),
                    query_hash: "hash".to_owned(),
                    requested_at: NOW + n,
                    config_fingerprint: "cfg".to_owned(),
                    vector_available: false,
                    result_limit: 10,
                    latency_us: 100,
                },
                &ids.iter()
                    .enumerate()
                    .map(|(rank, memory)| Candidate {
                        memory: memory.clone(),
                        rank,
                        selected: true,
                        score: 0.9,
                        signals: Signals::default(),
                    })
                    .collect::<Vec<_>>(),
            )
            .expect("note");
    }

    let after: Vec<(f64, f64)> = ids
        .iter()
        .map(|id| {
            let held = store.get(id).expect("get").expect("there");
            (held.strength.value, held.confidence)
        })
        .collect();
    assert_eq!(before, after, "twenty recorded searches moved a memory");
}

#[test]
fn a_search_that_is_not_served_touches_nothing() {
    // §11.6: a shadow policy computes and is discarded. Recall does not reinforce by default —
    // this is the test that keeps it that way, because a shadow that reinforced would let an
    // experiment decide what survives.
    let (store, ids) = a_store_of(3);
    let before: Vec<(f64, u32)> = ids
        .iter()
        .map(|id| {
            let held = store.get(id).expect("get").expect("there");
            (held.strength.value, held.strength.access_count)
        })
        .collect();

    for _ in 0..20 {
        let mut ask = Recall::of("deploying things", NOW);
        ask.limit = 10;
        ask.floor = 0.0;
        ask.scope_name = scope().to_string();
        let found = store.recall(&ask).expect("recall");
        assert!(!found.is_empty(), "the shadow found something to not count");
    }

    let after: Vec<(f64, u32)> = ids
        .iter()
        .map(|id| {
            let held = store.get(id).expect("get").expect("there");
            (held.strength.value, held.strength.access_count)
        })
        .collect();
    assert_eq!(before, after, "searching moved strength or an access count");
}

#[test]
fn the_ledger_can_be_dropped_without_touching_a_memory() {
    // The other half of "bounded": whatever it costs, it can be reclaimed, and reclaiming it
    // leaves the store believing exactly what it believed.
    let (mut store, ids) = a_store_of(5);
    store
        .note_injection(
            &Injection {
                id: "i1".to_owned(),
                recall: None,
                session: Some(SessionId::new("01RUN")),
                created_at: NOW,
                token_count: 10,
                remote: false,
                policy: "balanced".to_owned(),
            },
            &[(ids[0].clone(), Presentation::Asserted)],
        )
        .expect("inject");
    store
        .note_use(&Use {
            id: "a1".to_owned(),
            injection: Some("i1".to_owned()),
            session: Some(SessionId::new("01RUN")),
            reported_at: NOW,
            tool: None,
            action_hash: "h".to_owned(),
            attribution: Attribution::Explicit,
            memories: vec![ids[0].clone()],
        })
        .expect("use");

    let before: Vec<String> = store.all().expect("all").iter().map(Memory::text).collect();
    store
        .forget_ledger_before(NOW + 365 * 86_400)
        .expect("forget");
    let after: Vec<String> = store.all().expect("all").iter().map(Memory::text).collect();

    assert_eq!(before, after);
    assert_eq!(store.all().expect("all").len(), 5);
}
