//! M7's CALLUS and SLEEP: what crosses from a session into a project, and why.

use aeon_distil::{DISTINCT_SESSIONS, consolidate};
use aeon_lua::Settings;
use aeon_model::{Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, WitnessKind, floor};
use aeon_store::{Store, mint};

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
    memory.temporal = aeon_model::Temporal::recalled(at, at);
    store.keep_scratch(memory).expect("scratch");
}

fn run(store: &mut Store, now: Timestamp) -> aeon_distil::Consolidated {
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
fn a_second_pass_reinforces_rather_than_duplicating() {
    // A pass that ran nightly and added a copy of everything each time would fill a project
    // with restatements and make confidence meaningless.
    let mut store = Store::ephemeral().expect("store");
    said(&mut store, "s1", "the database is postgres", MARCH);
    said(&mut store, "s2", "the database is postgres", MARCH + 86_400);

    run(&mut store, AUGUST);
    let after_first = store.all().expect("export").len();
    let second = run(&mut store, AUGUST + 3600);

    assert_eq!(second.promoted.len(), 0);
    assert_eq!(second.reinforced, 1);
    assert_eq!(store.all().expect("export").len(), after_first);
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
