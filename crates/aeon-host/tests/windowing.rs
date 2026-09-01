//! M6's acceptance test: a session that would have overflowed reaches turn 90 instead.

use aeon_host::{Answering, Door, answer, answer_with};
use aeon_ipc::{Reply, Request};
use aeon_model::{ScopeId, floor};
use aeon_store::Store;

const NOW: aeon_model::Timestamp = 1_756_000_000;
const SESSION: &str = "01HZ";

struct Harness {
    store: Store,
}

impl Harness {
    fn new() -> Self {
        Self {
            store: Store::ephemeral().expect("store"),
        }
    }

    fn ask(&mut self, request: &Request) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: None,
            scratch: None,
            scope: ScopeId::new("/w/thing"),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        };
        answer(&mut at, &Door::Owner, request)
    }

    /// A plan, with a mask handler the way a configuration would supply one.
    fn plan(&mut self, window: serde_json::Value) -> serde_json::Value {
        let request = Request {
            call: "plan".into(),
            args: vec![serde_json::json!(SESSION), window],
        };
        let mut at = Answering {
            store: &mut self.store,
            scrollback: None,
            scratch: None,
            scope: ScopeId::new("/w/thing"),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        };
        let reply = answer_with(&mut at, &Door::Owner, &request, |entry| {
            entry
                .tool
                .as_deref()
                .map(|tool| format!("`{tool}` — ok, output elided"))
        });
        assert!(reply.ok, "{:?}", reply.error);
        reply
            .result
            .and_then(|r| r.into_iter().next())
            .expect("a plan")
    }

    /// One turn, as a harness streams it.
    fn observe(&mut self, cursor: u64, role: &str, tokens: u64, text: &str) {
        let mut turn = serde_json::json!({
            "cursor": cursor,
            "role": role,
            "tokens": tokens,
            "text": text,
        });
        if role == "tool" {
            turn["tool"] = serde_json::json!("shell");
            turn["kind"] = serde_json::json!("tool_result");
        }
        let reply = self.ask(&Request {
            call: "observe".into(),
            args: vec![serde_json::json!(SESSION), turn],
        });
        assert!(reply.ok, "{:?}", reply.error);
    }
}

/// A window small enough to overflow quickly, in the shape a harness sends.
fn window() -> serde_json::Value {
    serde_json::json!({
        "window": 20_000,
        "reserve": 4_000,
        "inject": 1_000,
        "mask_over": 500,
        "keep": 8,
    })
}

/// A turn of a coding session: a prompt, a reply, and a wall of tool output.
fn turn(harness: &mut Harness, n: u64) {
    let base = n * 3;
    harness.observe(base, "user", 40, "carry on");
    harness.observe(base + 1, "assistant", 120, "I will run the tests");
    harness.observe(base + 2, "tool", 900, &"test output ".repeat(300));
}

#[test]
fn a_session_that_would_have_overflowed_keeps_going() {
    // The milestone, end to end. At ~1,060 tokens a turn, a 15,000-token budget is gone by
    // turn 15 without a plan. With one, the session reaches 90 and is still under.
    let mut harness = Harness::new();
    let mut last = serde_json::Value::Null;

    for n in 0..90 {
        turn(&mut harness, n);
        last = harness.plan(window());
    }

    let after = last["budget"]["after"].as_u64().expect("a budget");
    let target = last["budget"]["window"].as_u64().expect("a target");
    assert!(
        last["fits"].as_bool().expect("a verdict"),
        "turn 90 did not fit: {}",
        last["why"]
    );
    assert!(after <= target, "{after} over {target}");
}

#[test]
fn the_plan_says_in_one_line_what_it_did() {
    let mut harness = Harness::new();
    for n in 0..30 {
        turn(&mut harness, n);
    }
    let plan = harness.plan(window());
    let why = plan["why"].as_str().expect("a line");
    assert!(why.contains("masked"), "{why}");
    assert!(why.contains("tokens"), "{why}");
}

#[test]
fn masking_carries_the_session_before_any_summary_is_needed() {
    // The ordering, proven on real traffic. Tool output is most of a coding session's window,
    // so masking alone should hold for a long time — and every summary avoided is a request
    // not made and information not lost.
    let mut harness = Harness::new();
    for n in 0..20 {
        turn(&mut harness, n);
    }
    let plan = harness.plan(window());
    assert!(!plan["mask"].as_array().expect("a list").is_empty());
    assert!(
        plan["summarise"].is_null(),
        "summarised too early: {}",
        plan["why"]
    );
}

