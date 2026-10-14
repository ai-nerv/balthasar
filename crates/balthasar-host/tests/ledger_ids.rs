//! Each search the ledger records is its own record, named after what was actually asked.

use balthasar_host::{Answering, Door, answer};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{ScopeId, floor};
use balthasar_store::Store;

const NOW: balthasar_model::Timestamp = 1_756_000_000;

fn recall(store: &mut Store, query: &str) -> Reply {
    let mut at = Answering {
        store,
        scrollback: None,
        scratch: None,
        scope: ScopeId::new("/w/thing"),
        agent: balthasar_model::AgentId::main(),
        now: NOW,
        inject_floor: floor::INJECT,
        live_floor: floor::LIVE,
        capture: true,
    };
    answer(
        &mut at,
        &Door::Owner,
        &Request {
            call: "recall".to_owned(),
            args: vec![serde_json::json!(query)],
        },
    )
}

fn recall_of(store: &Store, reply: &Reply) -> balthasar_store::Trace {
    let injection = reply.result[0]["injection"].as_str().expect("an injection");
    let recall = injection.replacen("inject-", "recall-", 1);
    store.trace_of(&recall).expect("read").expect("recorded")
}

#[test]
fn two_searches_in_one_second_are_two_records_each_with_its_own_query() {
    let mut store = Store::ephemeral().expect("store");
    let first = recall(&mut store, "deploy target");
    let second = recall(&mut store, "test command");
    let again = recall(&mut store, "deploy target");
    for reply in [&first, &second, &again] {
        assert!(reply.ok, "{:?}", reply.error);
    }
    let (a, b, c) = (
        recall_of(&store, &first),
        recall_of(&store, &second),
        recall_of(&store, &again),
    );
    assert_ne!(
        a.query_hash, b.query_hash,
        "the words asked are what is hashed"
    );
    assert_eq!(a.query_hash, c.query_hash);
    assert_ne!(
        a.recall, c.recall,
        "the same words twice are still two searches"
    );
}
