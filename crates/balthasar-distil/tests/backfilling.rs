//! M3: reading transcripts that already exist, through an adapter written in Lua.
//!
//! The shipped adapters are found by walking `config/sources/`, never by name. That keeps this
//! test honest about the claim it is testing — that a harness is a *file* — and it keeps
//! `gate-independent` able to stay strict, since no Rust file here names one.

use balthasar_model::scratch::{Scratch, ScratchFile};

use balthasar_distil::{Ingest, ingest};
use balthasar_lua::{Engine, Settings};
use balthasar_model::{ScopeId, Tier, Timestamp, WitnessKind};
use balthasar_store::{Recall, Store};
use std::path::{Path, PathBuf};

const MARCH: Timestamp = 1_710_000_000;
const AUGUST: Timestamp = 1_756_000_000;

fn config_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("config")
}

/// Every source adapter balthasar ships, loaded, with the ids they registered.
fn shipped() -> (Engine, Vec<String>) {
    let mut engine = Engine::new();
    let files: Vec<(PathBuf, bool)> =
        balthasar_lua::glob_paths(&format!("{}/sources/*.lua", config_dir().display()))
            .into_iter()
            .map(|p| (PathBuf::from(p), true))
            .collect();
    assert!(
        !files.is_empty(),
        "balthasar ships at least one source adapter"
    );
    engine
        .read(&files)
        .expect("every shipped adapter must load");

    let ids: Vec<String> = engine
        .config()
        .all("source")
        .into_iter()
        .map(|(id, _)| id.to_owned())
        .collect();
    (engine, ids)
}

/// A journal in the shape the shipped adapter reads, written to a real file.
fn journal(name: &str, lines: &[String]) -> ScratchFile {
    let path = Scratch::file("balthasar-ingest", name, "session.jsonl");
    std::fs::write(&path, lines.join("\n")).expect("write");
    path
}

fn meta(session: &str, opened: Timestamp) -> String {
    serde_json::json!({
        "record": "meta", "version": 0, "session": session,
        "cwd": "/home/you/work/thing", "started": opened
    })
    .to_string()
}

fn user(cursor: u64, text: &str) -> String {
    serde_json::json!({
        "record": "entry", "cursor": cursor,
        "entry": { "type": "user", "id": "m", "text": text }
    })
    .to_string()
}

fn call(cursor: u64, tool: &str, command: &str, ok: bool) -> String {
    serde_json::json!({
        "record": "entry", "cursor": cursor,
        "entry": {
            "type": "tool", "id": "t", "name": tool,
            "args": serde_json::json!({ "command": command }).to_string(),
            "result": { "output": "…", "is_error": !ok }
        }
    })
    .to_string()
}

/// Re-registering replaces the whole spec, so the override has to carry the rest forward.
fn ingest_one(
    source: &str,
    file: &Path,
    store: &mut Store,
    dry_run: bool,
) -> balthasar_distil::Report {
    let (mut engine, _) = shipped();
    // Keep the adapter's own `meta` and `line`, and give it one file to walk.
    engine
        .run(
            &format!(
                "local held = __balthasar_specs.source[{source:?}]\n\
                 held.sessions = function() return {{ {:?} }} end",
                file.to_string_lossy()
            ),
            "override.lua",
        )
        .expect("override");

    ingest(
        &mut engine,
        store,
        &Settings::default(),
        &Ingest {
            source: source.to_owned(),
            scope: ScopeId::global(),
            since: None,
            dry_run,
            now: AUGUST,
        },
    )
    .expect("ingest")
}

#[test]
fn every_shipped_adapter_loads_and_registers_itself() {
    let (_, ids) = shipped();
    assert!(
        !ids.is_empty(),
        "an adapter that registers nothing is inert"
    );
}

#[test]
fn every_shipped_adapter_offers_the_three_functions_ingest_calls() {
    // A missing one is not a compile error and not a runtime error either — it is an ingest
    // that reads nothing and reports success.
    let (mut engine, ids) = shipped();
    for id in &ids {
        for method in ["sessions", "meta", "line"] {
            assert!(
                engine.offers("source", id, method),
                "the '{id}' adapter declares no {method}()"
            );
        }
    }
}

#[test]
fn a_session_is_read_and_what_it_taught_is_kept() {
    // The canonical case, end to end and through Lua: a command that failed, then one that
    // worked, becomes a habit the next session starts with.
    let (_, ids) = shipped();
    let source = ids.first().expect("an adapter").clone();
    let file = journal(
        "taught",
        &[
            meta("s1", MARCH),
            user(1, "run the tests"),
            call(2, "shell", "cargo test", false),
            call(3, "shell", "make test", true),
        ],
    );

    let mut store = Store::ephemeral().expect("store");
    let report = ingest_one(&source, &file, &mut store, false);

    assert_eq!(report.sessions, 1);
    assert_eq!(
        report.observations, 3,
        "every entry became a turn; the meta line is not one"
    );
    assert!(report.promoted >= 1, "{report:?}");

    let found = store
        .recall(&Recall {
            query: "make test".into(),
            limit: 10,
            tiers: vec![Tier::Habit],
            floor: 0.0,
            include_archived: false,
            remote: false,
            relevance: 0.0,
            now: AUGUST,
            scope_name: "/w/thing".to_owned(),
            weights: balthasar_store::Weights::default().without_vectors(),
            embedding: None,
            near: true,
            reinforce: false,
        })
        .expect("recall");
    assert_eq!(found.len(), 1, "the habit is there");
    assert!(found[0].memory.text().contains("make test"));
}

