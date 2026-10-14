//! `observe` and `amend`: how a turn is recorded.

use crate::Answering;
use balthasar_ipc::{Reply, Request};
use balthasar_model::{Body, Memory, NoteKind, SessionId, Tier};
use balthasar_store::{State, mint};

/// Record one turn.
///
/// The turn goes to the scrollback, which is the only record of the conversation.
pub fn observe(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("observe needs a session");
    };
    let Some(turn) = request.args.get(1) else {
        return Reply::refused("observe needs a turn");
    };
    let session = SessionId::new(session);

    if at.scrollback.is_none() {
        return Reply::failed("this balthasar keeps no scrollback: the turn was not recorded");
    }
    let run = match bind_run(at, &session, turn) {
        Ok(run) => run,
        Err(why) => return Reply::refused(why),
    };

    // Recorded on first sight rather than requiring a harness to open one first.
    if let Some(scrollback) = at.scrollback.as_mut() {
        let _ = scrollback.open_run(
            &session,
            &at.scope.to_string(),
            &at.scope.to_string(),
            "peer",
            at.now,
        );
    }
    // Twice on purpose: the project keeps the registry, the run keeps its own row.
    let (scope, now) = (at.scope.clone(), at.now);
    if let Err(why) = at
        .store
        .open_session(&session, &scope, &scope.to_string(), "peer", now)
    {
        return Reply::refused(why.to_string());
    }
    match at.run(&session) {
        Ok(run) => {
            if let Err(why) = run.open_session(&session, &scope, &scope.to_string(), "peer", now) {
                return Reply::refused(why.to_string());
            }
        }
        Err(why) => return Reply::refused(why.to_string()),
    }

    // The scrollback first, and durably: nothing else happens until it is.
    let Some(scrollback) = at.scrollback.as_mut() else {
        return Reply::failed("this balthasar keeps no scrollback: the turn was not recorded");
    };
    let verbatim = turn_of(&session, turn, at.now);
    if let Err(why) = scrollback.write(&session, &verbatim) {
        return Reply::failed(why.to_string());
    }

    let cursor = turn
        .get("cursor")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let role = turn
        .get("role")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("user")
        .to_owned();
    let kind = turn
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("prose")
        .to_owned();
    let tool = turn
        .get("tool")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let text = turn
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    // Scratch, not fact. Observing is not what carries a claim across the ladder.
    let memory = if text.is_empty() {
        None
    } else {
        let mut held = Memory::new(
            mint(at.now),
            Tier::Scratch,
            at.scope.clone(),
            Body::note(text, NoteKind::Observation),
            at.now,
        );
        held.session = Some(run.clone());
        // The id that actually holds the text, which is not always the one that went in.
        let landed = match at.run(&session) {
            Ok(run) => run.keep_scratch(held),
            Err(why) => return Reply::refused(why.to_string()),
        };
        match landed {
            Ok(id) => Some(id),
            Err(why) => return Reply::refused(why.to_string()),
        }
    };

    // The first thing asked is the closest thing to a name a session has.
    if role == "user" && !text.is_empty() {
        let _ = at.store.title_session(&session, text);
        let _ = at
            .run(&session)
            .map(|run| run.title_session(&session, text));
    }

    let _ = (cursor, kind, tool, memory);
    Reply::none()
}

/// The turn a harness sent, in the shape the scrollback keeps.
///
/// `raw` is whatever the harness's own record is, carried verbatim and never parsed.
fn turn_of(
    session: &SessionId,
    turn: &serde_json::Value,
    now: balthasar_model::Timestamp,
) -> balthasar_store::Turn {
    let text = turn
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let _ = session;
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
        // What the extractors will read later.
        ok: turn.get("ok").and_then(serde_json::Value::as_bool),
        ms: turn.get("ms").and_then(serde_json::Value::as_u64),
        args: turn.get("args").map(ToString::to_string),
        cursor: turn
            .get("cursor")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
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
        // What the harness says about a tool row, all of it optional.
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

/// Revise the turn already at a cursor.
///
/// The same cursor is written twice when a tool call's result arrives, and the second write wins.
pub fn amend(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("amend needs a session");
    };
    let Some(turn) = request.args.get(1) else {
        return Reply::refused("amend needs a turn");
    };
    let session = SessionId::new(session);
    if let Err(why) = bind_run(at, &session, turn) {
        return Reply::refused(why);
    }
    let held = turn_of(&session, turn, at.now);

    let Some(scrollback) = at.scrollback.as_mut() else {
        return Reply::refused(
            "this balthasar keeps no scrollback — a harness relying on it for persistence must be \
             served by one that does",
        );
    };
    match scrollback.write(&session, &held) {
        Ok(()) => Reply::none(),
        Err(why) => Reply::refused(why.to_string()),
    }
}

fn bind_run(
    at: &Answering<'_>,
    session: &SessionId,
    turn: &serde_json::Value,
) -> Result<SessionId, String> {
    let run = match turn.get("run") {
        None => None,
        Some(serde_json::Value::String(run)) => Some(run.as_str()),
        Some(_) => return Err("turn.run must be a non-empty string".into()),
    };
    let transcript = at
        .scrollback
        .as_ref()
        .ok_or("this balthasar keeps no scrollback")?;
    transcript
        .bind_run(session, run)
        .map_err(|why| why.to_string())?;
    transcript.run_of(session).map_err(|why| why.to_string())
}
