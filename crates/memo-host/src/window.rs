//! `observe` and `plan` — the short-term half of the surface.
//!
//! These two are why the connection is held. A harness streams every turn as it settles and
//! asks what to send before every request, which is several calls per turn rather than the
//! occasional poll a control socket sees.

use crate::Answering;
use memo_buffer::{Plan, Window};
use memo_ipc::{Reply, Request};
use memo_model::{Body, Memory, NoteKind, SessionId, Tier};
use memo_store::{Entry, State, mint};

/// Record one turn.
///
/// The text goes into scratch, which is the session's own; the ledger row is its place in the
/// window. Two records because they answer different questions and change at different rates —
/// the claim may outlive every session, the row is rewritten on every plan.
pub fn observe(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("observe needs a session");
    };
    let Some(turn) = request.args.get(1) else {
        return Reply::refused("observe needs a turn");
    };
    let session = SessionId::new(session);

    // Recorded on first sight rather than requiring a harness to open one first. A session
    // that streamed turns and had no name was invisible to `memo sessions` and to
    // `memo promote`, so what it held could not be looked at or kept.
    if let Some(scrollback) = at.scrollback.as_mut() {
        let _ = scrollback.open_run(
            &session,
            &at.scope.to_string(),
            &at.scope.to_string(),
            "peer",
            at.now,
        );
    }
    // Twice on purpose. The project keeps the registry, so `memo sessions` can list runs
    // without opening every one of them; the run keeps its own row, so its scratch has
    // something to point at.
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

    // The scrollback first, and durably. For a harness with no journal of its own this call
    // answering is the only signal that the turn is safe, so nothing else happens until it is.
    if let Some(scrollback) = at.scrollback.as_mut() {
        let verbatim = turn_of(&session, turn, at.now);
        if let Err(why) = scrollback.write(&session, &verbatim) {
            return Reply::refused(why.to_string());
        }
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

    // The harness's own count when it has one. It billed the request; we estimated.
    let tokens = turn
        .get("tokens")
        .and_then(serde_json::Value::as_u64)
        .map_or_else(
            || u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX),
            |n| u32::try_from(n).unwrap_or(u32::MAX),
        );

    // Scratch, not fact. What a session says is the session's until something on the ladder
    // carries it across, and observing is not that something.
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
        // The id that actually holds the text, which is not always the one that went in: a
        // session repeating itself gets one row and several references to it.
        let landed = match at.run(&session) {
            Ok(run) => run.keep_scratch(held),
            Err(why) => return Reply::refused(why.to_string()),
        };
        match landed {
            Ok(id) => Some(id),
            Err(why) => return Reply::refused(why.to_string()),
        }
    };

    let entry = Entry {
        cursor,
        memory,
        role,
        kind,
        tool,
        tokens,
        state: State::Live,
        pinned: turn
            .get("pinned")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        at: at.now,
    };

    // The first thing asked is what the run was for, which is the closest thing to a name a
    // session has without asking a model to invent one.
    if entry.role == "user" && !text.is_empty() {
        let _ = at.store.title_session(&session, text);
        let _ = at
            .run(&session)
            .map(|run| run.title_session(&session, text));
    }

    match at
        .run(&session)
        .and_then(|run| run.observe(&session, &entry))
    {
        Ok(()) => Reply::none(),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// Say what a harness should send.
///
/// `describe` is the configuration's mask handler for a tool. Only the tool's author knows what
/// a useful stub says, so a turn nobody can describe is left alone rather than replaced with
/// something uninformative.
pub fn plan(
    at: &mut Answering<'_>,
    request: &Request,
    describe: impl FnMut(&Entry) -> Option<String>,
) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("plan needs a session");
    };
    let session = SessionId::new(session);
    let said = request.args.get(1);

    let fallback = Window::default();
    let window = Window {
        size: number(said, "window", u32::from(fallback.size > 0) * fallback.size),
        reserve: number(said, "reserve", fallback.reserve),
        inject: number(said, "inject", fallback.inject),
        mask_over: number(said, "mask_over", fallback.mask_over),
        keep: number(said, "keep", fallback.keep as u32) as usize,
        masked_cost: fallback.masked_cost,
    };

    let entries = match at.run(&session).and_then(|run| run.ledger(&session)) {
        Ok(entries) => entries,
        Err(why) => return Reply::refused(why.to_string()),
    };
    if entries.is_empty() {
        return Reply::refused(format!(
            "nothing has been observed for '{session}' — stream turns before asking what to send"
        ));
    }

    let plan = memo_buffer::plan(&entries, &window, describe);
    // The plan is recorded as it is handed over. A harness that applies it and then asks again
    // must not be told to mask what it has already masked.
    for masked in &plan.mask {
        let _ = at
            .run(&session)
            .map(|run| run.mark(&session, masked.cursor, State::Masked));
    }
    if let Some(span) = plan.summarise {
        for cursor in &plan.drop {
            if *cursor >= span.from && *cursor <= span.to {
                let _ = at
                    .run(&session)
                    .map(|run| run.mark(&session, *cursor, State::Summarised));
            }
        }
        // TIDE. The moment a span leaves the window is the last moment anybody will look at
        // it, which makes it the cheapest extraction point in the whole system and the one
        // every harness throws away. What was in it becomes a candidate — not a fact: a
        // distillation is worth less than the promotion floor precisely so that something
        // which merely scrolled past cannot become a belief on its own.
        let _ = distil(at, &session, &entries, span);
    }

    Reply::one(as_json(&plan))
}

