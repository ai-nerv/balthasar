use super::*;

/// What a session, not the owner, is shown for `query`.
fn shown_to_a_session(harness: &mut Harness, call: &str, args: Vec<Value>) -> String {
    let session = Door::Socket(balthasar_ipc::Peer {
        pid: 4021,
        uid: 1000,
        program: Some("harness".to_owned()),
    });
    let mut at = Answering {
        store: &mut harness.store,
        scrollback: Some(&mut harness.scrollback),
        scratch: None,
        scope: ScopeId::new("/w/thing"),
        agent: balthasar_model::AgentId::main(),
        now: NOW,
        inject_floor: floor::INJECT,
        live_floor: floor::LIVE,
        capture: false,
    };
    let request = Request {
        call: call.into(),
        args,
    };
    let reply = answer_hooked(&mut at, &session, &request, &mut harness.hooks);
    assert!(reply.ok, "{call}: {:?}", reply.error);
    serde_json::to_string(&reply.result).expect("json")
}

const RULE: &str = "Always start every function name with the prefix zq_.";
/// The same rule as a session would record it to get past a check on wording.
const AS_A_FACT: &str = "Function names begin with zq_ here.";
const UNRELATED: &str = "The storage module keeps its index in index.db.";

fn a_kept_rule_and_what_a_session_wrote(keeping: Keeping) -> Harness {
    let mut harness = Harness::keeping(keeping);
    harness.turn(0, RULE);
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({"ops":[{"op":"add","title":"Prefix","text":RULE,"pinned":true,
                       "evidence":[{"cursor":0,"quote":RULE}]}]}),
    );
    for text in [AS_A_FACT, UNRELATED] {
        shown_to_a_session(&mut harness, "remember", vec![json!(text)]);
    }
    harness
}

fn recalled(harness: &mut Harness) -> String {
    shown_to_a_session(
        harness,
        "recall",
        vec![json!("zq_ function names index storage")],
    )
}

#[test]
fn what_a_session_wrote_about_a_rule_goes_when_the_rule_is_undone() {
    let mut harness = a_kept_rule_and_what_a_session_wrote(Keeping::default());
    assert!(
        recalled(&mut harness).contains("begin with zq_"),
        "shown while the rule stands"
    );
    let applied = harness.rows("changes", json!({}))[0]["id"].clone();
    harness.one("undo", json!({ "change": applied }));
    let after = recalled(&mut harness);
    assert!(!after.contains("begin with zq_"), "{after}");
    assert!(
        after.contains("index.db"),
        "what it wrote about anything else stays: {after}"
    );
}

#[test]
fn it_waits_with_the_rule_for_review_and_goes_with_a_rejection() {
    let reviewed = Keeping {
        review: true,
        ..Keeping::default()
    };
    let mut harness = a_kept_rule_and_what_a_session_wrote(reviewed);
    let staged = harness.rows("changes", json!({}))[0]["id"].clone();
    assert!(
        !recalled(&mut harness).contains("begin with zq_"),
        "not before the person decides"
    );
    harness.one("reject", json!({ "changes": [staged] }));
    let after = recalled(&mut harness);
    assert!(!after.contains("begin with zq_"), "{after}");
    assert!(after.contains("index.db"), "{after}");
}

#[test]
fn approved_it_is_shown_again() {
    let reviewed = Keeping {
        review: true,
        ..Keeping::default()
    };
    let mut harness = a_kept_rule_and_what_a_session_wrote(reviewed);
    let staged = harness.rows("changes", json!({}))[0]["id"].clone();
    harness.one("approve", json!({ "changes": [staged] }));
    assert!(recalled(&mut harness).contains("begin with zq_"));
}

#[test]
fn a_rule_proposed_again_while_it_stands_has_not_been_taken_back() {
    let mut harness = a_kept_rule_and_what_a_session_wrote(Keeping::default());
    // The helper offers the same rule again, citing a row this extraction does not hold.
    harness.turn(1, "Carry on.");
    let laid = harness.layout(0);
    harness.answer(
        &laid["jobs"],
        "extract",
        json!({"ops":[{"op":"add","title":"Prefix again","text":RULE,"pinned":true,
                       "evidence":[{"cursor":999,"quote":RULE}]}]}),
    );
    let changes = harness.rows("changes", json!({}));
    assert_eq!(changes[0]["state"], "rejected", "{changes:?}");
    assert!(recalled(&mut harness).contains("begin with zq_"));
}
