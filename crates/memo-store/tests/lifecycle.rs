//! Tie    let befo    let after = store.all().expect("all").len();e = store.all().expect("all").len();-aware lifecycle, held to §9.8 of the future plan.
//!
//! The acceptance criteria are mostly negative: truth must not move because something helped,
//! habit statistics must not move without attribution, and no lifecycle path may delete.

use memo_model::{
    Attribution, Body, Environment, Memory, NoteKind, OutcomeKind, Presentation, Record, ScopeId,
    SessionId, Standing, Tier, Witness, WitnessId, WitnessKind,
};
use memo_store::{Injection, Store, Use, Verdict, mint};

const NOW: memo_model::Timestamp = 1_756_000_000;
const DAY: memo_model::Timestamp = 86_400;

fn scope() -> ScopeId {
    ScopeId::new("/w/thing")
}

fn kept(store: &mut Store, tier: Tier, text: &str) -> memo_model::MemoryId {
    let held = Memory::new(
        mint(NOW),
        tier,
        scope(),
        Body::note(text, NoteKind::Claim),
        NOW,
    );
    let id = held.id.clone();
    store
        .remember(
            held,
            Witness::new(
                WitnessId::new(format!("w-{text}")),
                WitnessKind::Imperative,
                SessionId::new("01RUN"),
                scope(),
                NOW,
            ),
            NOW,
        )
        .expect("remember");
    // A memory somebody typed arrives pinned, and a pinned memory ignores every decay rule.
    // Unpinning is the point of the test: these are about how things fade.
    store.pin(&id, false, NOW).expect("unpin");
    id
}

#[test]
fn a_fact_outlasts_an_episode_in_the_store() {
    // The whole of tier-aware lifecycle, measured rather than asserted: after a month of
    // nobody asking, a claim about the world is still stronger than a record of an afternoon.
    let mut store = Store::ephemeral().expect("store");
    let fact = kept(&mut store, Tier::Fact, "the deploy target is fly.io");
    let episode = kept(&mut store, Tier::Episode, "spent the morning on the build");

    store.weaken(NOW + 30 * DAY).expect("weaken");

    let fact_now = store
        .get(&fact)
        .expect("get")
        .expect("there")
        .strength
        .value;
    let episode_now = store
        .get(&episode)
        .expect("get")
        .expect("there")
        .strength
        .value;
    assert!(
        fact_now > episode_now,
        "fact {fact_now:.4} did not outlast episode {episode_now:.4}"
    );
}

#[test]
fn scratch_fades_fastest() {
    // Otherwise the notes of a thousand runs compete with the project's own memory.
    let mut store = Store::ephemeral().expect("store");
    let scratch = kept(&mut store, Tier::Scratch, "trying the staging box");
    let episode = kept(&mut store, Tier::Episode, "spent the morning on the build");

    store.weaken(NOW + 14 * DAY).expect("weaken");

    let scratch_now = store
        .get(&scratch)
        .expect("get")
        .expect("there")
        .strength
        .value;
    let episode_now = store
        .get(&episode)
        .expect("get")
        .expect("there")
        .strength
        .value;
    assert!(
        scratch_now < episode_now,
        "{scratch_now:.4} vs {episode_now:.4}"
    );
}

#[test]
fn using_a_fact_badly_does_not_make_it_less_true() {
    // §9.8's first criterion. Truth confidence never changes solely because a fact helped or
    // hurt a task — and the ledger is where that is decided, so this is the test that keeps the
    // two apart when both are live in one store.
    let mut store = Store::ephemeral().expect("store");
    let id = kept(&mut store, Tier::Fact, "the deploy target is fly.io");
    let before = store.get(&id).expect("get").expect("there").confidence;

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
            &[(id.clone(), Presentation::Asserted)],
        )
        .expect("inject");
    for n in 0..5 {
        let action = format!("a{n}");
        store
            .note_use(&Use {
                id: action.clone(),
                injection: Some("i1".to_owned()),
                session: Some(SessionId::new("01RUN")),
                reported_at: NOW + n,
                tool: Some("shell".to_owned()),
                action_hash: "h".to_owned(),
                attribution: Attribution::Explicit,
                memories: vec![id.clone()],
            })
            .expect("use");
        store
            .note_outcome(&Verdict {
                id: format!("{action}-o"),
                action,
                observed_at: NOW + n,
                kind: OutcomeKind::Failed,
                score: None,
                evidence_cursor: None,
                evaluator: "caller".to_owned(),
                note: None,
            })
            .expect("outcome");
    }

    let after = store.get(&id).expect("get").expect("there").confidence;
    assert!(
        (before - after).abs() < f64::EPSILON,
        "five failures moved truth: {before} -> {after}"
    );
    let utility = store.utility_of(&id).expect("utility");
    assert_eq!(utility.verified_harmful, 5, "and the harm was recorded");
}

