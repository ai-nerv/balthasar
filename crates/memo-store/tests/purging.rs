//! Explicit forgetting, and whether it survives an attempt to get the content back.
//!
//! §10.8: after a purge, adversarial evaluation must demonstrate that ordinary and fallback
//! retrieval cannot recover what was removed. That is what this file is: not "did the row go"
//! but "is every path back to it closed".

use memo_model::{
    Body, Derivation, Family, Memory, NoteKind, Relation, ScopeId, SessionId, Tier, View, Witness,
    WitnessId, WitnessKind,
};
use memo_store::{Reach, Recall, Store, mint};

const NOW: memo_model::Timestamp = 1_756_000_000;
const SECRET: &str = "the deploy token MEMO_DEPLOY_TOKEN is hunter2-abcdef-nevershare";

fn scope() -> ScopeId {
    ScopeId::new("/w/thing")
}

/// A store holding a secret, fully indexed and related to a neighbour.
fn a_store_with_a_secret() -> (Store, memo_model::MemoryId, memo_model::MemoryId) {
    let mut store = Store::ephemeral().expect("store");
    let mut ids = Vec::new();
    for (n, text) in [SECRET, "the deploy target is fly.io"].iter().enumerate() {
        let held = Memory::new(
            mint(NOW + n as i64),
            Tier::Fact,
            scope(),
            Body::note(*text, NoteKind::Claim),
            NOW,
        );
        let id = held.id.clone();
        store
            .remember(
                held,
                Witness::new(
                    WitnessId::new(format!("w{n}")),
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
    let (secret, neighbour) = (ids[0].clone(), ids[1].clone());

    store
        .index_entities(&secret, &scope().to_string(), SECRET)
        .expect("index");
    store
        .relate(&[Relation {
            from: secret.clone(),
            to: neighbour.clone(),
            view: View::SameEntity,
            weight: 0.9,
            source: Derivation::Rule,
            derivation_version: 1,
            evidence_cursor: None,
            created_at: NOW,
        }])
        .expect("relate");

    (store, secret, neighbour)
}

#[test]
fn a_purge_says_what_it_will_take_with_it() {
    // A purge cannot be undone, so the closure is shown while a person can still say no.
    let (store, secret, _) = a_store_with_a_secret();
    let closure = memo_store::closure_of(&store, &secret).expect("closure");

    assert!(closure.witnesses > 0, "its evidence");
    assert!(closure.relations > 0, "and its derived edges");
    assert!(closure.entities > 0, "and what it was about");
    assert!(closure.rows() >= 3);
}

#[test]
fn purging_an_indexed_memory_does_not_fail() {
    // It used to. `entity` carries a foreign key to `memory`, and purge did not clear it — so
    // "delete the key I pasted" failed on any memory that had ever been entity-indexed, which
    // is every memory anybody had searched for.
    let (mut store, secret, _) = a_store_with_a_secret();
    let gone = memo_store::purge(&mut store, &secret).expect("purge must not fail");
    assert_eq!(gone, 1);
}

#[test]
fn nothing_ordinary_finds_a_purged_secret() {
    let (mut store, secret, _) = a_store_with_a_secret();
    memo_store::purge(&mut store, &secret).expect("purge");

    for query in ["deploy token", "hunter2", "nevershare", "MEMO_DEPLOY_TOKEN"] {
        let mut ask = Recall::of(query, NOW);
        ask.limit = 50;
        ask.floor = 0.0;
        ask.include_archived = true;
        let found = store.recall(&ask).expect("recall");
        assert!(
            !found.iter().any(|h| h.memory.text().contains("hunter2")),
            "'{query}' recovered the secret"
        );
    }
}

#[test]
fn no_traversal_reaches_a_purged_secret() {
    // The path a defence that only cleared the memory row would leave open: the neighbour is
    // still there, and a derived edge still pointed at the hole.
    let (mut store, secret, neighbour) = a_store_with_a_secret();
    memo_store::purge(&mut store, &secret).expect("purge");

    let reached = store
        .traverse(std::slice::from_ref(&neighbour), &[], &Reach::default())
        .expect("walk");
    assert!(
        !reached.iter().any(|(id, _)| *id == secret),
        "a derived edge still pointed at it: {reached:#?}"
    );
    assert!(
        store
            .relations_of(&neighbour, &[Family::Entity], &Reach::default())
            .expect("read")
            .is_empty(),
        "the edge itself survived"
    );
}

#[test]
fn the_entity_index_forgets_what_it_was_about() {
    // Otherwise "what do we know about hunter2" still answers, from an index rather than a
    // memory — which is the same leak wearing a different table.
    let (mut store, secret, _) = a_store_with_a_secret();
    memo_store::purge(&mut store, &secret).expect("purge");

    let counts = store.entity_counts(&scope().to_string()).expect("counts");
    assert!(
        !counts.keys().any(|name| name.contains("hunter2")),
        "the index still names it: {counts:?}"
    );
}

#[test]
fn the_neighbour_survives_untouched() {
    // A purge that took the neighbourhood with it would be a denial of service dressed as
    // privacy.
    let (mut store, secret, neighbour) = a_store_with_a_secret();
    memo_store::purge(&mut store, &secret).expect("purge");

    let held = store.get(&neighbour).expect("get").expect("still there");
    assert_eq!(held.text(), "the deploy target is fly.io");
    assert!(
        !store
            .witnesses_of(&neighbour)
            .expect("witnesses")
            .is_empty()
    );
}

#[test]
fn purging_twice_is_not_an_error() {
    let (mut store, secret, _) = a_store_with_a_secret();
    assert_eq!(memo_store::purge(&mut store, &secret).expect("first"), 1);
    assert_eq!(memo_store::purge(&mut store, &secret).expect("again"), 0);
}

#[test]
fn forgetting_a_run_takes_what_it_owned_and_leaves_what_it_only_saw() {
    // §10.8's trajectory scope. A fact three runs agree on does not belong to any of them, and
    // taking it with one would be a different and much worse operation than the one asked for.
    let mut store = Store::ephemeral().expect("store");
    let doomed = SessionId::new("01DOOMED");
    let other = SessionId::new("01OTHER");

    // Owned by the run: scratch it created.
    let mut mine = Memory::new(
        mint(NOW),
        Tier::Scratch,
        scope(),
        Body::note("what that run was trying", NoteKind::Observation),
        NOW,
    );
    mine.session = Some(doomed.clone());
    let mine_id = mine.id.clone();
    store
        .remember(
            mine,
            Witness::new(
                WitnessId::new("w-mine"),
                WitnessKind::Imperative,
                doomed.clone(),
                scope(),
                NOW,
            ),
            NOW,
        )
        .expect("remember");

    // Shared: a project fact two runs witnessed, one of them the doomed one.
    let shared = Memory::new(
        mint(NOW + 1),
        Tier::Fact,
        scope(),
        Body::note("the deploy target is fly.io", NoteKind::Claim),
        NOW,
    );
    let shared_id = shared.id.clone();
    store
        .remember(
            shared,
            Witness::new(
                WitnessId::new("w-a"),
                WitnessKind::Imperative,
                doomed.clone(),
                scope(),
                NOW,
            ),
            NOW,
        )
        .expect("remember");
    store
        .attach(
            &shared_id,
            Witness::new(
                WitnessId::new("w-b"),
                WitnessKind::Imperative,
                other,
                scope(),
                NOW,
            ),
            NOW,
        )
        .expect("attach");

    memo_store::purge_session(&mut store, &doomed).expect("purge session");

    assert!(
        store.get(&mine_id).expect("get").is_none(),
        "what it owned went"
    );
    let kept = store
        .get(&shared_id)
        .expect("get")
        .expect("what it saw stayed");
    assert_eq!(kept.text(), "the deploy target is fly.io");
    let left = store.witnesses_of(&shared_id).expect("witnesses");
    assert_eq!(left.len(), 1, "and only the doomed run's evidence went");
    assert_eq!(left[0].session, SessionId::new("01OTHER"));
}

#[test]
fn forgetting_a_source_does_not_take_honest_memories_with_it() {
    // §10.8's environment scope. One poisoned page must not become a way to delete everything
    // it happened to agree with — that is the denial-of-service version of this operation.
    let mut store = Store::ephemeral().expect("store");
    let bad = memo_model::Domain::external("https://untrusted.test/guide");

    let only_theirs = Memory::new(
        mint(NOW),
        Tier::Fact,
        scope(),
        Body::note("something only that page said", NoteKind::Claim),
        NOW,
    );
    let theirs_id = only_theirs.id.clone();
    store
        .remember(
            only_theirs,
            Witness::new(
                WitnessId::new("w-ext"),
                WitnessKind::Distillation,
                SessionId::new("01RUN"),
                scope(),
                NOW,
            )
            .through(memo_model::Channel::ExternalContent, Some(bad.clone())),
            NOW,
        )
        .expect("remember");

    let also_ours = Memory::new(
        mint(NOW + 1),
        Tier::Fact,
        scope(),
        Body::note("something we knew anyway", NoteKind::Claim),
        NOW,
    );
    let ours_id = also_ours.id.clone();
    store
        .remember(
            also_ours,
            Witness::new(
                WitnessId::new("w-ext2"),
                WitnessKind::Distillation,
                SessionId::new("01RUN"),
                scope(),
                NOW,
            )
            .through(memo_model::Channel::ExternalContent, Some(bad.clone())),
            NOW,
        )
        .expect("remember");
    store
        .attach(
            &ours_id,
            Witness::new(
                WitnessId::new("w-said"),
                WitnessKind::Imperative,
                SessionId::new("01ME"),
                scope(),
                NOW,
            ),
            NOW,
        )
        .expect("attach");

    let gone = memo_store::purge_domain(&mut store, &bad).expect("purge domain");

    assert_eq!(gone, 1, "only what stood on that source alone");
    assert!(store.get(&theirs_id).expect("get").is_none());
    let kept = store.get(&ours_id).expect("get").expect("still there");
    assert_eq!(kept.text(), "something we knew anyway");
    let left = store.witnesses_of(&ours_id).expect("witnesses");
    assert_eq!(left.len(), 1, "with the tainted evidence removed");
    assert_eq!(left[0].session, SessionId::new("01ME"));
}