/// The turn a harness sent, in the shape the scrollback keeps.
///
/// `raw` is whatever the harness's own record is, carried verbatim and never parsed. That is
/// what lets a harness with no journal of its own treat this as one: it gets back exactly what
/// it wrote, and memo never needs to know what an entry means.
fn turn_of(
    session: &SessionId,
    turn: &serde_json::Value,
    now: memo_model::Timestamp,
) -> memo_store::Turn {
    let text = turn
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let _ = session;
    memo_store::Turn {
        // Which message this block belongs to, when the harness splits one into several. Absent
        // for a turn that is its own message, which is what a plain user turn is.
        entry: turn
            .get("entry")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        cursor: turn
            .get("cursor")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        at: turn
            .get("at")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(now),
        role: turn
            .get("role")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("user")
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
        // A string if the harness sent one, otherwise the whole object it sent. Either way it
        // comes back byte for byte.
        raw: turn
            .get("raw")
            .map(|raw| raw.as_str().map_or_else(|| raw.to_string(), str::to_owned)),
        revisions: 0,
    }
}

/// Revise the turn already at a cursor.
///
/// Not exceptional. A tool call is written when it is made and written again when its result
/// arrives, so the same cursor is written twice and the second write is the one that matters.
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
            "this memo keeps no scrollback — a harness relying on it for persistence must be \
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
/// What a harness restores from. Turns come back as they finally stood, so a tool call arrives
/// with its result rather than as it was first written.
pub fn replay(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("replay needs a session");
    };
    let session = SessionId::new(session);
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("this memo keeps no scrollback");
    };
    // Unbounded on purpose, and only here. `replay` is what a harness restoring a session
    // needs — every turn, in order, byte for byte — and truncating it would mean handing back
    // a session that is quietly missing its beginning. Everything that wants *part* of a
    // scrollback asks `scroll`, which is bounded.
    match scrollback.replay(&session) {
        Ok(turns) => Reply::one(serde_json::json!(turns)),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// `scroll(session, {want, from, to, cursor, terms, tokens, turns})`.
///
/// Part of a scrollback, within a budget. A run's transcript grows without limit — memo is the
/// only copy — and a model's context does not, so this is the read for everything except
/// restoring a session.
///
/// The reply carries what was left out and where to continue from, because a caller cannot
/// otherwise tell "that is all there was" from "that is all you asked for".
pub fn scroll(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("scroll needs a session");
    };
    let session = SessionId::new(session);
    let said = request.args.get(1);
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("this memo keeps no scrollback");
    };

    let number = |name: &str| -> Option<u64> {
        said.and_then(|s| s.get(name))
            .and_then(serde_json::Value::as_u64)
    };
    let budget = memo_store::Budget {
        tokens: number("tokens").unwrap_or(4_000) as usize,
        turns: number("turns").unwrap_or(200) as usize,
    };

    let want = match said
        .and_then(|s| s.get("want"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("tail")
    {
        "tail" => memo_store::Want::Tail,
        "span" => match (number("from"), number("to")) {
            (Some(from), Some(to)) => memo_store::Want::Span { from, to },
            _ => return Reply::refused("a span needs `from` and `to`"),
        },
        "around" => match number("cursor") {
            Some(cursor) => memo_store::Want::Around { cursor },
            None => return Reply::refused("`around` needs a cursor"),
        },
        "matching" => memo_store::Want::Matching {
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
/// A harness with no journal has no other way to know which cursor to allocate next, and
/// guessing wrong overwrites a turn that nothing else holds a copy of.
pub fn resume(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(session) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("resume needs a session");
    };
    let session = SessionId::new(session);
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Reply::refused("this memo keeps no scrollback");
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
/// One witness per turn, weighted below the promotion floor. What it buys is that a *second*
/// witness — the same thing recurring in another run, or a person confirming it — now has
/// something to land on rather than starting from nothing.
fn distil(
    at: &mut Answering<'_>,
    session: &SessionId,
    entries: &[Entry],
    span: memo_buffer::Span,
) -> Result<usize, memo_store::StoreError> {
    let mut carried = 0;
    for entry in entries {
        if entry.cursor < span.from || entry.cursor > span.to {
            continue;
        }
        let Some(id) = &entry.memory else { continue };
        let witness = memo_model::Witness::new(
            memo_model::WitnessId::new(format!("tide-{}-{}", session, entry.cursor)),
            memo_model::WitnessKind::Distillation,
            session.clone(),
            at.scope.clone(),
            entry.at,
        )
        .at_cursor(entry.cursor)
        .noted("left the context window (rules, not a model)");
        // In the run's own store, which is where the memory a summary stands in for lives.
        // Attaching in the project's would be evidence for a memory that is not there.
        let now = at.now;
        at.run(session)?.attach(id, witness, now)?;
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
