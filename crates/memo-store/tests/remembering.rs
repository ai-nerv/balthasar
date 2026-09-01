//! What M0 is for: a fact remembered, contradicted, and re-asked.
//!
//! The milestone's own acceptance test. A fact remembered, corrected and asked again answers
//! with the new one; both witnesses survive; and the old answer is still in the store.

use memo_model::{
    Body, Memory, MemoryId, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
};
use memo_store::{Landing, Recall, Store, mint};

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

/// A claim with no slot, which is what most of what people say looks like.
fn note(text: &str, at: Timestamp) -> Memory {
    Memory::new(
        mint(at),
        Tier::Fact,
        ScopeId::global(),
        Body::note(text, memo_model::NoteKind::Claim),
        at,
    )
}

fn saw(kind: WitnessKind, session: &str, at: Timestamp) -> Witness {
    Witness::new(
        WitnessId::new(format!("w-{session}-{at}-{kind}")),
        kind,
        SessionId::new(session),
        ScopeId::global(),
        at,
    )
}

#[test]
fn a_fact_remembered_can_be_recalled() {
    let mut store = store();
    let landing = store
        .remember(
            fact("project", "test_command", "make test", MARCH),
            saw(WitnessKind::Imperative, "s1", MARCH),
            MARCH,
        )
        .expect("remember");
    assert!(matches!(landing, Landing::Added(_)));

    let found = store
        .recall(&Recall::of("test command", MARCH))
        .expect("recall");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].memory.text(), "project test_command make test");
}

#[test]
fn saying_the_same_thing_twice_reinforces_rather_than_duplicates() {
    // Two sessions agreeing is one fact with two witnesses, not two facts. Anything else and
    // the store fills with restatements and confidence means nothing.
    let mut store = store();
    store
        .remember(
            fact("project", "test_command", "make test", MARCH),
            saw(WitnessKind::Distillation, "s1", MARCH),
            MARCH,
        )
        .expect("first");
    let again = store
        .remember(
            fact("project", "test_command", "make test", AUGUST),
            saw(WitnessKind::Distillation, "s2", AUGUST),
            AUGUST,
        )
        .expect("second");

    let Landing::Reinforced(id) = again else {
        panic!("a restatement is not a new fact: {again:?}");
    };
    let memory = store.get(&id).expect("get").expect("there");
    assert_eq!(memory.witnesses.len(), 2);
    assert_eq!(memory.distinct_sessions(), 2);
}

#[test]
fn evidence_from_two_sessions_beats_evidence_from_one() {
    // Diversity, end to end: a claim two unrelated runs saw outranks one a single run
    // repeated, even though both carry two witnesses.
    let mut store = store();
    for (session, at) in [("s1", MARCH), ("s1", MARCH + 60)] {
        store
            .remember(
                fact("loud", "says", "the same thing", at),
                saw(WitnessKind::Repetition, session, at),
                at,
            )
            .expect("loud");
    }
    for (session, at) in [("s1", MARCH), ("s2", MARCH + 60)] {
        store
            .remember(
                fact("spread", "says", "the same thing", at),
                saw(WitnessKind::Repetition, session, at),
                at,
            )
            .expect("spread");
    }

    let loud = store
        .live_slot("global", "loud", "says")
        .expect("slot")
        .expect("there");
    let spread = store
        .live_slot("global", "spread", "says")
        .expect("slot")
        .expect("there");
    assert!(
        spread.confidence > loud.confidence,
        "spread {} should beat loud {}",
        spread.confidence,
        loud.confidence
    );
}

#[test]
fn a_correction_supersedes_without_deleting() {
    // The milestone's acceptance test, and the reason `valid_to` exists.
    let mut store = store();
    let first = store
        .remember(
            fact("project", "test_command", "cargo test", MARCH),
            saw(WitnessKind::Distillation, "s1", MARCH),
            MARCH,
        )
        .expect("the first answer");
    let Landing::Added(old) = first else {
        panic!("expected a new fact: {first:?}");
    };

    let second = store
        .remember(
            fact("project", "test_command", "make test", AUGUST),
            saw(WitnessKind::Correction, "s2", AUGUST),
            AUGUST,
        )
        .expect("the correction");
    let Landing::Superseded { was, now } = second else {
        panic!("a different answer to a live slot supersedes: {second:?}");
    };
    assert_eq!(was, old);

    // Asked now, the store answers with the new one.
    let live = store
        .live_slot("global", "project", "test_command")
        .expect("slot")
        .expect("there");
    assert_eq!(live.id, now);
    assert_eq!(live.body.object(), Some("make test"));

    // Asked about March, it answers with the old one. This is the question a validity
    // interval exists for, and a delete would have destroyed it.
    let then = store
        .slot_at("global", "project", "test_command", MARCH + 10)
        .expect("slot at")
        .expect("there");
    assert_eq!(then.id, old);
    assert_eq!(then.body.object(), Some("cargo test"));
}

