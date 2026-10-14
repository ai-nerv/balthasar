//! The key `serve` is told on its standard input when it starts, and which every call on its socket
//! carries: against the real binary, as a coordinator starts it.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

mod serving;

use serde_json::json;
use serving::{KEY, Serving};

#[test]
fn serve_told_no_key_or_a_malformed_one_refuses_to_start() {
    let upper = format!("{}\n", KEY.to_uppercase());
    for fed in ["", "not a key\n", KEY, upper.as_str()] {
        let said = Serving::fed("nokey", fed, &[])
            .err()
            .unwrap_or_else(|| panic!("{fed:?}: it started"));
        assert!(said.contains("64 lowercase hex"), "{fed:?}: {said}");
    }
}

#[test]
fn a_call_without_the_key_or_with_another_is_refused_and_the_right_one_answered() {
    let serving = Serving::start("keys");
    let other = KEY.replace('5', "6");
    for key in [None, Some(other.as_str()), Some("")] {
        let told = serving.ask_with(key, "verbs", json!([]));
        assert_eq!(told["fault"], "refused", "{key:?}: {told}");
        assert_eq!(told["error"], balthasar_ipc::KEY_REFUSED, "{key:?}: {told}");
    }
    let answered = serving.ask("verbs", json!([]));
    assert_eq!(answered["ok"], true, "{answered}");
}

#[test]
fn a_second_peer_of_the_same_user_cannot_write_into_the_session() {
    let serving = Serving::start("second");
    let turn = |cursor: u64, text: &str| json!({ "cursor": cursor, "role": "user", "kind": "prose", "text": text, "raw": {} });
    let ours = serving.ask("observe", json!(["s", turn(1, "what the session said")]));
    assert_eq!(ours["ok"], true, "{ours}");

    for (verb, said) in [
        ("observe", turn(2, "planted")),
        ("amend", turn(1, "rewritten")),
        (
            "observe",
            json!({ "cursor": 3, "role": "trace", "kind": "permission", "text": "x" }),
        ),
    ] {
        let told = serving.ask_with(None, verb, json!(["s", said]));
        assert_eq!(told["error"], balthasar_ipc::KEY_REFUSED, "{verb}: {told}");
    }

    let replayed = serving.ask("replay", json!(["s"]));
    let rows = replayed["result"].as_array().expect("rows");
    assert_eq!(rows.len(), 1, "{replayed}");
    assert_eq!(rows[0]["text"], "what the session said");
    assert_eq!(rows[0]["revisions"], 0, "never amended");
}