#[test]
fn habit_statistics_move_only_on_attributable_outcomes() {
    // §9.8's second criterion, as arithmetic. Proximal evidence is not attribution, and a
    // procedure that gained authority from proximity would gain it from being popular.
    let mut record = Record::default();
    record.succeeded();
    assert_eq!(record.standing(false, false), Standing::Advisory);

    record.succeeded();
    assert_eq!(record.standing(false, false), Standing::Established);

    // A failure does not erase the history; it lowers the ratio and holds the standing back.
    record.failed();
    assert_eq!(record.tried, 3);
    assert_eq!(record.worked, 2);
    assert_eq!(record.standing(false, true), Standing::Advisory);
}

#[test]
fn a_changed_environment_suspends_rather_than_archives() {
    // §9.6: what was learned did not stop being true of the machine it was learned on.
    let learned = Environment {
        scope: Some("/w/thing".to_owned()),
        os: Some("linux".to_owned()),
        arch: Some("x86_64".to_owned()),
        ..Environment::default()
    };
    let elsewhere = Environment {
        scope: Some("/w/other".to_owned()),
        os: Some("windows".to_owned()),
        arch: Some("aarch64".to_owned()),
        ..Environment::default()
    };
    let record = Record {
        tried: 9,
        worked: 9,
    };

    assert!(elsewhere.has_moved_from(&learned));
    assert_eq!(record.standing(true, false), Standing::Suspended);
    assert!(!Standing::Suspended.may_offer(), "not offered");
    assert!(
        record.success().expect("some") > 0.9,
        "and its record is untouched"
    );
}

#[test]
fn no_lifecycle_path_removes_a_memory() {
    // §9.8's fourth criterion. Everything fades into the archive and stays readable.
    let mut store = Store::ephemeral().expect("store");
    let id = kept(&mut store, Tier::Scratch, "something transient");
    let before = store.all().expect("all").len();

    store.weaken(NOW + 400 * DAY).expect("weaken");
    store.sweep(NOW + 400 * DAY).expect("sweep");

    let after = store.all().expect("all").len();
    assert_eq!(before, after, "a lifecycle pass removed a row");

    let held = store.get(&id).expect("get").expect("still there");
    assert_eq!(held.tier, Tier::Archive, "it moved to the archive");
    assert!(held.archived_at.is_some());
}

#[test]
fn a_pinned_memory_ignores_every_tier_rule() {
    let mut store = Store::ephemeral().expect("store");
    let id = kept(&mut store, Tier::Scratch, "keep this whatever happens");
    store.pin(&id, true, NOW).expect("pin");

    store.weaken(NOW + 400 * DAY).expect("weaken");
    let held = store.get(&id).expect("get").expect("there");
    assert!((held.strength.value - 1.0).abs() < f64::EPSILON);
    assert_eq!(held.tier, Tier::Scratch, "and it did not move");
}

#[test]
fn staleness_is_about_validity_not_about_being_ignored() {
    // §9.2. A fact nobody has needed for a year is not stale; a fact whose interval closed is.
    assert!(!memo_model::is_stale(NOW - 200 * DAY, None, NOW));
    assert!(memo_model::is_stale(NOW - 10 * DAY, Some(NOW - DAY), NOW));
}
