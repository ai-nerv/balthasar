//! `observe` and `amend`: how a turn is recorded.
//!
//! The scrollback is written first and alone. A turn it did not take is `failed`; a call that
//! could never be taken is `refused`. Everything after the scrollback is best-effort and logged.

use crate::Answering;
use balthasar_ipc::{Reply, Request};
use balthasar_model::{Body, Memory, NoteKind, SessionId, Tier};
use balthasar_store::{Inserted, State, mint};

/// What every recording call carries, checked before anything is written.
struct Asked<'a> {
    session: SessionId,
    turn: &'a serde_json::Value,
    cursor: u64,
    run: Option<&'a str>,
}

/// Read the session, the turn, its cursor and its run, refusing any that is missing or malformed.
fn asked<'a>(verb: &str, request: &'a Request) -> Result<Asked<'a>, String> {
    let session = request
        .args
        .first()
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{verb} needs a session"))?;
    let turn = request
        .args
        .get(1)
        .ok_or_else(|| format!("{verb} needs a turn"))?;
    let cursor = turn
        .get("cursor")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("{verb} needs turn.cursor, a whole number of zero or more"))?;
    let run = match turn.get("run") {
        None => None,
        Some(serde_json::Value::String(run)) if !run.trim().is_empty() => Some(run.as_str()),
        Some(_) => return Err("turn.run must be a non-empty string".into()),
    };
    Ok(Asked {
        session: SessionId::new(session),
        turn,
        cursor,
        run,
    })
}

/// Record one turn.
///
/// Insert-only: a cursor that already holds the same turn is a retry and answers yes; one that
/// holds a different turn is refused, and `amend` is how a turn changes. A `trace` row is kept
/// beside the turns, never at one of their places.
pub fn observe(at: &mut Answering<'_>, request: &Request) -> Reply {
    let asked = match asked("observe", request) {
        Ok(asked) => asked,
        Err(why) => return Reply::refused(why),
    };
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::failed("this balthasar keeps no scrollback: the turn was not recorded");
    };
    let verbatim = turn_of(asked.turn, asked.cursor, at.now);

    if verbatim.role == "trace" {
        return match scrollback.notice(
            &asked.session,
            asked.cursor,
            verbatim.at,
            &verbatim.kind,
            &verbatim.text,
        ) {
            Ok(_) => Reply::none(),
            Err(why) => Reply::failed(format!("the trace was not recorded: {why}")),
        };
    }

    let run = match scrollback.run_for(&asked.session, asked.run) {
        Ok(run) => run,
        Err(balthasar_store::StoreError::Foreign(why)) => return Reply::refused(why),
        Err(why) => return Reply::failed(format!("the turn was not recorded: {why}")),
    };
    match scrollback.insert(&asked.session, &verbatim) {
        Ok(Inserted::Fresh) => {}
        Ok(Inserted::Same) => return Reply::none(),
        Ok(Inserted::Different) => {
            return Reply::refused(format!(
                "cursor {} already holds a different turn",
                asked.cursor
            ));
        }
        Err(why) => return Reply::failed(format!("the turn was not recorded: {why}")),
    }

    afterwards(at, &asked.session, &run, &verbatim);
    Reply::none()
}