#[test]
fn the_recent_turns_are_never_masked() {
    let mut harness = Harness::new();
    for n in 0..30 {
        turn(&mut harness, n);
    }
    let plan = harness.plan(window());
    let masked: Vec<u64> = plan["mask"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|m| m["cursor"].as_u64())
        .collect();
    let newest = 29 * 3 + 2;
    assert!(
        !masked.iter().any(|c| *c > newest - 8),
        "the detail still in play was masked: {masked:?}"
    );
}

#[test]
fn a_plan_is_not_re_applied_to_what_it_already_did() {
    // A harness that applies a plan and asks again must not be told to mask what it has
    // already masked — it would pay for the same saving twice and never converge.
    let mut harness = Harness::new();
    for n in 0..30 {
        turn(&mut harness, n);
    }
    let first = harness.plan(window());
    let second = harness.plan(window());
    let count = |plan: &serde_json::Value| plan["mask"].as_array().expect("a list").len();
    assert!(count(&first) > 0);
    assert!(
        count(&second) < count(&first),
        "the same turns were masked twice: {} then {}",
        count(&first),
        count(&second)
    );
}

#[test]
fn a_window_with_room_is_told_to_send_what_it_has() {
    // The common answer, and the one a harness gets on most turns.
    let mut harness = Harness::new();
    turn(&mut harness, 0);
    let plan = harness.plan(window());
    assert!(plan["mask"].as_array().expect("a list").is_empty());
    assert!(plan["summarise"].is_null());
    assert_eq!(plan["keep"].as_array().expect("a list").len(), 3);
    assert!(
        plan["why"]
            .as_str()
            .expect("a line")
            .contains("nothing to do")
    );
}

#[test]
fn a_turn_nobody_can_describe_is_left_alone() {
    // Only a tool's author knows what a useful stub says. With no handler there is nothing
    // honest to put in its place, and an uninformative one is worse than the output.
    let mut harness = Harness::new();
    for n in 0..30 {
        turn(&mut harness, n);
    }
    let reply = harness.ask(&Request {
        call: "plan".into(),
        args: vec![serde_json::json!(SESSION), window()],
    });
    let plan = reply
        .result
        .and_then(|r| r.into_iter().next())
        .expect("a plan");
    assert!(plan["mask"].as_array().expect("a list").is_empty());
}

#[test]
fn asking_about_a_session_nobody_observed_says_so() {
    let mut harness = Harness::new();
    let reply = harness.ask(&Request {
        call: "plan".into(),
        args: vec![serde_json::json!("never-seen"), window()],
    });
    assert!(!reply.ok);
    assert!(
        reply.error.expect("a reason").contains("stream turns"),
        "the fix should be in the message"
    );
}

#[test]
fn observing_the_same_turn_twice_does_not_grow_the_window() {
    // A harness re-sending a turn is correcting what it said about it, not adding a second
    // copy of it to the window.
    let mut harness = Harness::new();
    turn(&mut harness, 0);
    let first = harness.plan(window())["budget"]["was"]
        .as_u64()
        .expect("a cost");
    turn(&mut harness, 0);
    let again = harness.plan(window())["budget"]["was"]
        .as_u64()
        .expect("a cost");
    assert_eq!(first, again);
}

#[test]
fn what_a_session_said_is_its_own_and_not_the_projects() {
    // Observing is not remembering. A turn goes into scratch, which dies with the session
    // unless something on the ladder carries it across — and observing is not that something.
    let mut harness = Harness::new();
    turn(&mut harness, 0);

    let scratch = harness
        .store
        .census()
        .expect("census")
        .into_iter()
        .find(|(tier, _)| tier == "scratch")
        .map(|(_, n)| n)
        .unwrap_or(0);
    assert!(scratch > 0, "the turns were kept");

    let facts = harness
        .store
        .census()
        .expect("census")
        .into_iter()
        .find(|(tier, _)| tier == "fact")
        .map(|(_, n)| n)
        .unwrap_or(0);
    assert_eq!(facts, 0, "and none of them became the project's");
}
