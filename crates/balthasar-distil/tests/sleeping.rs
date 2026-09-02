//! M7's CALLUS and SLEEP: what crosses from a session into a project, and why.

use balthasar_distil::{DISTINCT_SESSIONS, consolidate};
use balthasar_lua::Settings;
use balthasar_model::{
    Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, WitnessKind, floor,
};
use balthasar_store::{Store, mint};

const MARCH: Timestamp = 1_710_000_000;
const AUGUST: Timestamp = 1_756_000_000;

fn scope() -> ScopeId {
    ScopeId::new("/w/thing")
}

/// One run saying one thing.
fn said(store: &mut Store, session: &str, text: &str, at: Timestamp) {
    let mut memory = Memory::new(
        mint(at),
        Tier::Scratch,
        scope(),
        Body::note(text, NoteKind::Observation),
        at,
    );
    memory.session = Some(SessionId::new(session));
    memory.temporal = balthasar_model::Temporal::recalled(at, at);
    store.keep_scratch(memory).expect("scratch");
}

fn run(store: &mut Store, now: Timestamp) -> balthasar_distil::Consolidated {
    consolidate(store, None, &Settings::default(), &scope(), now, false).expect("consolidate")
}

#[test]
fn one_run_repeating_itself_is_not_corroboration() {
    // A person being emphatic. Promoting it would let a single session install whatever it
    // said most often into a project every future session reads.
    let mut store = Store::ephemeral().expect("store");
    for at in [MARCH, MARCH + 60, MARCH + 120] {
        said(&mut store, "s1", "the database is postgres", at);
    }
    let report = run(&mut store, AUGUST);
    assert!(report.promoted.is_empty(), "{:?}", report.promoted);
}

#[test]
fn two_unrelated_runs_agreeing_crosses() {
    // The same thing surfacing in runs that knew nothing of each other is a property of the
    // world rather than of one conversation.
    let mut store = Store::ephemeral().expect("store");
    said(&mut store, "s1", "the database is postgres", MARCH);
    said(&mut store, "s2", "the database is postgres", MARCH + 86_400);

    let report = run(&mut store, AUGUST);
    assert_eq!(report.promoted.len(), 1, "{report:?}");
    assert_eq!(report.promoted[0], "the database is postgres");
}

#[test]
fn what_crossed_belongs_to_the_project_and_names_its_witnesses() {
    let mut store = Store::ephemeral().expect("store");
    for (session, at) in [
        ("s1", MARCH),
        ("s2", MARCH + 86_400),
        ("s3", MARCH + 172_800),
    ] {
        said(&mut store, session, "the database is postgres", at);
    }
    run(&mut store, AUGUST);

    let promoted = store
        .live_slot("/w/thing", "", "")
        .ok()
        .flatten()
        .or_else(|| store.all().ok()?.into_iter().find(|m| m.tier == Tier::Fact))
        .expect("something crossed");

    assert_eq!(promoted.tier, Tier::Fact);
    assert_eq!(promoted.scope, scope(), "it is the project's now");
    assert_eq!(
        promoted.distinct_sessions(),
        3,
        "one witness per run that saw it"
    );
    assert!(
        promoted
            .witnesses
            .iter()
            .all(|w| w.kind == WitnessKind::Repetition)
    );
}

#[test]
fn more_runs_agreeing_is_more_confidence() {
    // Diversity, end to end. The number a promoted claim lands with is a function of how many
    // unrelated runs saw it, not of how loudly any one of them said it.
    let confidence = |sessions: &[&str]| {
        let mut store = Store::ephemeral().expect("store");
        for (n, session) in sessions.iter().enumerate() {
            said(
                &mut store,
                session,
                "the database is postgres",
                MARCH + n as Timestamp * 86_400,
            );
        }
        run(&mut store, AUGUST);
        store
            .all()
            .expect("export")
            .into_iter()
            .find(|m| m.tier == Tier::Fact)
            .expect("something crossed")
            .confidence
    };

    assert!(confidence(&["s1", "s2", "s3", "s4"]) > confidence(&["s1", "s2"]));
}

#[test]
fn what_crossed_is_dated_from_when_it_was_first_seen() {
    // A thing learned in March and confirmed in August did not become true this afternoon.
    let mut store = Store::ephemeral().expect("store");
    said(&mut store, "s1", "the database is postgres", MARCH);
    said(&mut store, "s2", "the database is postgres", AUGUST);
    run(&mut store, AUGUST + 3600);

    let promoted = store
        .all()
        .expect("export")
        .into_iter()
        .find(|m| m.tier == Tier::Fact)
        .expect("something crossed");
    assert_eq!(promoted.temporal.valid_from, MARCH);
}

