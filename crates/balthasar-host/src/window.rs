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
        held.session = Some(session.clone());
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
/// `describe` is the mask handler for a tool; a turn nobody can describe is left alone.
pub fn plan(
    at: &mut Answering<'_>,
    request: &Request,
    describe: impl FnMut(&balthasar_store::Turn) -> Option<String>,
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

    let plan = balthasar_buffer::plan(&entries, &window, describe);
    // The plan is recorded as it is handed over, so asking again does not mask twice.
    for masked in &plan.mask {
        let _ = scrollback.mark(&session, masked.cursor, State::Masked);
    }
    if let Some(span) = plan.summarise {
        for cursor in &plan.drop {
            if *cursor >= span.from && *cursor <= span.to {
                let _ = scrollback.mark(&session, *cursor, State::Summarised);
            }
        }
        // TIDE. What was in a span leaving the window becomes a candidate, never a fact.
        let _ = distil(at, &session, &entries, span);
    }

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
    }
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
    // Unbounded on purpose, and only here. Everything wanting part of a scrollback asks `scroll`.
    match scrollback.replay(&session) {
        Ok(turns) => Reply::one(serde_json::json!(turns)),
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
        Ok(held) => Reply::one(serde_json::json!({
            "turns": held.turns,
            "tokens": held.tokens,
            "omitted": held.omitted,
            "next": held.next,
            "complete": held.is_complete(),
        })),
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
    Reply::one(serde_json::json!({ "next": next, "turns": turns }))
}

/// Attach a distillation witness to everything a summary is about to stand in for.
///
/// One witness per turn, weighted below the promotion floor.
fn distil(
    at: &mut Answering<'_>,
    session: &SessionId,
    entries: &[balthasar_store::Turn],
    span: balthasar_buffer::Span,
) -> Result<usize, balthasar_store::StoreError> {
    let mut carried = 0;
    for entry in entries {
        if entry.cursor < span.from || entry.cursor > span.to {
            continue;
        }
        let scope = at.scope.clone();
        let Some(id) = at.run(session)?.scratch_for(&scope, session, &entry.text)? else {
            continue;
        };
        let witness = balthasar_model::Witness::new(
            balthasar_model::WitnessId::new(format!("tide-{}-{}", session, entry.cursor)),
            balthasar_model::WitnessKind::Distillation,
            session.clone(),
            at.scope.clone(),
            entry.at,
        )
        .at_cursor(entry.cursor)
        .noted("left the context window (rules, not a model)");
        // In the run's own store, which is where the memory a summary stands in for lives.
        let now = at.now;
        at.run(session)?.attach(&id, witness, now)?;
        carried += 1;
    }
    Ok(carried)
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