#[test]
fn a_superseded_fact_is_still_in_the_export() {
    let mut store = store();
    store
        .remember(
            fact("project", "deploy", "heroku", MARCH),
            saw(WitnessKind::Imperative, "s1", MARCH),
            MARCH,
        )
        .expect("first");
    store
        .remember(
            fact("project", "deploy", "fly.io", AUGUST),
            saw(WitnessKind::Correction, "s2", AUGUST),
            AUGUST,
        )
        .expect("correction");

    let everything = store.all().expect("export");
    assert_eq!(everything.len(), 2, "nothing is deleted");
    let objects: Vec<Option<&str>> = everything.iter().map(|m| m.body.object()).collect();
    assert!(objects.contains(&Some("heroku")));
    assert!(objects.contains(&Some("fly.io")));
}

#[test]
fn both_witnesses_survive_a_correction() {
    // `memo why` has to be able to show the argument on either side of a change of mind.
    let mut store = store();
    store
        .remember(
            fact("project", "deploy", "heroku", MARCH),
            saw(WitnessKind::Imperative, "s1", MARCH),
            MARCH,
        )
        .expect("first");
    let landing = store
        .remember(
            fact("project", "deploy", "fly.io", AUGUST),
            saw(WitnessKind::Correction, "s2", AUGUST),
            AUGUST,
        )
        .expect("correction");
    let Landing::Superseded { was, now } = landing else {
        panic!("expected a supersession");
    };

    let old = store.get(&was).expect("get").expect("there");
    let new = store.get(&now).expect("get").expect("there");
    assert_eq!(old.witnesses.len(), 1);
    assert_eq!(new.witnesses.len(), 1);
    assert_eq!(old.witnesses[0].kind, WitnessKind::Imperative);
    assert_eq!(new.witnesses[0].kind, WitnessKind::Correction);
}

#[test]
fn a_correction_does_not_start_from_nothing() {
    // Being a correction of something established is itself evidence about the correction.
    // Starting a replacement at zero is why systems oscillate between a stale fact and its fix.
    let mut store = store();
    store
        .remember(
            fact("project", "deploy", "heroku", MARCH),
            saw(WitnessKind::Imperative, "s1", MARCH),
            MARCH,
        )
        .expect("a well-established belief");
    let landing = store
        .remember(
            fact("project", "deploy", "fly.io", AUGUST),
            saw(WitnessKind::Distillation, "s2", AUGUST),
            AUGUST,
        )
        .expect("a weakly-evidenced correction");
    let Landing::Superseded { now, .. } = landing else {
        panic!("expected a supersession");
    };

    let new = store.get(&now).expect("get").expect("there");
    assert!(
        new.confidence > WitnessKind::Distillation.weight() * 0.5,
        "a correction inherits what it corrected, got {}",
        new.confidence
    );
}

#[test]
fn a_superseded_fact_stops_being_asserted_but_stays_findable() {
    // Two floors, and they are different floors. This is the whole answer to staleness.
    let mut store = store();
    store
        .remember(
            fact("project", "deploy", "heroku", MARCH),
            saw(WitnessKind::Imperative, "s1", MARCH),
            MARCH,
        )
        .expect("first");
    store
        .remember(
            fact("project", "deploy", "fly.io", AUGUST),
            saw(WitnessKind::Correction, "s2", AUGUST),
            AUGUST,
        )
        .expect("correction");

    let old = store
        .slot_at("global", "project", "deploy", MARCH + 10)
        .expect("slot at")
        .expect("there");
    assert!(
        !old.is_assertable(0.35, AUGUST, false),
        "not asserted, at {}",
        old.confidence
    );
    assert!(old.confidence > 0.0, "still known to have been believed");
}

#[test]
fn an_archived_memory_leaves_the_live_results() {
    let mut store = store();
    let landing = store
        .remember(
            fact("project", "note", "something passing", MARCH),
            saw(WitnessKind::Distillation, "s1", MARCH),
            MARCH,
        )
        .expect("remember");
    let id = landing.id().clone();
    assert_eq!(
        store
            .recall(&Recall::of("passing", MARCH))
            .expect("before")
            .len(),
        1
    );

    store.archive(&id, AUGUST).expect("archive");
    assert!(
        store
            .recall(&Recall::of("passing", AUGUST))
            .expect("after")
            .is_empty()
    );

    let mut deep = Recall::of("passing", AUGUST);
    deep.include_archived = true;
    assert_eq!(store.recall(&deep).expect("archive search").len(), 1);
}

