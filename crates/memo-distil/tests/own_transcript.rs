//! The gap this closes: a run streamed live into memo's own scrollback is read by the rules.
//!
//! Before this, the extractive half of the ladder only ran on `memo ingest`, which walks a
//! *harness's* journal files through a Lua adapter. A session that came in over the socket got
//! TIDE when its turns left the window and CALLUS when another run agreed with it — and nobody
//! was watching it for "remember that we deploy with fly.io", which is the cheapest signal there
//! is and the one a person would most expect to work.

use memo_distil::{Ingest, TRANSCRIPT_SOURCE, distil_run, undistilled};
use memo_lua::{Engine, Settings};
use memo_model::{ScopeId, SessionId, Tier, Timestamp, WitnessKind};
use memo_store::{Recall, Store, Transcript, Turn};

const NOW: Timestamp = 1_756_000_000;

fn scope() -> ScopeId {
    ScopeId::new("/w/thing")
}

fn ask(dry_run: bool) -> Ingest {
    Ingest {
        source: TRANSCRIPT_SOURCE.to_owned(),
        scope: scope(),
        since: None,
        dry_run,
        now: NOW,
    }
}

/// A run, streamed into a scrollback the way `observe` streams one.
fn streamed(held: &mut Transcript, session: &SessionId, turns: Vec<Turn>) {
    held.open_run(session, "/w/thing", "/w/thing", "test", NOW)
        .expect("run");
    for turn in turns {
        held.write(session, &turn).expect("turn");
    }
}

fn said(cursor: u64, text: &str) -> Turn {
    Turn {
        cursor,
        at: NOW,
        role: "user".into(),
        kind: "prose".into(),
        text: text.to_owned(),
        ..Turn::default()
    }
}

