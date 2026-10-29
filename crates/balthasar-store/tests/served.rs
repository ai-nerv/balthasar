//! An archived memory stays out of an ordinary search, whichever path finds it.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::{
    Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
};
use balthasar_store::{Recall, Store, mint};

const AT: Timestamp = 1_756_000_000;
const SCOPE: &str = "/w/thing";

#[test]
fn an_entity_does_not_bring_back_a_forgotten_memory() {
    let mut store = Store::ephemeral().expect("store");
    let memory = Memory::new(
        mint(AT),
        Tier::Fact,
        ScopeId::new(SCOPE),
        Body::note("QWFORGOTTEN lives in crates/net/codec.rs", NoteKind::Claim),
        AT,
    );
    let witness = Witness::new(
        WitnessId::new("w-1"),
        WitnessKind::Imperative,
        SessionId::new("s1"),
        ScopeId::new(SCOPE),
        AT,
    );
    let id = store
        .remember(memory, witness, AT)
        .expect("remembered")
        .id()
        .clone();
    store.archive(&id, AT + 1).expect("forgotten");

    let mut ask = Recall::of("where is crates/net/codec.rs", AT + 2);
    ask.scope_name = SCOPE.to_owned();
    ask.floor = 0.0;
    let found = store.recall(&ask).expect("recall");
    assert!(
        found.iter().all(|hit| hit.memory.id != id),
        "the archive came back through its entity: {found:?}"
    );
}
