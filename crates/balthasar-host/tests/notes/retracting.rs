//! Undoing a rule takes back what the same words taught the project, and nothing else.

use super::*;
use balthasar_model::{
    Body, Memory, MemoryId, NoteKind, SessionId, Tier, Witness, WitnessId, WitnessKind,
};
use balthasar_store::{Change, mint};

/// A fact the rules took from the turn at `cursor`.
fn learned(harness: &mut Harness, text: &str, cursor: u64) -> MemoryId {
    let scope = ScopeId::new("/w/thing");
    let memory = Memory::new(
        mint(NOW),
        Tier::Fact,
        scope.clone(),
        Body::note(text, NoteKind::Claim),
        NOW,
    );
    let mut witness = Witness::new(
        WitnessId::new(format!("imperative:{cursor}:{}", text.len())),
        WitnessKind::Imperative,
        SessionId::new(SESSION),
        scope,
        NOW,
    );
    witness.cursor = Some(cursor);
    harness
        .store
        .remember(memory, witness, NOW)
        .expect("learned")
        .id()
        .clone()
}

#[test]
fn undoing_a_rule_closes_what_the_same_words_taught_the_project() {
    let mut harness = Harness::new();
    harness.turn(
        0,
        "Always use tabs for indentation in this project. We deploy with fly.",
    );
    let rule = learned(
        &mut harness,
        "Always use tabs for indentation in this project",
        0,
    );
    let other = learned(&mut harness, "we deploy with fly", 0);
    let after = json!({"title": "Indentation", "text": "Always use tabs for indentation in this project.",
                       "description": "house style", "pinned": true,
                       "evidence": {"sources": [{"session": SESSION, "cursor": 0}]}});
    let note = harness
        .scrollback
        .put_note(None, Some(&after), NOW)
        .expect("note")
        .expect("an id");
    let change = harness
        .scrollback
        .record_change(
            &Change {
                note: Some(note),
                op: "add".into(),
                after: Some(after.clone()),
                state: "applied".into(),
                by: "extract".into(),
                at: NOW,
                evidence: after["evidence"].clone(),
                ..Change::default()
            },
            Some(&SessionId::new(SESSION)),
        )
        .expect("change");

    let undone = harness.one("undo", json!({"change": change}));
    assert_eq!(undone["closed"], 1, "{undone}");
    let closed = harness.store.get(&rule).expect("get").expect("kept");
    assert!(
        closed.temporal.valid_to.is_some(),
        "the rule the person took back"
    );
    let open = harness.store.get(&other).expect("get").expect("kept");
    assert!(
        open.temporal.valid_to.is_none(),
        "another rule said in the same turn stays"
    );
}
