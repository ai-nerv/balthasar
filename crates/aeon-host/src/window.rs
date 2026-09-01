//! `observe` and `plan` — the short-term half of the surface.
//!
//! These two are why the connection is held. A harness streams every turn as it settles and
//! asks what to send before every request, which is several calls per turn rather than the
//! occasional poll a control socket sees.

use crate::Answering;
use aeon_buffer::{Plan, Window};
use aeon_ipc::{Reply, Request};
use aeon_model::{Body, Memory, NoteKind, SessionId, Tier};
use aeon_store::{Entry, State, mint};

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
    // that streamed turns and had no name was invisible to `aeon sessions` and to
    // `aeon promote`, so what it held could not be looked at or kept.
    if let Err(why) = at.store.open_session(
        &session,
        &at.scope.clone(),
        &at.scope.to_string(),
        "peer",
        at.now,
    ) {
        return Reply::refused(why.to_string());
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
        match at.store.keep_scratch(held) {
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
    }

    match at.store.observe(&session, &entry) {
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

    let entries = match at.store.ledger(&session) {
        Ok(entries) => entries,
        Err(why) => return Reply::refused(why.to_string()),
    };
    if entries.is_empty() {
        return Reply::refused(format!(
            "nothing has been observed for '{session}' — stream turns before asking what to send"
        ));
    }

    let plan = aeon_buffer::plan(&entries, &window, describe);
    // The plan is recorded as it is handed over. A harness that applies it and then asks again
    // must not be told to mask what it has already masked.
    for masked in &plan.mask {
        let _ = at.store.mark(&session, masked.cursor, State::Masked);
    }
    if let Some(span) = plan.summarise {
        for cursor in &plan.drop {
            if *cursor >= span.from && *cursor <= span.to {
                let _ = at.store.mark(&session, *cursor, State::Summarised);
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

/// Attach a distillation witness to everything a summary is about to stand in for.
///
/// One witness per turn, weighted below the promotion floor. What it buys is that a *second*
/// witness — the same thing recurring in another run, or a person confirming it — now has
/// something to land on rather than starting from nothing.
fn distil(
    at: &mut Answering<'_>,
    session: &SessionId,
    entries: &[Entry],
    span: aeon_buffer::Span,
) -> Result<usize, aeon_store::StoreError> {
    let mut carried = 0;
    for entry in entries {
        if entry.cursor < span.from || entry.cursor > span.to {
            continue;
        }
        let Some(id) = &entry.memory else { continue };
        let witness = aeon_model::Witness::new(
            aeon_model::WitnessId::new(format!("tide-{}-{}", session, entry.cursor)),
            aeon_model::WitnessKind::Distillation,
            session.clone(),
            at.scope.clone(),
            entry.at,
        )
        .at_cursor(entry.cursor)
        .noted("left the context window (rules, not a model)");
        at.store.attach(id, witness, at.now)?;
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