#[test]
fn a_second_pass_adds_nothing() {
    // A pass that ran nightly and added a copy of everything each time would fill a project
    // with restatements and make confidence meaningless.
    //
    // It must not keep *reinforcing* either. This once asserted a reinforcement on the second
    // pass, which only happened because the sweep was broken: spent scratch was never archived,
    // so every nightly run found the same two observations again and agreed with itself. One
    // pair of observations must be worth one crossing however many times the machine is idle.
    let mut store = Store::ephemeral().expect("store");
    said(&mut store, "s1", "the database is postgres", MARCH);
    said(&mut store, "s2", "the database is postgres", MARCH + 86_400);

    run(&mut store, AUGUST);
    let after_first = store.all().expect("export").len();
    let crossed = store
        .all()
        .expect("export")
        .into_iter()
        .find(|m| m.tier == Tier::Fact)
        .expect("something crossed");

    let second = run(&mut store, AUGUST + 3600);
    assert_eq!(second.promoted.len(), 0, "nothing crossed twice");
    assert_eq!(
        store.all().expect("export").len(),
        after_first,
        "and nothing was added"
    );

    let again = store.get(&crossed.id).expect("get").expect("still there");
    assert!(
        (again.confidence - crossed.confidence).abs() < f64::EPSILON,
        "an idle machine agreed with itself: {} -> {}",
        crossed.confidence,
        again.confidence
    );
}

#[test]
fn a_dry_run_writes_nothing() {
    let mut store = Store::ephemeral().expect("store");
    said(&mut store, "s1", "the database is postgres", MARCH);
    said(&mut store, "s2", "the database is postgres", MARCH + 86_400);

    let before = store.all().expect("export").len();
    let report = consolidate(
        &mut store,
        None,
        &Settings::default(),
        &scope(),
        AUGUST,
        true,
    )
    .expect("preview");
    assert!(report.dry_run);
    assert_eq!(report.promoted.len(), 1, "it still says what it would do");
    assert_eq!(
        store.all().expect("export").len(),
        before,
        "and does none of it"
    );
}

#[test]
fn two_projects_do_not_corroborate_each_other() {
    // Durable memory is the project's. A thing said once here and once somewhere else is not
    // a thing this project has learned.
    let mut store = Store::ephemeral().expect("store");
    said(&mut store, "s1", "the database is postgres", MARCH);

    let mut elsewhere = Memory::new(
        mint(MARCH),
        Tier::Scratch,
        ScopeId::new("/w/other"),
        Body::note("the database is postgres", NoteKind::Observation),
        MARCH,
    );
    elsewhere.session = Some(SessionId::new("s2"));
    store.keep_scratch(elsewhere).expect("scratch");

    let report = run(&mut store, AUGUST);
    assert!(report.promoted.is_empty(), "{:?}", report.promoted);
}

#[test]
fn what_crossed_is_asserted_and_what_did_not_is_merely_findable() {
    let mut store = Store::ephemeral().expect("store");
    said(&mut store, "s1", "the database is postgres", AUGUST);
    said(&mut store, "s2", "the database is postgres", AUGUST + 60);
    said(&mut store, "s1", "a passing remark", AUGUST);
    run(&mut store, AUGUST + 120);

    let held = store.all().expect("export");
    let promoted = held
        .iter()
        .find(|m| m.tier == Tier::Fact)
        .expect("something crossed");
    assert!(
        promoted.is_assertable(floor::INJECT, AUGUST + 120, false),
        "confidence {}",
        promoted.confidence
    );
    assert!(
        held.iter().any(|m| m.tier == Tier::Scratch),
        "and the rest stayed where it was"
    );
}

#[test]
fn the_bar_is_two_runs() {
    assert_eq!(DISTINCT_SESSIONS, 2);
}