fn ran(cursor: u64, command: &str, ok: bool) -> Turn {
    Turn {
        cursor,
        at: NOW,
        role: "tool".into(),
        kind: "tool_result".into(),
        text: if ok { "ok" } else { "failed" }.to_owned(),
        tool: Some("shell".to_owned()),
        ok: Some(ok),
        args: Some(format!(r#"{{"command":"{command}"}}"#)),
        ..Turn::default()
    }
}

#[test]
fn an_instruction_in_a_live_run_reaches_the_project() {
    let mut store = Store::ephemeral().expect("store");
    let mut engine = Engine::new();
    let settings = Settings::default();
    let mut held = Transcript::ephemeral().expect("scrollback");
    let session = SessionId::new("01LIVE");

    streamed(
        &mut held,
        &session,
        vec![
            said(1, "remember: we deploy with fly.io"),
            said(2, "carry on"),
        ],
    );

    let report = distil_run(
        &mut store,
        &mut engine,
        &settings,
        &held,
        &session,
        &ask(false),
    )
    .expect("distil");

    assert_eq!(report.sessions, 1);
    assert!(report.proposed > 0, "the rules found something");
    assert_eq!(report.promoted, 1, "and it crossed: {report:?}");

    let found = store.recall(&Recall::of("deploy", NOW)).expect("recall");
    let kept = found
        .iter()
        .map(|scored| &scored.memory)
        .find(|m| m.text().contains("fly.io"))
        .expect("the project learned it");
    assert_eq!(kept.tier, Tier::Fact);

    // The receipt points back at the turn it came from, in the transcript it was read out of.
    let why = store.witnesses_of(&kept.id).expect("witnesses");
    assert_eq!(why.len(), 1);
    assert_eq!(why[0].kind, WitnessKind::Imperative);
    assert_eq!(why[0].session, session);
    assert_eq!(why[0].cursor, Some(1), "and at which turn");
}

#[test]
fn a_repair_in_a_live_run_is_learned_from_ok_and_args() {
    // SCAR, which is the signal a coding agent produces for free. It is only visible because the
    // transcript now carries `ok` and `args` — without them these are two indistinguishable
    // lines of tool output and there is nothing to learn.
    let mut store = Store::ephemeral().expect("store");
    let mut engine = Engine::new();
    let settings = Settings::default();
    let mut held = Transcript::ephemeral().expect("scrollback");
    let session = SessionId::new("01SCAR");

    streamed(
        &mut held,
        &session,
        vec![
            said(1, "run the tests"),
            ran(2, "cargo test", false),
            ran(3, "make test", true),
        ],
    );

    let report = distil_run(
        &mut store,
        &mut engine,
        &settings,
        &held,
        &session,
        &ask(false),
    )
    .expect("distil");
    assert!(report.promoted > 0, "the repair crossed: {report:?}");

    // Cost, specifically. Any other witness kind would mean something else was learned and the
    // repair was missed — which is what happened before `ok` and `args` were on the turn.
    let kinds: Vec<WitnessKind> = store
        .all()
        .expect("all")
        .iter()
        .flat_map(|m| store.witnesses_of(&m.id).unwrap_or_default())
        .map(|w| w.kind)
        .collect();
    assert!(
        kinds.contains(&WitnessKind::Cost),
        "learned as a repair, not as something else: {kinds:?}"
    );
}

#[test]
fn a_repair_is_invisible_when_the_harness_says_nothing_about_its_tools() {
    // The same three turns, minus `ok` and `args`. This is the state the transcript was in
    // before this work: two indistinguishable lines of tool output, and nothing to learn.
    let mut store = Store::ephemeral().expect("store");
    let mut engine = Engine::new();
    let settings = Settings::default();
    let mut held = Transcript::ephemeral().expect("scrollback");
    let session = SessionId::new("01BLIND");

    let bare = |cursor: u64, text: &str| Turn {
        cursor,
        at: NOW,
        role: "tool".into(),
        kind: "tool_result".into(),
        text: text.to_owned(),
        tool: Some("shell".to_owned()),
        ..Turn::default()
    };
    streamed(
        &mut held,
        &session,
        vec![said(1, "run the tests"), bare(2, "failed"), bare(3, "ok")],
    );

    distil_run(
        &mut store,
        &mut engine,
        &settings,
        &held,
        &session,
        &ask(false),
    )
    .expect("distil");

    let kinds: Vec<WitnessKind> = store
        .all()
        .expect("all")
        .iter()
        .flat_map(|m| store.witnesses_of(&m.id).unwrap_or_default())
        .map(|w| w.kind)
        .collect();
    assert!(
        !kinds.contains(&WitnessKind::Cost),
        "a scar cannot be invented from output nobody labelled: {kinds:?}"
    );
}

#[test]
fn a_run_with_no_tool_detail_is_read_without_complaint() {
    // The ordinary case for a harness that streams prose and says nothing about its tools. It
    // must produce no repair rather than failing, because inventing an `ok` would invent a scar.
    let mut store = Store::ephemeral().expect("store");
    let mut engine = Engine::new();
    let settings = Settings::default();
    let mut held = Transcript::ephemeral().expect("scrollback");
    let session = SessionId::new("01QUIET");

    streamed(
        &mut held,
        &session,
        vec![said(1, "what does this do?"), said(2, "thanks")],
    );

    let report = distil_run(
        &mut store,
        &mut engine,
        &settings,
        &held,
        &session,
        &ask(false),
    )
    .expect("distil");
    assert_eq!(report.sessions, 1, "the run was read");
    assert_eq!(report.promoted, 0, "and taught nothing, which is correct");
}

#[test]
fn reading_a_run_twice_teaches_nothing_the_second_time() {
    // Stamped like an ingest, so `memo consolidate` can run on a timer without re-reading every
    // run in the project on every pass.
    let mut store = Store::ephemeral().expect("store");
    let mut engine = Engine::new();
    let settings = Settings::default();
    let mut held = Transcript::ephemeral().expect("scrollback");
    let session = SessionId::new("01TWICE");

    streamed(
        &mut held,
        &session,
        vec![said(1, "remember: the deploy target is fly.io")],
    );

    let unread = undistilled(&store, &held, 10).expect("unread");
    assert_eq!(unread, vec![session.clone()], "it is waiting to be read");

    let first = distil_run(
        &mut store,
        &mut engine,
        &settings,
        &held,
        &session,
        &ask(false),
    )
    .expect("first");
    assert_eq!(first.promoted, 1);

    let again = distil_run(
        &mut store,
        &mut engine,
        &settings,
        &held,
        &session,
        &ask(false),
    )
    .expect("again");
    assert_eq!(again.already_read, 1);
    assert_eq!(again.sessions, 0, "it was not read a second time");

    assert!(
        undistilled(&store, &held, 10).expect("unread").is_empty(),
        "and it no longer offers itself"
    );
}

#[test]
fn a_rehearsal_writes_nothing() {
    let mut store = Store::ephemeral().expect("store");
    let mut engine = Engine::new();
    let settings = Settings::default();
    let mut held = Transcript::ephemeral().expect("scrollback");
    let session = SessionId::new("01DRY");

    streamed(
        &mut held,
        &session,
        vec![said(1, "remember: we deploy with fly.io")],
    );

    let report = distil_run(
        &mut store,
        &mut engine,
        &settings,
        &held,
        &session,
        &ask(true),
    )
    .expect("dry");
    assert_eq!(report.promoted, 1, "it says what would cross");
    assert!(
        store.all().expect("all").is_empty(),
        "and the store is untouched"
    );
    assert_eq!(
        undistilled(&store, &held, 10).expect("unread"),
        vec![session],
        "a rehearsal does not stamp the run as read"
    );
}
