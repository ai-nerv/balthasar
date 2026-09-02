//! The configuration memo ships must load, and must mean what it says.
//!
//! A shipped config that does not run is worse than none: it is the first thing anybody edits,
//! and they would be editing something already broken.

use memo_lua::{Engine, Settings};
use std::path::PathBuf;

/// The `config/` directory of this checkout.
fn shipped() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("config")
}

#[test]
fn the_shipped_configuration_loads() {
    let mut engine = Engine::new();
    engine
        .read(&[(shipped().join("init.lua"), true)])
        .expect("the config memo ships must load");
}

#[test]
fn the_shipped_configuration_leaves_every_default_alone() {
    // Everything in `init.lua` is commented out except the gates. Installing it must turn
    // nothing on that was off — it gives you the real file to edit, and nothing else.
    let mut engine = Engine::new();
    engine
        .read(&[(shipped().join("init.lua"), true)])
        .expect("load");
    assert_eq!(Settings::from(&engine.config()), Settings::default());
}

#[test]
fn the_shipped_sections_are_declared_in_reading_order() {
    let mut engine = Engine::new();
    engine
        .read(&[(shipped().join("init.lua"), true)])
        .expect("load");
    let config = engine.config();
    let names: Vec<&str> = config
        .all("section")
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        names,
        ["identity", "how-this-project-works", "recent", "relevant"]
    );
}

#[test]
fn the_shipped_gate_refuses_a_credential() {
    let mut engine = Engine::new();
    engine
        .read(&[(shipped().join("init.lua"), true)])
        .expect("load");
    let answer = engine.ask(
        "promote",
        &[serde_json::json!({ "text": "the token is sk-abcdefghijklmnopqrstuvwxyz" })],
    );
    assert_eq!(
        answer.and_then(|v| v.get("promote").cloned()),
        Some(serde_json::json!(false))
    );
}

#[test]
fn the_shipped_gate_leaves_ordinary_memories_alone() {
    let mut engine = Engine::new();
    engine
        .read(&[(shipped().join("init.lua"), true)])
        .expect("load");
    let answer = engine.ask(
        "promote",
        &[serde_json::json!({ "text": "we run the tests with make test", "tier": "fact" })],
    );
    assert_eq!(answer, None, "nobody claimed it, so the ladder decides");
}

#[test]
fn a_habit_learned_by_failing_is_promoted_high() {
    let mut engine = Engine::new();
    engine
        .read(&[(shipped().join("init.lua"), true)])
        .expect("load");
    let answer = engine
        .ask(
            "promote",
            &[serde_json::json!({ "tier": "habit", "witness": "cost", "text": "make test works" })],
        )
        .expect("the shipped gate claims it");
    assert_eq!(answer.get("importance"), Some(&serde_json::json!("high")));
}
