use super::*;
use balthasar_model::ScopeId;
use balthasar_store::Store;

const NOW: balthasar_model::Timestamp = 1_756_000_000;

fn store() -> (Store, SessionId) {
    let mut store = Store::ephemeral().expect("store");
    let id = SessionId::new("01RUN");
    store
        .open_session(&id, &ScopeId::new("/w/thing"), "/w/thing", "a-harness", NOW)
        .expect("open");
    (store, id)
}

fn title_of(store: &Store, id: &SessionId) -> Option<String> {
    store.session_by_id(id).expect("read").and_then(|s| s.title)
}

#[test]
fn the_first_thing_asked_is_the_title_until_something_better_comes() {
    let (mut store, id) = store();
    store.title_session(&id, "hi").expect("title");
    assert_eq!(title_of(&store, &id).as_deref(), Some("hi"));

    assert!(
        store
            .retitle_session(&id, "rename sessions from the model")
            .expect("retitle")
    );
    assert_eq!(
        title_of(&store, &id).as_deref(),
        Some("rename sessions from the model"),
        "four runs opened with `hi` are four runs called `hi`"
    );
}

#[test]
fn a_name_a_person_typed_is_the_end_of_it() {
    let (mut store, id) = store();
    store.rename_session(&id, "the UI work").expect("rename");
    assert!(store.is_named(&id).expect("named"));

    assert!(
        !store
            .retitle_session(&id, "something a model preferred")
            .expect("retitle"),
        "the model was allowed to overwrite a name somebody typed"
    );
    assert_eq!(title_of(&store, &id).as_deref(), Some("the UI work"));
}

#[test]
fn renaming_again_is_the_persons_to_do() {
    let (mut store, id) = store();
    store.rename_session(&id, "first").expect("rename");
    store.rename_session(&id, "second").expect("rename again");
    assert_eq!(title_of(&store, &id).as_deref(), Some("second"));
}

#[test]
fn an_empty_title_is_not_a_title() {
    let (mut store, id) = store();
    store.title_session(&id, "hi").expect("title");
    assert!(!store.retitle_session(&id, "   ").expect("retitle"));
    assert_eq!(title_of(&store, &id).as_deref(), Some("hi"));
}

#[test]
fn a_run_nobody_has_named_is_not_named() {
    let (store, id) = store();
    assert!(!store.is_named(&id).expect("named"));
}
