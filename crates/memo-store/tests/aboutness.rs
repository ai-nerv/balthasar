//! What a memory is *about*, and how many answers a predicate may hold.
//!
//! Two mechanisms taken from the reference implementations in `xtra/`: mem0's rarity-weighted
//! entity boost, and R-Mem's split between predicates that name one current answer and
//! predicates that name an accumulating set.

use memo_model::{
    Body, Memory, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
};
use memo_store::{Landing, Recall, Store, mint};

const NOW: Timestamp = 1_756_000_000;
const SCOPE: &str = "/w/thing";

fn store() -> Store {
    Store::ephemeral().expect("a store")
}

fn scope() -> ScopeId {
    ScopeId::new(SCOPE)
}

fn saw(name: &str) -> Witness {
    Witness::new(
        WitnessId::new(name),
        WitnessKind::Imperative,
        SessionId::new("s1"),
        scope(),
        NOW,
    )
}

fn keep(store: &mut Store, body: Body, witness: &str) -> Landing {
    let memory = Memory::new(mint(NOW), Tier::Fact, scope(), body, NOW);
    store.remember(memory, saw(witness), NOW).expect("remember")
}

fn ask(query: &str) -> Recall {
    let mut recall = Recall::of(query, NOW);
    recall.scope_name = SCOPE.to_owned();
    recall.near = true;
    recall.floor = 0.0;
    recall
}

// ── entities ────────────────────────────────────────────────────────────────

#[test]
fn a_query_about_a_thing_finds_it_without_sharing_a_word() {
    // The case full-text search cannot reach: "fly.io" and "deployment" have no token in
    // common, and both are about the same thing.
    let mut store = store();
    keep(
        &mut store,
        Body::note("we ship to fly.io now", memo_model::NoteKind::Claim),
        "w1",
    );

    let lexical = store.recall(&Recall::of("fly.io", NOW)).expect("recall");
    assert_eq!(lexical.len(), 1, "the words alone find it");

    let found = store.recall(&ask("what about fly.io")).expect("recall");
    assert_eq!(found.len(), 1);
    assert!(found[0].entity > 0.0, "and so does what it is about");
}

#[test]
fn the_entity_signal_can_add_a_candidate_the_words_missed() {
    // The deliberate departure from mem0, where a boost may only reorder what the first stage
    // found. A gate with no way out of it decides in advance what can never be found.
    let mut store = store();
    let landing = keep(
        &mut store,
        Body::note(
            "`make test` is what works here",
            memo_model::NoteKind::Claim,
        ),
        "w1",
    );
    let id = landing.id().clone();

    // No shared token: "here" is a stopword, and nothing else overlaps.
    let bare = store
        .recall(&Recall::of("`make test`", NOW))
        .expect("recall");
    let with_entities = store.recall(&ask("`make test`")).expect("recall");

    assert!(
        with_entities.iter().any(|hit| hit.memory.id == id),
        "the entity index found it"
    );
    assert!(with_entities.len() >= bare.len());
}

#[test]
fn a_rare_thing_counts_for_more_than_a_common_one() {
    // mem0's idea, end to end: one of two "Postgres" memories is likelier to be the one you
    // want than one of fifty.
    let mut rare = store();
    keep(
        &mut rare,
        Body::note("the staging box runs Redis", memo_model::NoteKind::Claim),
        "w1",
    );

    let mut common = store();
    for n in 0..40 {
        keep(
            &mut common,
            Body::note(
                format!("note {n} about Redis and other things"),
                memo_model::NoteKind::Claim,
            ),
            &format!("w{n}"),
        );
    }

    let one = rare.recall(&ask("Redis")).expect("recall");
    let many = common.recall(&ask("Redis")).expect("recall");
    assert!(
        one[0].entity > many[0].entity,
        "rare {} should beat common {}",
        one[0].entity,
        many[0].entity
    );
}

#[test]
fn a_memory_can_say_what_it_is_about() {
    let mut store = store();
    let landing = keep(
        &mut store,
        Body::note(
            "the bug is in src/lib.rs, run `make test`",
            memo_model::NoteKind::Claim,
        ),
        "w1",
    );
    let about: Vec<String> = store
        .entities_of(landing.id())
        .expect("entities")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(about.contains(&"src/lib.rs".to_owned()), "{about:?}");
    assert!(about.contains(&"make test".to_owned()), "{about:?}");
}

#[test]
fn one_projects_entities_do_not_reach_another() {
    // Durable memory is the project's, and so is knowing what it is about.
    let mut store = store();
    keep(
        &mut store,
        Body::note("we use Redis", memo_model::NoteKind::Claim),
        "w1",
    );

    let found = store.by_entity("/w/other", "Redis", 10).expect("entities");
    assert!(found.is_empty(), "another project's index is empty");
}

#[test]
fn a_query_about_nothing_costs_nothing() {
    let store = store();
    assert!(
        store
            .by_entity(SCOPE, "it does the thing", 10)
            .expect("entities")
            .is_empty()
    );
}

// ── cardinality ─────────────────────────────────────────────────────────────

#[test]
fn a_predicate_naming_one_answer_is_superseded() {
    let mut store = store();
    keep(
        &mut store,
        Body::fact("project", "deploy_target", "heroku"),
        "w1",
    );
    let landing = keep(
        &mut store,
        Body::fact("project", "deploy_target", "fly.io"),
        "w2",
    );

    assert!(
        matches!(landing, Landing::Superseded { .. }),
        "a new deploy target replaces the old one: {landing:?}"
    );
    let live = store
        .live_slot(SCOPE, "project", "deploy_target")
        .expect("slot")
        .expect("there");
    assert_eq!(live.body.object(), Some("fly.io"));
}

#[test]
fn a_predicate_naming_a_set_accumulates() {
    // R-Mem's distinction, and the defect it exposed: the unique index applied to every fact,
    // so memo could not record that somebody liked two things.
    let mut store = store();
    let first = keep(&mut store, Body::fact("you", "likes", "sushi"), "w1");
    let second = keep(&mut store, Body::fact("you", "likes", "pizza"), "w2");

    assert!(matches!(first, Landing::Added(_)), "{first:?}");
    assert!(
        matches!(second, Landing::Added(_)),
        "liking a second thing is not a correction of the first: {second:?}"
    );

    let everything = store.all().expect("export");
    let held: Vec<Option<&str>> = everything.iter().map(|m| m.body.object()).collect();
    assert!(
        held.contains(&Some("sushi")) && held.contains(&Some("pizza")),
        "{held:?}"
    );
}

#[test]
fn saying_the_same_thing_twice_still_reinforces() {
    // A set accumulates distinct answers. It does not accumulate the same answer twice.
    let mut store = store();
    keep(&mut store, Body::fact("you", "likes", "sushi"), "w1");
    let again = keep(&mut store, Body::fact("you", "likes", "sushi"), "w2");
    assert!(matches!(again, Landing::Reinforced(_)), "{again:?}");
}

#[test]
fn a_project_can_have_several_central_files() {
    // The extractor produces one of these per file it sees read repeatedly. Under the old
    // index the second silently superseded the first.
    let mut store = store();
    keep(
        &mut store,
        Body::fact("project", "central_file", "src/lib.rs"),
        "w1",
    );
    keep(
        &mut store,
        Body::fact("project", "central_file", "src/store.rs"),
        "w2",
    );
    assert_eq!(store.all().expect("export").len(), 2);
}
