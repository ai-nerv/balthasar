//! M4: what a model is actually told, and what it is not.

use aeon_model::{
    Body, Importance, Memory, Privacy, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId,
    WitnessKind, floor,
};
use aeon_recall::{Ask, Bound, Section, assemble};
use aeon_store::{Store, Weights, mint};

const NOW: Timestamp = 1_756_000_000;

fn fresh() -> Store {
    Store::ephemeral().expect("a store")
}

fn keep(
    store: &mut Store,
    tier: Tier,
    body: Body,
    importance: Importance,
    kind: WitnessKind,
) -> Memory {
    let mut memory = Memory::new(mint(NOW), tier, ScopeId::global(), body, NOW);
    memory.strength.importance = importance;
    let witness = Witness::new(
        WitnessId::new(format!("w{}", memory.content_hash)),
        kind,
        SessionId::new("s1"),
        ScopeId::global(),
        NOW,
    );
    let landing = store.remember(memory, witness, NOW).expect("remember");
    store.get(landing.id()).expect("get").expect("there")
}

fn section(id: &str, spec: serde_json::Value) -> Section {
    Section::read(id, &spec).expect("a section")
}

fn ask(tokens: usize, bound: Bound) -> Ask {
    Ask {
        turn: String::new(),
        tokens,
        bound,
        floor: floor::INJECT,
        weights: Weights::default().without_vectors(),
        now: NOW,
        scope: "global".to_owned(),
    }
}

fn plain(text: &str, _memory: &Memory) -> Option<String> {
    Some(text.to_owned())
}

#[test]
fn what_is_asserted_is_injected() {
    let mut store = fresh();
    keep(
        &mut store,
        Tier::Fact,
        Body::fact("project", "test_command", "make test"),
        Importance::High,
        WitnessKind::Imperative,
    );

    let context = assemble(
        &[(store, true)],
        &[section("facts", serde_json::json!({ "tiers": ["fact"] }))],
        &ask(1000, Bound::Local),
        plain,
    )
    .expect("assemble");

    assert_eq!(context.sections.len(), 1);
    assert!(context.text().contains("make test"), "{}", context.text());
}

#[test]
fn what_is_merely_findable_is_not() {
    // The two floors, at the boundary that matters. One distillation is worth keeping and is
    // not worth telling a model.
    let mut store = fresh();
    keep(
        &mut store,
        Tier::Fact,
        Body::fact("project", "guess", "maybe"),
        Importance::Normal,
        WitnessKind::Distillation,
    );

    let context = assemble(
        &[(store, true)],
        &[section("facts", serde_json::json!({ "tiers": ["fact"] }))],
        &ask(1000, Bound::Local),
        plain,
    )
    .expect("assemble");
    assert!(context.is_empty(), "{}", context.text());
}

#[test]
fn a_section_may_ask_for_more_certainty_than_the_store() {
    // A build command that is wrong is worse than no build command.
    let mut store = fresh();
    keep(
        &mut store,
        Tier::Fact,
        Body::fact("project", "test_command", "make test"),
        Importance::High,
        WitnessKind::Cost,
    );

    let lenient = assemble(
        &[(store, true)],
        &[section("facts", serde_json::json!({ "tiers": ["fact"] }))],
        &ask(1000, Bound::Local),
        plain,
    )
    .expect("assemble");
    assert!(!lenient.is_empty());

    let mut store = fresh();
    keep(
        &mut store,
        Tier::Fact,
        Body::fact("project", "test_command", "make test"),
        Importance::High,
        WitnessKind::Cost,
    );
    let strict = assemble(
        &[(store, true)],
        &[section(
            "facts",
            serde_json::json!({ "tiers": ["fact"], "min_confidence": 0.9 }),
        )],
        &ask(1000, Bound::Local),
        plain,
    )
    .expect("assemble");
    assert!(strict.is_empty(), "a section's own floor holds");
}

#[test]
fn a_local_memory_does_not_leave_the_machine() {
    let mut store = fresh();
    let memory = keep(
        &mut store,
        Tier::Fact,
        Body::fact("you", "name", "Sam"),
        Importance::High,
        WitnessKind::Imperative,
    );
    store
        .db_privacy(&memory.id, Privacy::Local)
        .expect("mark it local");

    let sections = [section("facts", serde_json::json!({ "tiers": ["fact"] }))];
    let here =
        assemble(&[(store, true)], &sections, &ask(1000, Bound::Local), plain).expect("assemble");
    assert!(!here.is_empty(), "a local model may be told");
}

#[test]
fn one_claim_restated_is_said_once() {
    let mut store = fresh();
    for text in [
        "we run the tests with make test",
        "the tests are run with make test",
    ] {
        keep(
            &mut store,
            Tier::Fact,
            Body::note(text, aeon_model::NoteKind::Claim),
            Importance::High,
            WitnessKind::Imperative,
        );
    }

    let context = assemble(
        &[(store, true)],
        &[section("facts", serde_json::json!({ "tiers": ["fact"] }))],
        &ask(1000, Bound::Local),
        plain,
    )
    .expect("assemble");
    assert_eq!(context.sections[0].lines.len(), 1);
    assert_eq!(context.deduplicated, 1);
}

