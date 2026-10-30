//! F6: decisions in the person's own words, and what each replaces when the turn says so.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_distil::{Ingest, Observation, Role, decisions, distil_run};
use balthasar_lua::{Engine, Settings};
use balthasar_model::{
    Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
};
use balthasar_store::{Recall, Store, Transcript, Turn, mint};

const NOW: Timestamp = 1_756_000_000;

fn scope() -> ScopeId {
    ScopeId::new("/w/thing")
}

fn user(text: &str) -> Observation {
    Observation {
        role: Role::User,
        text: text.to_owned(),
        ..Observation::default()
    }
}

fn held(store: &mut Store, text: &str) {
    let memory = Memory::new(
        mint(NOW - 86_400),
        Tier::Fact,
        scope(),
        Body::note(text, NoteKind::Claim),
        NOW - 86_400,
    );
    let witness = Witness::new(
        WitnessId::new(format!("w-{}", text.len())),
        WitnessKind::Imperative,
        SessionId::new("earlier"),
        scope(),
        NOW - 86_400,
    );
    store.remember(memory, witness, NOW - 86_400).expect("held");
}

/// Choices and changes a person makes, each of which must be caught.
const DECIDED: &[&str] = &[
    "let's go with zstd for the transport",
    "we'll use postgres for the queue",
    "we decided to vendor the sdk",
    "we're going with pnpm from here",
    "we switched to ruff for linting",
    "use make instead of cargo test",
    "we no longer ship the arm32 build",
];

/// The same words used otherwise: none of these is a decision.
const NOT_DECIDED: &[&str] = &[
    "the migrations folder is generated",
    "a replacement part arrived today",
    "we did not switch to zstd in the end",
    "should we go with postgres or mysql?",
    "I never decided to vendor anything",
    "the build is run with make",
    "the changes are in the second commit",
    "he said we picked the wrong one once",
    "nobody is using the old dashboard",
    "the switch statement is too long",
];

#[test]
fn a_choice_is_caught_in_the_persons_words() {
    let store = Store::ephemeral().expect("store");
    let missed: Vec<&str> = DECIDED
        .iter()
        .copied()
        .filter(|said| decisions(&store, &scope(), &[user(said)], NOW, &[]).is_empty())
        .collect();
    let recall = 1.0 - missed.len() as f64 / DECIDED.len() as f64;
    assert!(recall >= 0.8, "missed: {missed:?}");
}

#[test]
fn a_noun_a_question_or_a_denial_is_not_a_decision() {
    let store = Store::ephemeral().expect("store");
    let taken: Vec<&str> = NOT_DECIDED
        .iter()
        .copied()
        .filter(|said| !decisions(&store, &scope(), &[user(said)], NOW, &[]).is_empty())
        .collect();
    let precision = 1.0 - taken.len() as f64 / NOT_DECIDED.len() as f64;
    assert!(precision >= 0.9, "taken as decisions: {taken:?}");
}

#[test]
fn a_quantity_change_replaces_the_quantity_held() {
    let mut store = Store::ephemeral().expect("store");
    held(&mut store, "the job makes 3 retries");
    let found = decisions(&store, &scope(), &[user("bump the retries to 7")], NOW, &[]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].replaces.is_some(), "{found:?}");
    let asked = decisions(
        &store,
        &scope(),
        &[user("the job makes 7 retries now")],
        NOW,
        &[],
    );
    assert!(
        asked
            .iter()
            .any(|c| c.replaces.is_some() || c.disputes.is_some()),
        "{asked:?}"
    );
}

#[test]
fn a_bare_reply_linked_by_the_answer_replaces() {
    let mut store = Store::ephemeral().expect("store");
    held(&mut store, "we compress the transport with gzip");
    let turns = [
        user("zstd"),
        Observation {
            role: Role::Assistant,
            text: "Noted: zstd replaces the earlier gzip for the transport.".to_owned(),
            ..Observation::default()
        },
    ];
    let found = decisions(&store, &scope(), &turns, NOW, &[]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].text(), "we compress the transport with zstd");
    assert!(found[0].replaces.is_some());
}

#[test]
fn distilled_end_to_end_the_old_value_stops_being_offered() {
    let mut store = Store::ephemeral().expect("store");
    held(&mut store, "we compress the transport with gzip");
    let mut scrollback = Transcript::ephemeral().expect("scrollback");
    let session = SessionId::new("01DECIDE");
    scrollback
        .open_run(&session, "/w/thing", "/w/thing", "test", NOW)
        .expect("run");
    for (cursor, role, text) in [
        (1, "user", "zstd"),
        (
            2,
            "assistant",
            "Switching from gzip to zstd for the transport.",
        ),
    ] {
        let turn = Turn {
            cursor,
            at: NOW,
            role: role.to_owned(),
            kind: role.to_owned(),
            text: text.to_owned(),
            ..Turn::default()
        };
        scrollback.write(&session, &turn).expect("recorded");
    }
    let ask = Ingest {
        source: "transcript".into(),
        scope: scope(),
        since: None,
        dry_run: false,
        now: NOW,
    };
    distil_run(
        &mut store,
        &mut Engine::new(),
        &Settings::default(),
        &scrollback,
        &session,
        &ask,
    )
    .expect("distilled");
    let found: Vec<String> = store
        .recall(&Recall::of("transport", NOW))
        .expect("recall")
        .into_iter()
        .map(|hit| hit.memory.text())
        .collect();
    assert!(found.iter().any(|t| t.contains("zstd")), "{found:?}");
    assert!(!found.iter().any(|t| t.contains("gzip")), "{found:?}");
}
