//! Ending a disagreement, which is the person's to end.
//!
//! A sweep makes two disagreeing claims less believed, and that is the right thing to do while
//! nobody knows which is wrong. These cover what happens once somebody does know.

use balthasar_model::{
    Body, LinkRelation, Memory, MemoryId, NoteKind, ScopeId, SessionId, Tier, Witness, WitnessId,
    WitnessKind,
};
use balthasar_store::{Store, mint};

const NOW: balthasar_model::Timestamp = 1_756_000_000;

fn scope() -> ScopeId {
    ScopeId::new("/w/thing")
}

/// A claim a person stated once.
fn claim(store: &mut Store, text: &str) -> MemoryId {
    let held = Memory::new(
        mint(NOW),
        Tier::Fact,
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
    id
}

/// Two claims, linked as disagreeing the way a sweep leaves them.
fn disagreeing(store: &mut Store) -> (MemoryId, MemoryId) {
    let a = claim(store, "The tests run on CI.");
    let b = claim(store, "The tests need a GPU that no developer has.");
    for (src, dst) in [(&a, &b), (&b, &a)] {
        store
            .link(src, dst, LinkRelation::Contradicts, NOW)
            .expect("link");
    }
    for id in [&a, &b] {
        store.rescore(id, NOW).expect("rescore");
    }
    (a, b)
}

fn confidence(store: &Store, id: &MemoryId) -> f64 {
    store.get(id).expect("read").expect("memory").confidence
}

#[test]
fn an_open_disagreement_is_listed_once_and_says_both_sides() {
    let mut store = Store::ephemeral().expect("store");
    let (a, b) = disagreeing(&mut store);
    let open = store.disagreements("/w/thing").expect("open");
    assert_eq!(open.len(), 1, "one pair, not one row per direction");
    let (left, right) = &open[0];
    let ids = [left.id.clone(), right.id.clone()];
    assert!(ids.contains(&a) && ids.contains(&b));
}

#[test]
fn keeping_one_lifts_the_pressure_off_it_without_rewriting_an_edge() {
    let mut store = Store::ephemeral().expect("store");
    let (a, b) = disagreeing(&mut store);
    let under_dispute = confidence(&store, &a);

    store.settle(&a, &b, NOW).expect("settle");

    assert!(
        confidence(&store, &a) > under_dispute,
        "the one kept should recover what the disagreement cost it: {} then {}",
        under_dispute,
        confidence(&store, &a)
    );
    // The edge is still on record — what changed is that the other claim stopped being current.
    let held = store.get(&a).expect("read").expect("memory");
    assert!(
        held.links
            .iter()
            .any(|link| link.rel == LinkRelation::Contradicts && link.to == b),
        "the disagreement is still part of the record"
    );
    assert!(
        held.links
            .iter()
            .any(|link| link.rel == LinkRelation::Supersedes && link.to == b),
        "and what replaced it is said"
    );
    let dropped = store.get(&b).expect("read").expect("memory");
    assert!(dropped.temporal.valid_to.is_some(), "no longer current");
    assert!(
        dropped.archived_at.is_none(),
        "retired is not deleted; it stays findable"
    );
    assert!(store.disagreements("/w/thing").expect("open").is_empty());
}

#[test]
fn reconciling_overrides_the_edge_and_leaves_both_standing() {
    let mut store = Store::ephemeral().expect("store");
    let (a, b) = disagreeing(&mut store);
    let under_dispute = confidence(&store, &a);

    store.reconcile(&a, &b, NOW).expect("reconcile");

    for id in [&a, &b] {
        let held = store.get(id).expect("read").expect("memory");
        assert!(held.temporal.valid_to.is_none(), "{id} is still current");
        // Nothing is deleted here: that a model read them as disagreeing stays on record, and
        // the person's word overrides it rather than erasing it.
        assert!(
            held.links
                .iter()
                .any(|link| link.rel == LinkRelation::Contradicts),
            "{id} lost the edge instead of having it overridden"
        );
        assert!(
            held.links
                .iter()
                .any(|link| link.rel == LinkRelation::Reconciled),
            "{id} does not record that a person settled it"
        );
    }
    assert!(
        confidence(&store, &a) > under_dispute,
        "both recover: nothing is pulling against them any more"
    );
    assert!(store.disagreements("/w/thing").expect("open").is_empty());
}

#[test]
fn a_superseded_claim_was_never_an_open_disagreement() {
    // `supersede` writes a contradicts edge of its own, old -> new. It has to stay out of this
    // listing, or every ordinary correction would come back as something to settle.
    let mut store = Store::ephemeral().expect("store");
    let first = claim(&mut store, "The port is 8080.");
    let replacement = Memory::new(
        mint(NOW + 1),
        Tier::Fact,
        scope(),
        Body::note("The port is 9090.", NoteKind::Claim),
        NOW + 1,
    );
    store
        .remember(
            replacement,
            Witness::new(
                WitnessId::new("w-later"),
                WitnessKind::Correction,
                SessionId::new("01RUN"),
                scope(),
                NOW + 1,
            ),
            NOW + 1,
        )
        .expect("remember");
    assert!(
        store.get(&first).expect("read").is_some(),
        "still on record"
    );
    assert!(
        store.disagreements("/w/thing").expect("open").is_empty(),
        "a correction is not a disagreement waiting on anybody"
    );
}

#[test]
fn another_projects_claims_are_not_this_projects_to_settle() {
    let mut store = Store::ephemeral().expect("store");
    disagreeing(&mut store);
    assert_eq!(store.disagreements("/w/thing").expect("open").len(), 1);
    assert!(store.disagreements("/w/other").expect("open").is_empty());
}

#[test]
fn settling_the_same_pair_twice_is_agreeing_with_yourself() {
    let mut store = Store::ephemeral().expect("store");
    let (a, b) = disagreeing(&mut store);
    store.reconcile(&a, &b, NOW).expect("reconcile");
    let once = confidence(&store, &a);
    store.reconcile(&a, &b, NOW).expect("again");
    assert!((confidence(&store, &a) - once).abs() < 1e-9);
    assert!(store.disagreements("/w/thing").expect("open").is_empty());
}
