//! M2: the config API, as a configuration author would meet it.

use balthasar_lua::{Engine, LuaError, Settings};

fn run(source: &str) -> Engine {
    let mut engine = Engine::new();
    engine
        .run(source, "test.lua")
        .expect("the config must load");
    engine.harvest();
    engine
}

#[test]
fn a_setting_is_assigned_and_read_back() {
    let engine = run("balthasar.inject_floor = 0.6");
    assert_eq!(engine.config().number("inject_floor"), Some(0.6));
}

#[test]
fn a_config_can_read_and_reassign_its_own_settings() {
    // Settings are plain fields harvested at the end rather than intercepted as they are
    // written, so only the value a config finished with is the one it meant.
    let engine = run(
        "balthasar.inject_floor = 0.6\nif balthasar.inject_floor > 0.5 then balthasar.inject_floor = 0.4 end",
    );
    assert_eq!(engine.config().number("inject_floor"), Some(0.4));
}

#[test]
fn a_nested_table_exists_before_a_config_touches_it() {
    // `balthasar.decay.normal = 0.02` must work without a config writing `balthasar.decay = {}` first.
    let engine = run("balthasar.decay.normal = 0.02");
    assert_eq!(engine.config().nested("decay", "normal"), Some(0.02));
}

#[test]
fn a_config_is_a_program() {
    // The whole reason a configuration format was not enough: it can loop and branch.
    let engine = run(r#"
        for _, name in ipairs({ "one", "two", "three" }) do
          balthasar.section(name, { weight = 1 })
        end
        "#);
    assert_eq!(engine.config().all("section").len(), 3);
}

#[test]
fn registering_twice_replaces_rather_than_appends() {
    // The map form. A config that loops over a directory must be safe to re-run.
    let engine = run(r#"
        balthasar.section("facts", { weight = 1 })
        balthasar.section("facts", { weight = 9 })
        "#);
    let config = engine.config();
    let sections = config.all("section");
    assert_eq!(sections.len(), 1);
    assert_eq!(
        sections[0].1.get("weight").and_then(|w| w.as_i64()),
        Some(9)
    );
}

#[test]
fn the_file_returns_nothing() {
    let mut engine = Engine::new();
    engine
        .run("balthasar.inject_floor = 0.5", "test.lua")
        .expect("a config that returns nothing is a config");
}

#[test]
fn an_asked_handler_answers_or_passes() {
    // The `on.` contract: nil means not mine, a table means do this instead, first non-nil
    // wins.
    let mut engine = Engine::new();
    engine
        .run(
            r#"
            balthasar.on.scope(function(cwd)
              if cwd:find("scratch") then return { id = "global" } end
            end)
            balthasar.on.scope(function(cwd)
              return { id = "fallback" }
            end)
            "#,
            "test.lua",
        )
        .expect("load");

    let scratch = engine.ask("scope", &[serde_json::json!("/home/you/scratch/x")]);
    assert_eq!(
        scratch.and_then(|v| v.get("id").cloned()),
        Some(serde_json::json!("global"))
    );

    let other = engine.ask("scope", &[serde_json::json!("/home/you/work/x")]);
    assert_eq!(
        other.and_then(|v| v.get("id").cloned()),
        Some(serde_json::json!("fallback"))
    );
}

#[test]
fn a_question_nobody_claimed_answers_nothing() {
    let mut engine = Engine::new();
    assert_eq!(engine.ask("scope", &[serde_json::json!("/anywhere")]), None);
}

#[test]
fn one_raising_handler_does_not_cost_the_others_their_say() {
    let mut engine = Engine::new();
    engine
        .run(
            r#"
            balthasar.on.promote(function(c) error("this one is broken") end)
            balthasar.on.promote(function(c) return { promote = true } end)
            "#,
            "test.lua",
        )
        .expect("load");
    let answer = engine.ask("promote", &[serde_json::json!({ "tier": "fact" })]);
    assert_eq!(
        answer.and_then(|v| v.get("promote").cloned()),
        Some(serde_json::json!(true))
    );
}

#[test]
fn a_told_handler_is_pure_side_effect() {
    // The `did.` contract: every one runs, and what it returns is ignored.
    let mut engine = Engine::new();
    engine
        .run(
            r#"
            balthasar.did.promote(function(m) balthasar.log("first: " .. m.text) end)
            balthasar.did.promote(function(m) error("broken") end)
            balthasar.did.promote(function(m) balthasar.log("third: " .. m.text) end)
            "#,
            "test.lua",
        )
        .expect("load");
    engine.tell("promote", &[serde_json::json!({ "text": "a thing" })]);

    let log = engine.config().log;
    assert_eq!(log, ["first: a thing", "third: a thing"]);
}

#[test]
fn a_handler_registered_against_an_unknown_name_is_refused_at_load() {
    // A gate that never fires is usually one registered against a name balthasar does not ask.
    // Better to say so while the config is loading than to be silently inert forever.
    let mut engine = Engine::new();
    let outcome = engine.run("balthasar.on.nonsense(function() end)", "test.lua");
    assert!(
        matches!(outcome, Err(LuaError::Runtime { .. })),
        "{outcome:?}"
    );
}

#[test]
fn registering_something_that_is_not_a_function_is_refused() {
    let mut engine = Engine::new();
    let outcome = engine.run("balthasar.on.scope(\"not a function\")", "test.lua");
    assert!(
        matches!(outcome, Err(LuaError::Runtime { .. })),
        "{outcome:?}"
    );
}

#[test]
fn a_config_that_will_not_parse_names_the_file() {
    let mut engine = Engine::new();
    let outcome = engine.run("this is not lua ===", "mine.lua");
    match outcome {
        Err(LuaError::Syntax { file, .. }) => assert_eq!(file, "mine.lua"),
        other => panic!("expected a syntax error naming the file, got {other:?}"),
    }
}

#[test]
fn a_registrar_needs_a_name_first() {
    let mut engine = Engine::new();
    let outcome = engine.run("balthasar.section({ weight = 1 })", "test.lua");
    assert!(
        matches!(outcome, Err(LuaError::Runtime { .. })),
        "{outcome:?}"
    );
}

#[test]
fn the_helpers_a_scope_handler_needs_are_there() {
    let mut engine = Engine::new();
    engine
        .run(
            r#"
            balthasar.on.scope(function(cwd)
              return { id = balthasar.path.basename(cwd), home = balthasar.home ~= nil }
            end)
            "#,
            "test.lua",
        )
        .expect("load");
    let answer = engine
        .ask("scope", &[serde_json::json!("/home/you/work/thing")])
        .expect("an answer");
    assert_eq!(answer.get("id"), Some(&serde_json::json!("thing")));
    assert_eq!(answer.get("home"), Some(&serde_json::json!(true)));
}

#[test]
fn a_gate_can_refuse_a_credential() {
    // The shipped gate from the plan, running for real.
    let mut engine = Engine::new();
    engine
        .run(
            r#"
            balthasar.on.promote(function(c)
              if balthasar.looks_like_secret(c.text) then
                return { promote = false, reason = "looks like a credential" }
              end
            end)
            "#,
            "test.lua",
        )
        .expect("load");

    let key = engine.ask(
        "promote",
        &[serde_json::json!({ "text": "token is sk-abcdefghijklmnopqrstuv" })],
    );
    assert_eq!(
        key.and_then(|v| v.get("promote").cloned()),
        Some(serde_json::json!(false))
    );

    let prose = engine.ask("promote", &[serde_json::json!({ "text": "we use make" })]);
    assert_eq!(prose, None, "ordinary prose is nobody's business");
}

#[test]
fn a_source_adapter_gets_a_json_parser() {
    // Commitment 1 rests on this: a harness is a Lua file, and that file must not have to
    // carry its own parser.
    let mut engine = Engine::new();
    engine
        .run(
            r#"
            balthasar.on.admit(function(raw)
              local r = balthasar.json.decode(raw)
              if not r then return { keep = false } end
              return { keep = true, kind = r.type }
            end)
            "#,
            "test.lua",
        )
        .expect("load");

    let good = engine
        .ask("admit", &[serde_json::json!(r#"{"type":"user"}"#)])
        .expect("an answer");
    assert_eq!(good.get("kind"), Some(&serde_json::json!("user")));

    let bad = engine
        .ask("admit", &[serde_json::json!("{ not json")])
        .expect("an answer");
    assert_eq!(bad.get("keep"), Some(&serde_json::json!(false)));
}

#[test]
fn settings_come_out_in_the_shapes_balthasar_uses() {
    let engine = run(r#"
        balthasar.inject_floor = 0.6
        balthasar.decay.normal = 0.02
        balthasar.witness.distillation = 0.1
        balthasar.budget.candidates = 100
        balthasar.imperatives = { "merk" }
        "#);
    let settings = Settings::from(&engine.config());
    assert_eq!(settings.floors.inject, 0.6);
    assert_eq!(settings.decay.normal, 0.02);
    assert_eq!(
        settings.weight(balthasar_model::WitnessKind::Distillation),
        0.1
    );
    assert_eq!(settings.budget.candidates, 100);
    assert_eq!(settings.imperatives, ["merk"]);
    assert_eq!(
        settings.floors.live,
        balthasar_model::floor::LIVE,
        "what was not said keeps its default"
    );
}

mod trust {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("balthasar-trust-{name}"));
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).expect("mkdir");
        at
    }

    fn write(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write");
        path
    }

    #[test]
    fn a_project_file_may_choose() {
        // Setting a floor is a choice about this project. It is allowed.
        let dir = scratch("choose");
        let file = write(&dir, ".balthasar.lua", "balthasar.inject_floor = 0.6\n");
        let mut engine = Engine::new();
        engine.read(&[(file, false)]).expect("choosing is allowed");
        assert_eq!(engine.config().number("inject_floor"), Some(0.6));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_project_file_may_not_declare_a_source() {
        // A source says how somebody's transcripts are read and what is believed as a result.
        // A file that arrived with `git clone` does not get to decide that.
        let dir = scratch("source");
        let file = write(
            &dir,
            ".balthasar.lua",
            "balthasar.source(\"mine\", { sessions = function() end })\n",
        );
        let mut engine = Engine::new();
        match engine.read(&[(file, false)]) {
            Err(LuaError::Untrusted { what, .. }) => assert!(what.contains("source"), "{what}"),
            other => panic!("a project file must not declare a source: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_project_file_may_not_replace_a_source_the_owner_declared() {
        // The hole beside the one above, and the one nothing caught: the check compared the set
        // of *names*, so adding `source("mine")` was refused while redeclaring an existing
        // `source("shared")` left the names unchanged and was accepted. Registration is keyed on
        // `(registrar, id)` and the last write wins, so the project's version became the one
        // that runs — a replacement is as dangerous as an addition and looks like nothing.
        let dir = scratch("replace");
        let owner = write(
            &dir,
            "machine.lua",
            "balthasar.source(\"shared\", { sessions = function() return {} end })\n",
        );
        let project = write(
            &dir,
            ".balthasar.lua",
            "balthasar.source(\"shared\", { sessions = function() return { \"mine\" } end })\n",
        );
        let mut engine = Engine::new();
        match engine.read(&[(owner, true), (project, false)]) {
            Err(LuaError::Untrusted { what, .. }) => assert!(what.contains("shared"), "{what}"),
            other => panic!("a project file must not replace a source: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_project_file_may_not_name_a_command_to_run() {
        // A distiller can be `{ kind = "command", argv = {...} }`. Letting a cloned file set
        // one is arbitrary code execution on `git clone`.
        let dir = scratch("distiller");
        let file = write(
            &dir,
            ".balthasar.lua",
            "balthasar.distiller = { kind = \"command\", argv = { \"curl\", \"evil\" } }\n",
        );
        let mut engine = Engine::new();
        match engine.read(&[(file, false)]) {
            Err(LuaError::Untrusted { what, .. }) => assert!(what.contains("distiller"), "{what}"),
            other => panic!("a project file must not name a command: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_project_file_may_not_vouch_for_itself() {
        // `balthasar.trusted` is how the owner vouches. A file that could add itself to it would
        // make the whole boundary decorative.
        let dir = scratch("selftrust");
        let file = write(&dir, ".balthasar.lua", "balthasar.trusted = { \"/\" }\n");
        let mut engine = Engine::new();
        assert!(matches!(
            engine.read(&[(file, false)]),
            Err(LuaError::Untrusted { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_owners_own_file_may_declare_anything() {
        let dir = scratch("owner");
        let file = write(
            &dir,
            "init.lua",
            "balthasar.source(\"mine\", { kind = \"jsonl\" })\nbalthasar.distiller = { kind = \"endpoint\" }\n",
        );
        let mut engine = Engine::new();
        engine.read(&[(file, true)]).expect("the owner may declare");
        assert_eq!(engine.config().all("source").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_a_config_loads_runs_beside_it() {
        let dir = scratch("load");
        write(
            &dir,
            "sections.lua",
            "balthasar.section(\"facts\", { weight = 3 })\n",
        );
        let file = write(&dir, "init.lua", "balthasar.load(\"sections.lua\")\n");
        let mut engine = Engine::new();
        engine.read(&[(file, true)]).expect("load");
        assert_eq!(engine.config().all("section").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_diamond_of_loads_terminates() {
        // Two files each asking for a third must not run it twice, and a file asking for
        // itself must not run forever.
        let dir = scratch("diamond");
        write(
            &dir,
            "shared.lua",
            "balthasar.section(\"shared\", { weight = 1 })\n",
        );
        write(&dir, "a.lua", "balthasar.load(\"shared.lua\")\n");
        write(&dir, "b.lua", "balthasar.load(\"shared.lua\")\n");
        let file = write(
            &dir,
            "init.lua",
            "balthasar.load(\"a.lua\")\nbalthasar.load(\"b.lua\")\n",
        );
        let mut engine = Engine::new();
        engine.read(&[(file, true)]).expect("load");
        assert_eq!(engine.config().all("section").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_exists_and_does_not_load_is_fatal() {
        // It expressed an intention that has not been carried out. Applying half of it is
        // worse than refusing.
        let dir = scratch("broken");
        let file = write(&dir, "init.lua", "balthasar.inject_floor = = =\n");
        let mut engine = Engine::new();
        assert!(engine.read(&[(file, true)]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

mod specs {
    use super::*;

    #[test]
    fn a_registered_spec_keeps_its_functions() {
        // A source adapter is mostly callbacks. A registrar that kept only the JSON would
        // silently discard everything the adapter was written to do — and the ingest would
        // report zero observations with no error at all.
        let mut engine = Engine::new();
        engine
            .run(
                r#"
                balthasar.source("harness", {
                  kind = "jsonl",
                  line = function(raw)
                    local r = balthasar.json.decode(raw)
                    if not r then return nil end
                    return { role = r.role, text = r.text }
                  end,
                })
                "#,
                "test.lua",
            )
            .expect("load");

        assert!(engine.offers("source", "harness", "line"));
        let observation = engine
            .call(
                "source",
                "harness",
                "line",
                &[serde_json::json!(r#"{"role":"user","text":"hello"}"#)],
            )
            .expect("the adapter answers");
        assert_eq!(observation.get("role"), Some(&serde_json::json!("user")));
    }

    #[test]
    fn the_describable_part_is_still_recorded() {
        let mut engine = Engine::new();
        engine
            .run(
                "balthasar.source(\"harness\", { kind = \"jsonl\", line = function() end })",
                "test.lua",
            )
            .expect("load");
        engine.harvest();
        let config = engine.config();
        let spec = config.one("source", "harness").expect("declared");
        assert_eq!(spec.get("kind"), Some(&serde_json::json!("jsonl")));
    }

    #[test]
    fn a_line_the_adapter_skips_answers_nothing() {
        let mut engine = Engine::new();
        engine
            .run(
                "balthasar.source(\"harness\", { line = function(raw) return nil end })",
                "test.lua",
            )
            .expect("load");
        assert_eq!(
            engine.call("source", "harness", "line", &[serde_json::json!("x")]),
            None
        );
    }

    #[test]
    fn an_adapter_that_raises_costs_its_line_and_nothing_else() {
        // A source walks somebody else's file. One bad line should cost that line, not the
        // ingest.
        let mut engine = Engine::new();
        engine
            .run(
                "balthasar.source(\"harness\", { line = function(raw) error(\"bad\") end })",
                "test.lua",
            )
            .expect("load");
        assert_eq!(
            engine.call("source", "harness", "line", &[serde_json::json!("x")]),
            None
        );
        assert!(engine.offers("source", "harness", "line"), "still usable");
    }

    #[test]
    fn asking_an_unregistered_source_answers_nothing() {
        let mut engine = Engine::new();
        assert!(!engine.offers("source", "nobody", "line"));
        assert_eq!(engine.call("source", "nobody", "line", &[]), None);
    }
}