#[test]
fn what_is_recalled_stops_fading() {
    let mut store = store();
    let landing = store
        .remember(
            fact("project", "build", "make build", MARCH),
            saw(WitnessKind::Imperative, "s1", MARCH),
            MARCH,
        )
        .expect("remember");
    let id = landing.id().clone();

    store.touch(&id, AUGUST).expect("touch");
    let memory = store.get(&id).expect("get").expect("there");
    assert_eq!(memory.strength.access_count, 1);
    assert_eq!(memory.strength.last_accessed, AUGUST);
}

#[test]
fn purging_is_the_only_thing_that_removes_anything() {
    // "Delete the API key I pasted" must be answerable with yes.
    let mut store = store();
    let landing = store
        .remember(
            fact("secret", "value", "sk-hunter2", MARCH),
            saw(WitnessKind::Manual, "s1", MARCH),
            MARCH,
        )
        .expect("remember");
    let id = landing.id().clone();

    assert_eq!(memo_store::purge(&mut store, &id).expect("purge"), 1);
    assert!(store.get(&id).expect("get").is_none());
    assert!(store.all().expect("export").is_empty());
}

#[test]
fn a_memory_id_is_not_confused_with_another() {
    let store = store();
    assert!(
        store
            .get(&MemoryId::new("nothing-like-this"))
            .expect("get")
            .is_none()
    );
}

#[test]
fn evidence_for_one_claim_is_not_swallowed_by_another() {
    // Regression. Witness ids were globally unique, so a caller with a per-session naming
    // scheme silently lost evidence the moment two claims shared a session and a moment —
    // and the loss surfaced later as a fact with one fewer witness than it had earned.
    let mut store = store();
    for subject in ["one", "other"] {
        store
            .remember(
                fact(subject, "says", "a thing", MARCH),
                saw(WitnessKind::Repetition, "s1", MARCH),
                MARCH,
            )
            .expect("remember");
    }
    for subject in ["one", "other"] {
        let found = store
            .live_slot("global", subject, "says")
            .expect("slot")
            .expect("there");
        assert_eq!(found.witnesses.len(), 1, "{subject} kept its evidence");
    }
}

#[test]
fn one_claim_restated_lands_on_the_claim_already_held() {
    // The write path's half of learning from paraphrase. Before this, a project told a thing in
    // one run and told it again in other words in the next held two beliefs with one witness
    // each — so neither ever reached the confidence the two runs between them had earned.
    let mut store = store();
    for (session, at) in [("s1", MARCH), ("s2", MARCH + 86_400)] {
        store
            .remember(
                note("we run the tests with make test", at),
                saw(WitnessKind::Imperative, session, at),
                at,
            )
            .expect("first wording");
    }
    let restated = store
        .remember(
            note("the tests are run with make test", MARCH + 172_800),
            saw(WitnessKind::Imperative, "s3", MARCH + 172_800),
            MARCH + 172_800,
        )
        .expect("restated");

    assert!(
        matches!(restated, memo_store::Landing::Reinforced(_)),
        "a restatement reinforces rather than replacing: {restated:?}"
    );
    let all = store.all().expect("all");
    assert_eq!(
        all.len(),
        1,
        "one claim: {:?}",
        all.iter().map(memo_model::Memory::text).collect::<Vec<_>>()
    );
    assert_eq!(
        store.witnesses_of(&all[0].id).expect("witnesses").len(),
        3,
        "and every run that said it is on the record"
    );
}

#[test]
fn one_word_swapped_is_a_different_claim_however_alike_it_reads() {
    // The direction that would do damage. These share three content words out of five, which is
    // enough overlap to read as a rewording — and they are two claims about two subjects.
    let mut store = store();
    for (text, session) in [
        ("the staging box is at 10.0.0.7", "s1"),
        ("the staging box is at 10.0.0.8", "s2"),
        ("the primary box is at 10.0.0.7", "s3"),
    ] {
        store
            .remember(
                note(text, MARCH),
                saw(WitnessKind::Manual, session, MARCH),
                MARCH,
            )
            .expect("keep");
    }
    let held = store.all().expect("all");
    assert!(
        held.len() >= 2,
        "a changed value is not corroboration: {:?}",
        held.iter()
            .map(memo_model::Memory::text)
            .collect::<Vec<_>>()
    );
}
