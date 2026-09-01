//! What a harness installs, and what happens to it when aeon is not there.
//!
//! The degradation story is the one that matters. A memory layer that can brick the agent is
//! worse than no memory layer, so every call has to answer `nil` and let the harness carry on
//! exactly as it did before.

use aeon_lua::{CLIENT, Engine};
use std::path::PathBuf;

/// The drop-in a harness loads, as it ships.
fn shipped() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/harness/aeon.lua");
    std::fs::read_to_string(&path).expect("aeon ships a harness drop-in")
}

/// Run a script with the drop-in loaded, and take what it left.
fn through(script: &str) -> String {
    let mut engine = Engine::new();
    let source = format!(
        r#"
        local harness = assert(load({harness:?}, "harness.lua"))()
        aeon.answer = tostring({script})
        "#,
        harness = shipped()
    );
    engine
        .run(&source, "probe.lua")
        .expect("the drop-in must load");
    engine.harvest();
    engine
        .config()
        .string("answer")
        .expect("an answer")
        .to_owned()
}

#[test]
fn the_drop_in_loads() {
    assert_eq!(through("harness._NAME"), "aeon.harness");
}

#[test]
fn nothing_is_live_before_anything_connects() {
    assert_eq!(through("harness.live()"), "false");
}

#[test]
fn observing_with_no_aeon_is_not_an_error() {
    // Fire and forget. What is lost is one turn's observation, and the harness still has its
    // own transcript to backfill from later.
    assert_eq!(through("harness.observe('s', {})"), "false");
}

#[test]
fn planning_with_no_aeon_answers_nothing() {
    // The whole degradation story: not an error to handle, an absence to carry on through.
    // A harness that gets nil does whatever it did before aeon existed.
    assert_eq!(through("harness.plan('s', {}) == nil"), "true");
}

#[test]
fn every_call_survives_aeon_being_absent() {
    for call in [
        "harness.plan('s', {})",
        "harness.recall('anything')",
        "harness.remember('a thing')",
    ] {
        assert_eq!(through(&format!("{call} == nil")), "true", "{call} raised");
    }
}

#[test]
fn opening_without_the_client_says_what_is_missing() {
    // The usual cause is that `aeon configs` has not been run, and saying so is what makes
    // that fixable rather than mysterious.
    let why = through("select(2, harness.open({}))");
    assert!(why.contains("client library"), "{why}");
}

#[test]
fn a_failed_connection_is_not_retried_every_turn() {
    // A harness asks several times per turn. Paying for a connect attempt on each of them,
    // for a daemon that is not running, would put a syscall storm in the turn loop.
    let answer = through(&format!(
        "(function() \
               harness.open({{ source = {CLIENT:?}, where = {{ path = '/no/such/socket' }} }}) \
               local _, why = harness.open({{ source = {CLIENT:?} }}) \
               return why \
             end)()"
    ));
    assert!(answer.contains("earlier this session"), "{answer}");
}

#[test]
fn closing_when_nothing_is_open_is_harmless() {
    assert_eq!(
        through("(function() harness.close() return harness.live() end)()"),
        "false"
    );
}
