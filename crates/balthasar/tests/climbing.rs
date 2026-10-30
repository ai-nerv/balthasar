//! The ladder `serve` climbs on its own, against the real binary: what one session said is
//! asserted in the next, with no verb run by hand.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

mod serving;

use balthasar_model::scratch::Scratch;
use serde_json::json;
use serving::{KEY, Serving};
use std::time::{Duration, Instant};

/// A coordinator's directory under `at`, saying `lua`.
fn told(at: &Scratch, lua: &str) -> String {
    let dir = at.join("given");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("balthasar.lua"), lua).expect("given");
    dir.to_string_lossy().into_owned()
}

fn rule() -> serde_json::Value {
    json!({"cursor": 1, "role": "user", "kind": "prose", "text": "remember: we deploy with fly.io"})
}

/// The row a search for the deploy rule finds, looked for until `within` has passed.
fn found(serving: &Serving, within: Duration) -> Option<serde_json::Value> {
    let deadline = Instant::now() + within;
    loop {
        let said = serving.ask("recall", json!(["deploy"]));
        let row = said["result"].as_array().and_then(|rows| {
            rows.iter()
                .find(|row| row["text"].as_str().is_some_and(|t| t.contains("fly.io")))
                .cloned()
        });
        if row.is_some() || Instant::now() >= deadline {
            return row;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_rule_said_in_one_session_is_asserted_in_the_next() {
    let at = Scratch::new("bcl", "next");
    let given = told(
        &at,
        "balthasar.ladder = { after_idle_s = 0, daily = false }",
    );
    let serving = Serving::fed("next", &format!("{KEY}\n"), &[("NERV_GIVEN", &given)])
        .unwrap_or_else(|said| panic!("serve: {said}"));
    assert_eq!(serving.ask("observe", json!(["one", rule()]))["ok"], true);

    let row = found(&serving, Duration::from_secs(20)).expect("learned with no verb run by hand");
    assert_eq!(row["asserted"], true, "{row}");
}

#[test]
fn a_session_still_speaking_is_not_read_yet() {
    let serving = Serving::start("still");
    assert_eq!(serving.ask("observe", json!(["one", rule()]))["ok"], true);
    serving.ask("status", json!([]));
    assert!(found(&serving, Duration::from_secs(1)).is_none());
}

#[test]
fn told_no_ladder_serve_reads_nothing_on_its_own() {
    let at = Scratch::new("bcl", "off");
    let given = told(&at, "balthasar.ladder = false");
    let serving = Serving::fed("off", &format!("{KEY}\n"), &[("NERV_GIVEN", &given)])
        .unwrap_or_else(|said| panic!("serve: {said}"));
    assert_eq!(serving.ask("observe", json!(["one", rule()]))["ok"], true);
    serving.ask("status", json!([]));
    assert!(found(&serving, Duration::from_secs(1)).is_none());
}
