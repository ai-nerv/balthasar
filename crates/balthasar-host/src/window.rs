//! `observe` and `plan` — the short-term half of the surface.

use crate::Answering;
use balthasar_buffer::{Plan, Window};
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

/// Say what model this run talks to, and how much it holds.
///
/// balthasar does the compacting, so it has to know the size of the thing it compacts for.
pub fn model(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("model needs a session");
    };
    let session = SessionId::new(session);
    let Some(scrollback) = at.scrollback.as_mut() else {
        return Reply::refused("this balthasar keeps no scrollback");
    };

    let said = request.args.get(1);
    let Some(name) = said
        .and_then(|s| s.get("model"))
        .and_then(serde_json::Value::as_str)
    else {
        // Asking rather than telling.
        return match scrollback.model_of(&session) {
            Ok(Some((name, context))) => {
                Reply::one(serde_json::json!({ "model": name, "context": context }))
            }
            Ok(None) => Reply::one(serde_json::json!({ "model": null, "context": null })),
            Err(why) => Reply::refused(why.to_string()),
        };
    };
    let context = said
        .and_then(|s| s.get("context"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(Window::default().size);

    match scrollback.note_model(&session, name, context) {
        Ok(()) => Reply::one(serde_json::json!({ "model": name, "context": context })),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// Say what a harness should send.
///
/// `describe` is the configured stub for a tool; without one a tool row gets its own stub or
/// the generic one. Nothing is recorded: a plan is a proposal, and `applied` is what records.
pub fn plan(
    at: &mut Answering<'_>,
    request: &Request,
    mut describe: impl FnMut(&balthasar_store::Turn) -> Option<String>,
) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("plan needs a session");
    };
    let session = SessionId::new(session);
    let said = request.args.get(1);

    let Some(scrollback) = at.scrollback.as_mut() else {
        return Reply::refused("planning needs a scrollback to plan over");
    };

    // What the harness said, else what the run recorded, else the shipped guess.
    let fallback = Window::default();
    let noted = scrollback.model_of(&session).ok().flatten();
    let window = Window {
        size: number(
            said,
            "window",
            noted.as_ref().map_or(fallback.size, |(_, size)| *size),
        ),
        reserve: number(said, "reserve", fallback.reserve),
        inject: number(said, "inject", fallback.inject),
        mask_over: number(said, "mask_over", fallback.mask_over),
        keep: number(said, "keep", fallback.keep as u32) as usize,
        masked_cost: fallback.masked_cost,
    };

    let entries = match scrollback.in_window(&session) {
        Ok(entries) => entries,
        Err(why) => return Reply::refused(why.to_string()),
    };
    if entries.is_empty() {
        return Reply::refused(format!(
            "nothing has been observed for '{session}' — stream turns before asking what to send"
        ));
    }

    let plan = balthasar_buffer::plan(&entries, &window, |turn| stub_for(turn, &mut describe));
    Reply::one(as_json(&plan))
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

/// What a stubbed tool row says: a configured handler first, then the tool's own words, then a
/// generic line. A turn that is not a tool's gets nothing.
pub(crate) fn stub_for(
    turn: &balthasar_store::Turn,
    describe: &mut impl FnMut(&balthasar_store::Turn) -> Option<String>,
) -> Option<String> {
    if !turn.is_tool() {
        return None;
    }
    if let Some(configured) = describe(turn) {
        return Some(configured);
    }
    let said = turn.stub.clone().unwrap_or_else(|| {
        format!(
            "`{}` result elided (~{} tokens)",
            turn.tool.as_deref().unwrap_or("tool"),
            turn.weight()
        )
    });
    Some(match &turn.handle {
        Some(handle) if !said.contains(handle.as_str()) => {
            format!("{said} — `{handle}` to see it again")
        }
        _ => said,
    })
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

/// Everything a run said, in order.
///
/// What a harness restores from. Turns come back as they finally stood.
pub fn replay(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("replay needs a session");
    };
    let session = SessionId::new(session);
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("this balthasar keeps no scrollback");
    };
    // Whole on purpose, and only here: everything wanting part of a scrollback asks `scroll`. With
    // `{from, bytes}` it comes a page at a time, since a long run in one frame was past the limit.
    let paging = request.args.get(1);
    let from = paging.and_then(|p| p["from"].as_u64()).unwrap_or(0);
    let budget = paging
        .and_then(|p| p["bytes"].as_u64())
        .and_then(|b| usize::try_from(b).ok());
    match scrollback.replay(&session) {
        Ok(turns) => {
            let mut rows = Vec::new();
            let mut spent = 0;
            for turn in turns.iter().filter(|turn| turn.cursor >= from) {
                let row = serde_json::json!(turn);
                if let Some(budget) = budget {
                    let size = row.to_string().len();
                    if !rows.is_empty() && spent + size > budget {
                        break;
                    }
                    spent += size;
                }
                rows.push(row);
            }
            Reply::rows(rows)
        }
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// `scroll(session, {want, from, to, cursor, terms, tokens, turns})`.
///
/// Part of a scrollback, within a budget.
///
/// The reply carries what was left out and where to continue from.
pub fn scroll(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("scroll needs a session");
    };
    let session = SessionId::new(session);
    let said = request.args.get(1);
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("this balthasar keeps no scrollback");
    };

    let number = |name: &str| -> Option<u64> {
        said.and_then(|s| s.get(name))
            .and_then(serde_json::Value::as_u64)
    };
    let budget = balthasar_store::Budget {
        tokens: number("tokens").unwrap_or(4_000) as usize,
        turns: number("turns").unwrap_or(200) as usize,
    };

    let want = match said
        .and_then(|s| s.get("want"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("tail")
    {
        "tail" => balthasar_store::Want::Tail,
        "span" => match (number("from"), number("to")) {
            (Some(from), Some(to)) => balthasar_store::Want::Span { from, to },
            _ => return Reply::refused("a span needs `from` and `to`"),
        },
        "around" => match number("cursor") {
            Some(cursor) => balthasar_store::Want::Around { cursor },
            None => return Reply::refused("`around` needs a cursor"),
        },
        "matching" => balthasar_store::Want::Matching {
            terms: said
                .and_then(|s| s.get("terms"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        },
        other => {
            return Reply::refused(format!(
                "no such read: '{other}' — try tail, span, around or matching"
            ));
        }
    };

    match scrollback.read(&session, &want, &budget) {
        Err(why) => Reply::refused(why.to_string()),
        Ok(held) => {
            // What a model reads back, inside a window it is trying to save: what was said,
            // and never the harness's own record of it, which is several times the size and
            // is `replay`'s to give.
            let turns: Vec<serde_json::Value> = held
                .turns
                .iter()
                .filter_map(|turn| serde_json::to_value(turn).ok())
                .map(|mut turn| {
                    if let Some(fields) = turn.as_object_mut() {
                        fields.remove("raw");
                    }
                    turn
                })
                .collect();
            Reply::one(serde_json::json!({
                "turns": turns,
                "tokens": held.tokens,
                "omitted": held.omitted,
                "next": held.next,
                "complete": held.is_complete(),
            }))
        }
    }
}

/// Where a restarting harness left off.
///
/// Guessing the next cursor wrong overwrites a turn that nothing else holds a copy of.
pub fn resume(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("resume needs a session");
    };
    let session = SessionId::new(session);
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("this balthasar keeps no scrollback");
    };
    let next = match scrollback.next_cursor(&session) {
        Ok(next) => next,
        Err(why) => return Reply::refused(why.to_string()),
    };
    let turns = scrollback.replay(&session).map(|t| t.len()).unwrap_or(0);
    let run = match scrollback.run_of(&session) {
        Ok(run) => run,
        Err(why) => return Reply::refused(why.to_string()),
    };
    Reply::one(serde_json::json!({ "next": next, "turns": turns, "run": run.as_str() }))
}

/// A number a harness sent, or the shipped default.
fn number(said: Option<&serde_json::Value>, name: &str, fallback: u32) -> u32 {
    said.and_then(|s| s.get(name))
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(fallback)
}

/// The plan, as a harness receives it.
fn as_json(plan: &Plan) -> serde_json::Value {
    serde_json::json!({
        "keep": plan.keep,
        "mask": plan.mask.iter().map(|m| serde_json::json!({
            "cursor": m.cursor,
            "as": m.r#as,
            "was": m.was,
        })).collect::<Vec<_>>(),
        "drop": plan.drop,
        "summarise": plan.summarise.map(|s| serde_json::json!({ "from": s.from, "to": s.to })),
        "budget": {
            "window": plan.target,
            "was": plan.was,
            "after": plan.used,
        },
        "fits": plan.fits,
        "why": plan.why,
    })
}
