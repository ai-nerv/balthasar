//! The use-and-outcome ledger, held to §6.10 of the future plan.
//!
//! Each test here is one of the named requirements. They are written as the plan states them,
//! because the value of the ledger is entirely in what it refuses to conclude.

use memo_model::{
    Attribution, Body, Memory, MemoryId, NoteKind, OutcomeKind, Presentation, ScopeId, SessionId,
    Tier, Witness, WitnessId, WitnessKind,
};
use memo_store::{Candidate, Injection, RecallRun, Signals, Store, Use, Verdict, mint};

const NOW: memo_model::Timestamp = 1_756_000_000;
const DAY: memo_model::Timestamp = 86_400;

fn scope() -> ScopeId {
    ScopeId::new("/w/thing")
}

fn run() -> SessionId {
    SessionId::new("01RUN")
}

/// A store holding one memory, and its id.
fn one_memory(text: &str) -> (Store, MemoryId) {
    let mut store = Store::ephemeral().expect("store");
    let held = Memory::new(
        mint(NOW),
        Tier::Fact,
        scope(),
        Body::note(text, NoteKind::Claim),
        NOW,
    );
    let id = held.id.clone();
    let witness = Witness::new(
        WitnessId::new("w1"),
        WitnessKind::Imperative,
        run(),
        scope(),
        NOW,
    );
    store.remember(held, witness, NOW).expect("remember");
    (store, id)
}

fn searched(store: &mut Store, id: &MemoryId, recall: &str, selected: bool) {
    store
        .note_recall(
            &RecallRun {
                id: recall.to_owned(),
                scope: scope(),
                session: Some(run()),
                query_hash: "hash-of-a-query".to_owned(),
                requested_at: NOW,
                config_fingerprint: "cfg".to_owned(),
                vector_available: false,
                result_limit: 10,
                latency_us: 120,
            },
            &[Candidate {
                memory: id.clone(),
                rank: 0,
                selected,
                score: 0.8,
                signals: Signals::default(),
            }],
        )
        .expect("note recall");
}

fn injected(store: &mut Store, id: &MemoryId, injection: &str, recall: Option<&str>) {
    store
        .note_injection(
            &Injection {
                id: injection.to_owned(),
                recall: recall.map(str::to_owned),
                session: Some(run()),
                created_at: NOW,
                token_count: 24,
                remote: false,
                policy: "balanced".to_owned(),
            },
            &[(id.clone(), Presentation::Asserted)],
        )
        .expect("note injection");
}

fn acted(store: &mut Store, id: &MemoryId, action: &str, injection: &str, how: Attribution) {
    store
        .note_use(&Use {
            id: action.to_owned(),
            injection: Some(injection.to_owned()),
            session: Some(run()),
            reported_at: NOW + 60,
            tool: Some("shell".to_owned()),
            action_hash: "hash-of-an-action".to_owned(),
            attribution: how,
            memories: vec![id.clone()],
        })
        .expect("note use");
}

fn ended(store: &mut Store, action: &str, kind: OutcomeKind) {
    store
        .note_outcome(&Verdict {
            id: format!("{action}-outcome"),
            action: action.to_owned(),
            observed_at: NOW + 120,
            kind,
            score: None,
            evidence_cursor: Some(7),
            evaluator: "caller".to_owned(),
            note: None,
        })
        .expect("note outcome");
}

#[test]
fn a_retrieved_but_unselected_memory_gets_no_use_evidence() {
    // Considering something is not using it. Otherwise every candidate in every search would
    // accumulate evidence about its own worth.
    let (mut store, id) = one_memory("the deploy target is fly.io");
    searched(&mut store, &id, "r1", false);

    let held = store.utility_of(&id).expect("utility");
    assert!(!held.is_verified());
    assert_eq!(held.verified_helpful, 0);
    assert_eq!(held.verified_harmful, 0);

    let (considered, selected) = store.times_retrieved(&id).expect("retrieved");
    assert_eq!((considered, selected), (1, 0));
}

#[test]
fn an_injected_but_ignored_memory_is_ignored_and_not_failed() {
    // The distinction the whole ledger exists for. A memory that was shown and not used tells
    // you about the ranking; calling it a failure would blame the memory for being surfaced.
    let (mut store, id) = one_memory("the deploy target is fly.io");
    searched(&mut store, &id, "r1", true);
    injected(&mut store, &id, "i1", Some("r1"));
    acted(&mut store, &id, "a1", "i1", Attribution::Explicit);
    ended(&mut store, "a1", OutcomeKind::Ignored);

    let held = store.utility_of(&id).expect("utility");
    assert_eq!(held.ignored, 1);
    assert_eq!(held.verified_harmful, 0, "ignored is not harm");
    assert!(!held.is_verified());
}

