//! The shipped examples, run.
//!
//! pi ships roughly seventy-eight example extensions. That is not documentation — it is how they
//! know the extension surface works, and balthasar's had never been used by anybody who did not
//! write it. An example that does not load is worse than no example, because somebody copies it.
//!
//! These read each file over the shipped configuration, exactly as a plugin directory would, and
//! check that it declared what it says it declares.

use balthasar_lua::Engine;
use std::path::PathBuf;

/// The `config/` directory of this checkout.
fn shipped() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("config")
}

/// The `examples/plugin/` directory of this checkout.
fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/plugin")
        .join(name)
}

/// The shipped configuration with one example layered over it, as the runtimepath would.
fn layered(name: &str) -> Engine {
    let mut engine = Engine::new();
    engine
        .read(&[(shipped().join("init.lua"), true), (example(name), true)])
        .unwrap_or_else(|why| panic!("{name}: {why}"));
    engine
}

#[test]
fn a_source_example_adds_one_without_taking_the_shipped_one_with_it() {
    // The independence commitment, made usable: a new source is a file somebody drops in a
    // directory rather than a release. This one is deliberately not a harness, because a source
    // is a shape and not a category.
    // The shipped ids are read rather than written down here: naming a harness in a Rust file is
    // what `gate-independent` exists to stop, and this test is about layering rather than about
    // which harnesses ship.
    let mut plain = Engine::new();
    plain
        .read(&[(shipped().join("init.lua"), true)])
        .expect("the shipped configuration loads");
    let before = plain.config();
    let shipped_ids: Vec<&str> = before.all("source").into_iter().map(|(id, _)| id).collect();
    assert!(!shipped_ids.is_empty(), "there is a shipped source to keep");

    let engine = layered("shell-history.lua");
    let config = engine.config();
    let named: Vec<&str> = config.all("source").into_iter().map(|(id, _)| id).collect();

    assert!(named.contains(&"zsh-history"), "{named:?}");
    for kept in &shipped_ids {
        assert!(
            named.contains(kept),
            "and the shipped sources are untouched: {named:?}"
        );
    }
}

#[test]
fn a_section_example_takes_its_place_in_the_declared_order() {
    // Sections are keyed and ordered, so where an added one lands is the whole question. This
    // asks for `order = 15` — after identity, before how-this-project-works — and that is a
    // claim about the shipped numbers as much as about the example.
    let engine = layered("build-commands.lua");
    let config = engine.config();
    let named: Vec<&str> = config
        .all("section")
        .into_iter()
        .map(|(id, _)| id)
        .collect();

    assert!(named.contains(&"commands-that-work"), "{named:?}");
    for shipped in ["identity", "how-this-project-works", "recent", "relevant"] {
        assert!(
            named.contains(&shipped),
            "the shipped sections are untouched: {named:?}"
        );
    }

    let added = config
        .one("section", "commands-that-work")
        .expect("declared");
    assert_eq!(
        added.get("order").and_then(serde_json::Value::as_f64),
        Some(15.0),
        "between identity (10) and how-this-project-works (20)"
    );
    assert!(
        added
            .get("min_confidence")
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|floor| floor >= 0.8),
        "a wrong build command is worse than no build command"
    );
}