#[test]
fn two_runs_wording_one_claim_differently_corroborate() {
    // What CALLUS could not do before: corroboration was an exact digest, so a claim said two
    // ways was two claims said once each — and neither reached the bar.
    let mut store = Store::ephemeral().expect("store");
    said(&mut store, "s1", "we use make test", MARCH);
    said(&mut store, "s2", "run make test instead", MARCH + 86_400);

    let report = run(&mut store, AUGUST);
    assert_eq!(report.promoted.len(), 1, "{:?}", report.promoted);

    // Both runs are witnesses, which is what makes the diversity real rather than asserted.
    let kept = store.all().expect("all");
    let fact = kept.iter().find(|m| m.tier == Tier::Fact).expect("a fact");
    let why = store.witnesses_of(&fact.id).expect("witnesses");
    assert_eq!(why.len(), 2);
    assert!(why.iter().all(|w| w.kind == WitnessKind::Repetition));

    // And the note says how it was matched. A rewording judged to be one claim is weaker
    // evidence than a repeat, and `balthasar why` must not present them as the same thing.
    assert!(
        why.iter()
            .any(|w| w.note.as_deref().is_some_and(|n| n.contains("reworded"))),
        "the note admits it was a rewording: {:?}",
        why.iter().map(|w| w.note.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn a_claim_and_its_replacement_never_corroborate_each_other() {
    // The failure that would make this whole change unsafe. These two runs disagree, and a
    // store that read them as agreeing would assert a deploy target on the strength of a run
    // that said the opposite.
    let mut store = Store::ephemeral().expect("store");
    said(&mut store, "s1", "we deploy with fly.io", MARCH);
    said(&mut store, "s2", "we deploy with heroku", MARCH + 86_400);

    let report = run(&mut store, AUGUST);
    assert!(
        report.promoted.is_empty(),
        "neither is corroborated: {:?}",
        report.promoted
    );
}

#[test]
fn one_run_saying_it_two_ways_is_still_one_run() {
    // Merging must not manufacture the very diversity it is counted for. A person restating
    // themselves is the case CALLUS exists to refuse.
    let mut store = Store::ephemeral().expect("store");
    said(&mut store, "s1", "we use make test", MARCH);
    said(&mut store, "s1", "run make test instead", MARCH + 60);

    let report = run(&mut store, AUGUST);
    assert!(report.promoted.is_empty(), "{:?}", report.promoted);
    assert_eq!(DISTINCT_SESSIONS, 2, "the bar is unchanged");
}

#[test]
fn a_promoted_fact_records_what_it_was_made_from() {
    // Without this edge a purge of the scratch leaves the fact standing with nothing pointing
    // at where it came from — and the next pass writes the claim back out of the survivor.
    let mut store = Store::ephemeral().expect("store");
    said(&mut store, "s1", "the database is postgres", MARCH);
    said(&mut store, "s2", "the database is postgres", MARCH + 86_400);

    let report = run(&mut store, AUGUST);
    assert_eq!(report.promoted.len(), 1, "{:?}", report.promoted);

    let all = store.all().expect("all");
    let fact = all.iter().find(|m| m.tier == Tier::Fact).expect("a fact");
    let links = store.links_of(&fact.id).expect("links");
    let derived: Vec<_> = links
        .iter()
        .filter(|l| l.rel == balthasar_model::LinkRelation::DerivedFrom)
        .collect();
    assert_eq!(
        derived.len(),
        2,
        "one edge per scratch it was made from: {links:?}"
    );
}

#[test]
fn purging_the_scratch_takes_the_fact_made_from_it() {
    // The cascade. Erasing a claim has to reach what the claim became, or the compliance record
    // says it was removed while a consolidation pass rewrites it tomorrow.
    let mut store = Store::ephemeral().expect("store");
    said(
        &mut store,
        "s1",
        "the deploy token is hunter2-nevershare",
        MARCH,
    );
    said(
        &mut store,
        "s2",
        "the deploy token is hunter2-nevershare",
        MARCH + 86_400,
    );
    run(&mut store, AUGUST);

    // The sweep at the end of a pass archives spent scratch, so the source is found through the
    // edge rather than by tier — which is the point of recording the edge at all.
    let all = store.all().expect("all");
    let fact = all.iter().find(|m| m.tier == Tier::Fact).expect("a fact");
    let scratch = store
        .links_of(&fact.id)
        .expect("links")
        .into_iter()
        .find(|l| l.rel == balthasar_model::LinkRelation::DerivedFrom)
        .expect("it records what it was made from")
        .to;

    // The prompt has to say so before anything is removed.
    let closure = balthasar_store::closure_of(&store, &scratch).expect("closure");
    assert_eq!(closure.derived, 1, "one belief goes with it");

    balthasar_store::purge(&mut store, &scratch).expect("purge");

    let left = store.all().expect("all");
    assert!(
        !left.iter().any(|m| m.tier == Tier::Fact),
        "the belief made out of it went with it: {:?}",
        left.iter().map(|m| (m.tier, m.text())).collect::<Vec<_>>()
    );
    // The other run's scratch is a different memory and nobody asked for it. Taking a sibling
    // along would be a larger operation than the one requested — `balthasar forget --session` is the
    // verb for that, and it is a different verb on purpose.
    assert_eq!(left.len(), 1, "and only its own descendants went");
}

#[test]
fn a_cycle_in_the_derivation_graph_terminates() {
    // Nothing writes a cycle today, but a graph that can be walked has to be walked safely —
    // the alternative is a purge that never returns.
    let mut store = Store::ephemeral().expect("store");
    said(&mut store, "s1", "a claim that recurs", MARCH);
    said(&mut store, "s2", "a claim that recurs", MARCH + 86_400);
    run(&mut store, AUGUST);

    let all = store.all().expect("all");
    let a = &all[0].id;
    let b = &all[1].id;
    store
        .link(a, b, balthasar_model::LinkRelation::DerivedFrom, AUGUST)
        .expect("edge");
    store
        .link(b, a, balthasar_model::LinkRelation::DerivedFrom, AUGUST)
        .expect("the other way");

    assert!(balthasar_store::purge(&mut store, a).is_ok(), "it returns");
}