#[test]
fn a_failed_explicit_use_is_harm_without_touching_truth() {
    // The load-bearing separation: utility evidence must never move confidence. A fact can be
    // perfectly true and harmful to act on.
    let (mut store, id) = one_memory("the deploy target is fly.io");
    let before = store.get(&id).expect("get").expect("there").confidence;

    searched(&mut store, &id, "r1", true);
    injected(&mut store, &id, "i1", Some("r1"));
    acted(&mut store, &id, "a1", "i1", Attribution::Explicit);
    ended(&mut store, "a1", OutcomeKind::Failed);

    let held = store.utility_of(&id).expect("utility");
    assert_eq!(held.verified_harmful, 1);
    assert_eq!(held.helpfulness(), Some(0.0));

    let after = store.get(&id).expect("get").expect("there").confidence;
    assert!(
        (before - after).abs() < f64::EPSILON,
        "confidence moved: {before} -> {after}"
    );
}

#[test]
fn a_successful_action_that_named_no_memory_credits_nothing() {
    // Something good happening after an injection is not evidence the injection helped.
    let (mut store, id) = one_memory("the deploy target is fly.io");
    searched(&mut store, &id, "r1", true);
    injected(&mut store, &id, "i1", Some("r1"));
    store
        .note_use(&Use {
            id: "a1".to_owned(),
            injection: Some("i1".to_owned()),
            session: Some(run()),
            reported_at: NOW + 60,
            tool: Some("shell".to_owned()),
            action_hash: "unrelated".to_owned(),
            attribution: Attribution::Explicit,
            memories: Vec::new(),
        })
        .expect("note use");
    ended(&mut store, "a1", OutcomeKind::Succeeded);

    let held = store.utility_of(&id).expect("utility");
    assert_eq!(held.verified_helpful, 0, "it named no memory");
    assert!(!held.is_verified());
}

#[test]
fn proximity_alone_is_recorded_but_never_counted() {
    // Do not infer causality from temporal adjacency. The observation is kept for analysis and
    // kept out of the counters that decide what a memory may claim about itself.
    let (mut store, id) = one_memory("the deploy target is fly.io");
    searched(&mut store, &id, "r1", true);
    injected(&mut store, &id, "i1", Some("r1"));
    acted(&mut store, &id, "a1", "i1", Attribution::Proximal);
    ended(&mut store, "a1", OutcomeKind::Succeeded);

    let held = store.utility_of(&id).expect("utility");
    assert_eq!(held.proximal, 1, "it was recorded");
    assert_eq!(held.verified_helpful, 0, "and not counted");
    assert!(!held.is_verified());
}

#[test]
fn an_unknown_outcome_never_becomes_a_failure() {
    // Time passing is not evidence. A memory used a year ago with nobody reporting is still
    // unknown, and a system that decayed it into failure would punish silence.
    let (mut store, id) = one_memory("the deploy target is fly.io");
    searched(&mut store, &id, "r1", true);
    injected(&mut store, &id, "i1", Some("r1"));
    acted(&mut store, &id, "a1", "i1", Attribution::Explicit);

    let held = store.utility_of(&id).expect("utility");
    assert_eq!(held.unknown, 1);
    assert_eq!(held.verified_harmful, 0);
    assert_eq!(held.helpfulness(), None, "no evidence is not bad evidence");
}

#[test]
fn replaying_an_outcome_is_idempotent() {
    // A caller replaying its own event log must not double-count itself into confidence.
    let (mut store, id) = one_memory("the deploy target is fly.io");
    searched(&mut store, &id, "r1", true);
    injected(&mut store, &id, "i1", Some("r1"));
    acted(&mut store, &id, "a1", "i1", Attribution::Explicit);
    ended(&mut store, "a1", OutcomeKind::Succeeded);
    ended(&mut store, "a1", OutcomeKind::Succeeded);
    ended(&mut store, "a1", OutcomeKind::Succeeded);

    let held = store.utility_of(&id).expect("utility");
    assert_eq!(held.verified_helpful, 1, "one action, one outcome");
}