#[test]
fn a_line_a_handler_withholds_is_not_said() {
    let mut store = fresh();
    keep(
        &mut store,
        Tier::Fact,
        Body::note("the token is sk-secret", aeon_model::NoteKind::Claim),
        Importance::High,
        WitnessKind::Imperative,
    );
    keep(
        &mut store,
        Tier::Fact,
        Body::note("we deploy with fly", aeon_model::NoteKind::Claim),
        Importance::High,
        WitnessKind::Imperative,
    );

    let context = assemble(
        &[(store, true)],
        &[section("facts", serde_json::json!({ "tiers": ["fact"] }))],
        &ask(1000, Bound::Remote),
        |text, _| (!text.contains("sk-")).then(|| text.to_owned()),
    )
    .expect("assemble");

    assert_eq!(context.redacted, 1);
    assert!(!context.text().contains("sk-secret"), "{}", context.text());
    assert!(context.text().contains("fly"));
}

#[test]
fn a_handler_may_rewrite_rather_than_withhold() {
    let mut store = fresh();
    keep(
        &mut store,
        Tier::Fact,
        Body::note("the token is sk-secret", aeon_model::NoteKind::Claim),
        Importance::High,
        WitnessKind::Imperative,
    );

    let context = assemble(
        &[(store, true)],
        &[section("facts", serde_json::json!({ "tiers": ["fact"] }))],
        &ask(1000, Bound::Remote),
        |text, _| Some(text.replace("sk-secret", "sk-…")),
    )
    .expect("assemble");
    assert!(context.text().contains("sk-…"), "{}", context.text());
    assert_eq!(context.redacted, 0);
}

#[test]
fn a_budget_is_honoured() {
    let mut store = fresh();
    for n in 0..50 {
        keep(
            &mut store,
            Tier::Fact,
            Body::fact(
                "project",
                format!("thing_{n}"),
                format!("a value number {n}"),
            ),
            Importance::High,
            WitnessKind::Imperative,
        );
    }

    let context = assemble(
        &[(store, true)],
        &[section("facts", serde_json::json!({ "tiers": ["fact"] }))],
        &ask(40, Bound::Local),
        plain,
    )
    .expect("assemble");
    assert!(
        context.tokens <= 44,
        "{} tokens for a 40-token budget",
        context.tokens
    );
    assert!(!context.is_empty(), "and it said something");
}

#[test]
fn what_one_section_does_not_use_goes_to_the_next() {
    // Without this, a thin section wastes the room it was allotted while the one after it is
    // truncated three lines from the end.
    let mut store = fresh();
    keep(
        &mut store,
        Tier::Fact,
        Body::fact("you", "name", "Sam"),
        Importance::High,
        WitnessKind::Imperative,
    );
    for n in 0..20 {
        keep(
            &mut store,
            Tier::Habit,
            Body::habit(
                format!("situation number {n}"),
                vec![format!("do thing {n}")],
            ),
            Importance::High,
            WitnessKind::Imperative,
        );
    }

    let sections = [
        section(
            "identity",
            serde_json::json!({ "weight": 1, "order": 10, "tiers": ["fact"] }),
        ),
        section(
            "habits",
            serde_json::json!({ "weight": 1, "order": 20, "tiers": ["habit"] }),
        ),
    ];
    let context =
        assemble(&[(store, true)], &sections, &ask(200, Bound::Local), plain).expect("assemble");

    let habits = context
        .sections
        .iter()
        .find(|s| s.id == "habits")
        .expect("habits were rendered");
    // An even split would have given habits half of 800 characters. Identity used ~20 of its
    // 400, and the rest went here.
    assert!(habits.lines.len() > 8, "only {} lines", habits.lines.len());
}

#[test]
fn a_chronological_section_stays_chronological() {
    // Sorting episodes by salience puts last week above this morning, which tells a reader
    // nothing about what happened.
    let mut store = fresh();
    for (n, when) in [(1, NOW - 300), (2, NOW - 200), (3, NOW - 100)] {
        let mut memory = Memory::new(
            mint(when),
            Tier::Episode,
            ScopeId::global(),
            Body::episode(
                format!("the {n} thing happened"),
                aeon_model::Span::at(n),
                vec![],
                aeon_model::Outcome::Done,
            ),
            when,
        );
        memory.temporal = aeon_model::Temporal::recalled(NOW, when);
        let witness = Witness::new(
            WitnessId::new(format!("w{n}")),
            WitnessKind::Imperative,
            SessionId::new("s1"),
            ScopeId::global(),
            when,
        );
        store.remember(memory, witness, NOW).expect("remember");
    }

    let context = assemble(
        &[(store, true)],
        &[section(
            "recent",
            serde_json::json!({ "tiers": ["episode"], "preserve_order": true }),
        )],
        &ask(1000, Bound::Local),
        plain,
    )
    .expect("assemble");

    let lines = &context.sections[0].lines;
    assert!(lines[0].contains("the 1 thing"), "{lines:?}");
    assert!(lines[2].contains("the 3 thing"), "{lines:?}");
}

#[test]
fn a_section_with_a_limit_keeps_to_it() {
    let mut store = fresh();
    for n in 0..10 {
        keep(
            &mut store,
            Tier::Fact,
            Body::fact("project", format!("thing_{n}"), "a value"),
            Importance::High,
            WitnessKind::Imperative,
        );
    }
    let context = assemble(
        &[(store, true)],
        &[section(
            "facts",
            serde_json::json!({ "tiers": ["fact"], "limit": 3 }),
        )],
        &ask(1000, Bound::Local),
        plain,
    )
    .expect("assemble");
    assert_eq!(context.sections[0].lines.len(), 3);
}

#[test]
fn nothing_worth_saying_says_nothing() {
    let context = assemble(
        &[(fresh(), true)],
        &[section("facts", serde_json::json!({ "tiers": ["fact"] }))],
        &ask(1000, Bound::Local),
        plain,
    )
    .expect("assemble");
    assert!(context.is_empty());
    assert_eq!(context.text(), "");
}