/// What follows a recorded turn: the binding, the run registries, scratch and a title. None of it
/// can unrecord the turn, so each step that fails is logged and the rest carry on.
fn afterwards(
    at: &mut Answering<'_>,
    session: &SessionId,
    run: &str,
    turn: &balthasar_store::Turn,
) {
    let (scope, now) = (at.scope.clone(), at.now);
    let failed = |step: &str, why: &dyn std::fmt::Display| {
        balthasar_model::noted!(
            "observe: {session} cursor {}: {step} failed: {why}",
            turn.cursor
        );
    };
    if let Some(scrollback) = at.scrollback.as_mut() {
        if let Err(why) = scrollback.pin_run(session, run) {
            failed("binding the run", &why);
        }
        if let Err(why) = scrollback.open_run(session, scope.as_str(), scope.as_str(), "peer", now)
        {
            failed("opening the run", &why);
        }
    }
    if let Err(why) = at
        .store
        .open_session(session, &scope, &scope.to_string(), "peer", now)
    {
        failed("registering the session", &why);
    }
    match at.run(session) {
        Ok(store) => {
            if let Err(why) = store.open_session(session, &scope, &scope.to_string(), "peer", now) {
                failed("opening the run's session", &why);
            }
        }
        Err(why) => failed("opening the run's store", &why),
    }

    // Only what a person or the model said is mirrored into scratch; a tool's output is not.
    let said = matches!(turn.role.as_str(), "user" | "assistant") && !turn.is_tool();
    if said && !turn.text.is_empty() {
        let mut held = Memory::new(
            mint(now),
            Tier::Scratch,
            scope.clone(),
            Body::note(&turn.text, NoteKind::Observation),
            now,
        );
        held.session = Some(SessionId::new(run));
        if turn.role == "assistant" {
            held.provenance.who = Some(balthasar_store::SAID_BY_MODEL.to_owned());
        }
        match at.run(session) {
            Ok(store) => {
                if let Err(why) = store.keep_scratch(held) {
                    failed("keeping it in scratch", &why);
                }
            }
            Err(why) => failed("opening the run's store", &why),
        }
    }

    // The first thing asked is the closest thing to a name a session has.
    if turn.role == "user" && !turn.text.is_empty() {
        let _ = at.store.title_session(session, &turn.text);
        let _ = at
            .run(session)
            .map(|run| run.title_session(session, &turn.text));
    }
}

/// The turn a harness sent, in the shape the scrollback keeps.
///
/// `raw` is whatever the harness's own record is, carried verbatim and never parsed.
fn turn_of(
    turn: &serde_json::Value,
    cursor: u64,
    now: balthasar_model::Timestamp,
) -> balthasar_store::Turn {
    let text = turn
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    balthasar_store::Turn {
        // Which message this block belongs to, when the harness splits one into several.
        pinned: turn
            .get("pinned")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        state: State::default(),
        entry: turn
            .get("entry")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        // What the harness was charged, when it says. Absent is not zero, and a reader estimates.
        tokens: turn
            .get("tokens")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        ok: turn.get("ok").and_then(serde_json::Value::as_bool),
        ms: turn.get("ms").and_then(serde_json::Value::as_u64),
        args: turn.get("args").map(ToString::to_string),
        cursor,
        at: turn
            .get("at")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(now),
        // Not "user". A turn arriving without a role is a turn nobody vouched for.
        role: turn
            .get("role")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("other")
            .to_owned(),
        kind: turn
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("prose")
            .to_owned(),
        text: text.to_owned(),
        tool: turn
            .get("tool")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        // A string if the harness sent one, otherwise the whole object it sent, byte for byte.
        raw: turn
            .get("raw")
            .map(|raw| raw.as_str().map_or_else(|| raw.to_string(), str::to_owned)),
        revisions: 0,
        group: turn.get("group").and_then(serde_json::Value::as_u64),
        stub: said(turn, "stub"),
        handle: said(turn, "handle"),
        keep: flag(turn, "keep"),
        error: flag(turn, "error"),
    }
}

/// A non-empty string field.
fn said(turn: &serde_json::Value, name: &str) -> Option<String> {
    turn.get(name)
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
}

/// A boolean field, false when absent.
fn flag(turn: &serde_json::Value, name: &str) -> bool {
    turn.get(name)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Revise the turn already at a cursor, keeping what it said before.
///
/// The one verb that changes a turn: a tool call is observed when it is made and amended when its
/// result arrives.
pub fn amend(at: &mut Answering<'_>, request: &Request) -> Reply {
    let asked = match asked("amend", request) {
        Ok(asked) => asked,
        Err(why) => return Reply::refused(why),
    };
    let held = turn_of(asked.turn, asked.cursor, at.now);
    let Some(scrollback) = at.scrollback.as_mut() else {
        return Reply::failed(
            "this balthasar keeps no scrollback — a harness relying on it for persistence must be \
             served by one that does",
        );
    };
    let run = match scrollback.run_for(&asked.session, asked.run) {
        Ok(run) => run,
        Err(balthasar_store::StoreError::Foreign(why)) => return Reply::refused(why),
        Err(why) => return Reply::failed(format!("the turn was not recorded: {why}")),
    };
    if let Err(why) = scrollback.write(&asked.session, &held) {
        return Reply::failed(format!("the turn was not recorded: {why}"));
    }
    if let Err(why) = scrollback.pin_run(&asked.session, &run) {
        balthasar_model::noted!("amend: {}: binding the run failed: {why}", asked.session);
    }
    Reply::none()
}