#[test]
fn the_ledger_holds_no_query_and_no_action_arguments() {
    // The privacy rule as a test rather than a promise. Hashes and cursors are enough to trace;
    // a copy of what was said is not.
    let (mut store, id) = one_memory("the deploy target is fly.io");
    searched(&mut store, &id, "r1", true);
    injected(&mut store, &id, "i1", Some("r1"));
    acted(&mut store, &id, "a1", "i1", Attribution::Explicit);
    ended(&mut store, "a1", OutcomeKind::Succeeded);

    let trace = store.trace_of("r1").expect("trace").expect("there");
    assert_eq!(trace.query_hash, "hash-of-a-query");
    assert!(!trace.query_hash.contains(' '), "a hash, not a sentence");
    assert_eq!(trace.considered.len(), 1);
    assert_eq!(trace.actions.len(), 1);
    assert_eq!(trace.actions[0].outcome, OutcomeKind::Succeeded);
}

#[test]
fn every_injected_memory_can_be_traced_back_to_its_recall() {
    // §6.11's first acceptance criterion.
    let (mut store, id) = one_memory("the deploy target is fly.io");
    searched(&mut store, &id, "r1", true);
    injected(&mut store, &id, "i1", Some("r1"));

    let trace = store.trace_of("r1").expect("trace").expect("there");
    let (found, rank, selected, score) = &trace.considered[0];
    assert_eq!(found, &id);
    assert_eq!(*rank, 0);
    assert!(*selected);
    assert!(*score > 0.0, "and what it scored");
}

#[test]
fn popularity_and_usefulness_are_different_numbers() {
    // §6.11's second criterion, at the store level: a memory retrieved constantly with no
    // attributed outcome must be distinguishable from one that actually helped.
    let (mut store, popular) = one_memory("a memory everything matches");
    let helpful = {
        let held = Memory::new(
            mint(NOW + 1),
            Tier::Fact,
            scope(),
            Body::note("the memory that worked", NoteKind::Claim),
            NOW,
        );
        let id = held.id.clone();
        let witness = Witness::new(
            WitnessId::new("w2"),
            WitnessKind::Imperative,
            run(),
            scope(),
            NOW,
        );
        store.remember(held, witness, NOW).expect("remember");
        id
    };

    for n in 0..20 {
        searched(&mut store, &popular, &format!("r{n}"), true);
    }
    searched(&mut store, &helpful, "rh", true);
    injected(&mut store, &helpful, "ih", Some("rh"));
    acted(&mut store, &helpful, "ah", "ih", Attribution::Explicit);
    ended(&mut store, "ah", OutcomeKind::Succeeded);

    assert_eq!(store.times_retrieved(&popular).expect("retrieved").0, 20);
    assert!(
        !store.utility_of(&popular).expect("utility").is_verified(),
        "twenty retrievals are not evidence"
    );
    assert!(
        store.utility_of(&helpful).expect("utility").is_verified(),
        "one attributed success is"
    );
}

#[test]
fn forgetting_the_ledger_leaves_every_memory_standing() {
    // Retention removes telemetry, not memory. A store whose whole ledger has aged out still
    // believes exactly what it believed, with exactly the same evidence.
    let (mut store, id) = one_memory("the deploy target is fly.io");
    let before = store.get(&id).expect("get").expect("there").confidence;
    let witnesses = store.witnesses_of(&id).expect("witnesses").len();

    searched(&mut store, &id, "r1", true);
    injected(&mut store, &id, "i1", Some("r1"));
    acted(&mut store, &id, "a1", "i1", Attribution::Explicit);
    ended(&mut store, "a1", OutcomeKind::Succeeded);

    let gone = store.forget_ledger_before(NOW + 90 * DAY).expect("forget");
    assert!(gone > 0, "something aged out");
    assert!(store.trace_of("r1").expect("trace").is_none());

    let after = store.get(&id).expect("get").expect("still there");
    assert!((before - after.confidence).abs() < f64::EPSILON);
    assert_eq!(store.witnesses_of(&id).expect("witnesses").len(), witnesses);
    assert_eq!(after.text(), "the deploy target is fly.io");
}

#[test]
fn retention_keeps_what_is_still_inside_the_window() {
    let (mut store, id) = one_memory("the deploy target is fly.io");
    searched(&mut store, &id, "r1", true);

    let gone = store.forget_ledger_before(NOW - DAY).expect("forget");
    assert_eq!(gone, 0, "nothing was old enough");
    assert!(store.trace_of("r1").expect("trace").is_some());
}