#[test]
fn what_was_learned_is_dated_from_when_it_happened() {
    // Backfilling six months of transcripts this afternoon must not claim all of it became
    // true this afternoon.
    let (_, ids) = shipped();
    let source = ids.first().expect("an adapter").clone();
    let file = journal(
        "dated",
        &[
            meta("s1", MARCH),
            call(1, "shell", "cargo test", false),
            call(2, "shell", "make test", true),
        ],
    );

    let mut store = Store::ephemeral().expect("store");
    ingest_one(&source, &file, &mut store, false);

    let memory = store
        .all()
        .expect("export")
        .into_iter()
        .next()
        .expect("something was kept");
    assert_eq!(memory.temporal.valid_from, MARCH);
    assert_eq!(memory.temporal.observed_at, AUGUST);
    assert_eq!(memory.witnesses[0].kind, WitnessKind::Cost);
}

#[test]
fn reading_the_same_journal_twice_adds_nothing() {
    // The stamp's whole purpose. An ingest that is not safe to re-run is one nobody runs.
    let (_, ids) = shipped();
    let source = ids.first().expect("an adapter").clone();
    let file = journal(
        "twice",
        &[
            meta("s1", MARCH),
            call(1, "shell", "cargo test", false),
            call(2, "shell", "make test", true),
        ],
    );

    let mut store = Store::ephemeral().expect("store");
    let first = ingest_one(&source, &file, &mut store, false);
    let before = store.all().expect("export").len();

    let second = ingest_one(&source, &file, &mut store, false);
    assert_eq!(second.already_read, 1, "the stamp was recognised");
    assert_eq!(second.observations, 0, "nothing was re-read");
    assert_eq!(store.all().expect("export").len(), before);
    assert!(first.promoted > 0);
}

#[test]
fn a_dry_run_writes_nothing() {
    let (_, ids) = shipped();
    let source = ids.first().expect("an adapter").clone();
    let file = journal(
        "dry",
        &[
            meta("s1", MARCH),
            call(1, "shell", "cargo test", false),
            call(2, "shell", "make test", true),
        ],
    );

    let mut store = Store::ephemeral().expect("store");
    let report = ingest_one(&source, &file, &mut store, true);
    assert!(report.dry_run);
    assert!(report.promoted > 0, "it still says what it would do");
    assert!(
        store.all().expect("export").is_empty(),
        "and does none of it"
    );
}

#[test]
fn an_instruction_a_person_typed_is_kept() {
    let (_, ids) = shipped();
    let source = ids.first().expect("an adapter").clone();
    let file = journal(
        "said",
        &[meta("s1", MARCH), user(1, "remember: we deploy with fly")],
    );

    let mut store = Store::ephemeral().expect("store");
    ingest_one(&source, &file, &mut store, false);

    let found = store.all().expect("export");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].text(), "we deploy with fly");
    assert_eq!(found[0].witnesses[0].kind, WitnessKind::Imperative);
}

#[test]
fn a_line_the_adapter_does_not_recognise_costs_that_line_only() {
    // A journal with a torn tail, or a record from a newer build. One bad line must not end
    // the ingest.
    let (_, ids) = shipped();
    let source = ids.first().expect("an adapter").clone();
    let file = journal(
        "torn",
        &[
            meta("s1", MARCH),
            "{ this line is not json".to_owned(),
            serde_json::json!({ "record": "entry", "cursor": 2, "entry": { "type": "future" } })
                .to_string(),
            user(3, "remember: we deploy with fly"),
        ],
    );

    let mut store = Store::ephemeral().expect("store");
    let report = ingest_one(&source, &file, &mut store, false);
    assert_eq!(report.observations, 1, "the good line still arrived");
    assert_eq!(store.all().expect("export").len(), 1);
}

#[test]
fn a_file_the_adapter_will_not_describe_is_skipped() {
    // `sessions()` globs a directory and will find things that are not transcripts.
    let (_, ids) = shipped();
    let source = ids.first().expect("an adapter").clone();
    let file = journal("stranger", &["# a markdown file, somehow".to_owned()]);

    let mut store = Store::ephemeral().expect("store");
    let report = ingest_one(&source, &file, &mut store, false);
    assert_eq!(report.sessions, 0);
    assert!(store.all().expect("export").is_empty());
}

#[test]
fn a_turn_the_model_errored_on_is_not_remembered() {
    // Remembering an errored turn teaches the next session to produce more of them.
    let (_, ids) = shipped();
    let source = ids.first().expect("an adapter").clone();
    let file = journal(
        "errored",
        &[
            meta("s1", MARCH),
            serde_json::json!({
                "record": "entry", "cursor": 1,
                "entry": { "type": "assistant", "id": "m", "text": "half a thought",
                           "thinking": "", "stop_reason": "error", "error": "boom" }
            })
            .to_string(),
        ],
    );

    let mut store = Store::ephemeral().expect("store");
    let report = ingest_one(&source, &file, &mut store, false);
    assert_eq!(report.observations, 0);
}
