//! `plan` and `model`: what a harness should send, and for which model.

use crate::Answering;
use balthasar_buffer::{Plan, Window};
use balthasar_ipc::{Reply, Request};
use balthasar_model::SessionId;

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
