//! End-to-end CLI tests for wired decay rates, learned ranking shadow/opt-in,
//! external benchmark execution, and ledger retention.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::scratch::Scratch;
use balthasar_recall::{FEATURES, LAYOUT, Model};
use std::path::Path;
use std::process::Command;

fn run(dir: &Path, config_home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_balthasar"))
        .current_dir(dir)
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("XDG_CACHE_HOME", dir.join("cache"))
        .args(args)
        .output()
        .expect("run balthasar")
}

#[test]
fn decay_command_respects_configured_rates() {
    let dir = Scratch::new("balthasar-wired", "decay");
    let cfg = dir.join("cfg");
    std::fs::create_dir_all(cfg.join("balthasar")).expect("mkdir");
    std::fs::write(
        cfg.join("balthasar/init.lua"),
        "balthasar.decay.normal = 0.8\nbalthasar.decay.inertia = false\n",
    )
    .expect("write cfg");
    let store = dir.join("m.db");
    let store_str = store.to_string_lossy().into_owned();
    {
        let mut db = balthasar_store::Store::open(&store).expect("open");
        let scope = balthasar_model::ScopeId::new(dir.to_string_lossy().as_ref());
        let mem = balthasar_model::Memory::new(
            balthasar_store::mint(1_756_000_000),
            balthasar_model::Tier::Habit,
            scope.clone(),
            balthasar_model::Body::note("we use uv", balthasar_model::NoteKind::Claim),
            1_756_000_000,
        );
        let witness = balthasar_model::Witness::new(
            balthasar_model::WitnessId::new("w-decay-1"),
            balthasar_model::WitnessKind::Imperative,
            balthasar_model::SessionId::new("s1"),
            scope,
            1_756_000_000,
        );
        db.remember(mem, witness, 1_756_000_000).expect("remember");
    }

    let later = (1_756_000_000_i64 + 20 * 86_400).to_string();
    let decayed = run(
        &dir,
        &cfg,
        &[
            "--store", &store_str, "--at", &later, "decay", "--now", "--json",
        ],
    );
    assert!(decayed.status.success(), "{decayed:?}");
    let body: serde_json::Value = serde_json::from_slice(&decayed.stdout).expect("json");
    let swept = body["result"][0]["swept"].as_array().expect("swept");
    assert_eq!(
        swept.len(),
        1,
        "aggressive decay rate should sweep after 20d: {body}"
    );
}

#[test]
fn recall_shadow_learned_and_opt_in_use_fitted_policy() {
    let dir = Scratch::new("balthasar-wired", "learned");
    let cfg = dir.join("cfg");
    let store = dir.join("m.db");
    let store_str = store.to_string_lossy().into_owned();

    for text in [
        "deploy api workers using flyctl",
        "deploy static docs using cloudflare pages",
    ] {
        let out = run(
            &dir,
            &cfg,
            &[
                "--no-config",
                "--store",
                &store_str,
                "--at",
                "1756000000",
                "remember",
                text,
            ],
        );
        assert!(out.status.success(), "{out:?}");
    }

    let mut model = Model {
        weights: vec![0.0; FEATURES.len()],
        bias: 0.0,
        mean: vec![0.0; FEATURES.len()],
        scale: vec![1.0; FEATURES.len()],
        trained_on: 400,
        holdout_auc: 0.82,
        layout: LAYOUT,
    };
    model.weights[0] = 3.0;
    model
        .save(&store.with_extension("policy.json"))
        .expect("save");

    let shadowed = run(
        &dir,
        &cfg,
        &[
            "--no-config",
            "--store",
            &store_str,
            "--at",
            "1756000000",
            "recall",
            "deploy",
            "--shadow",
            "learned",
        ],
    );
    assert!(shadowed.status.success(), "{shadowed:?}");
    let stdout = String::from_utf8_lossy(&shadowed.stdout);
    assert!(stdout.contains("shadow learned"), "{stdout}");

    let learned = run(
        &dir,
        &cfg,
        &[
            "--no-config",
            "--store",
            &store_str,
            "--at",
            "1756000000",
            "recall",
            "deploy",
            "--learned",
            "--json",
        ],
    );
    assert!(learned.status.success(), "{learned:?}");
    let body: serde_json::Value = serde_json::from_slice(&learned.stdout).expect("json");
    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(body["n"], 2, "{body}");
}

#[test]
fn eval_external_executes_loaded_cases() {
    let dir = Scratch::new("balthasar-wired", "external");
    let cfg = dir.join("cfg");
    let bench = dir.join("bench");
    std::fs::create_dir_all(&bench).expect("mkdir");
    std::fs::write(
        bench.join("locomo.jsonl"),
        "{\"turns\":[{\"text\":\"remember: we deploy to fly.io\"}],\"questions\":[{\"asks\":\"where do we deploy\",\"answer\":\"fly.io\"}]}\n",
    )
    .expect("write dataset");

    let bench_str = bench.to_string_lossy().into_owned();
    let out = run(
        &dir,
        &cfg,
        &[
            "--no-config",
            "eval",
            "--sessions",
            "2",
            "--external",
            &bench_str,
        ],
    );
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("1/1") && stdout.contains("locomo"),
        "{stdout}"
    );
}
