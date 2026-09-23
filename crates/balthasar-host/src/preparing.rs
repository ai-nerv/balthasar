//! Preparing the next working state in the background, and installing it when it is sound.
//!
//! The job reads the span about to leave the live set together with the state already active, and
//! answers a new state. What comes back is checked by [`crate::working::Working::may_replace`]
//! before it is written, and written as a generation that is `Ready` — never active. Activating is
//! a separate step at a turn boundary, and needs no model.

use crate::Answering;
use crate::queue::pending;
use crate::working::Working;
use balthasar_model::SessionId;
use balthasar_store::{Budget, Generation, Job, Ready, StoreError, Summary, Want};
use serde_json::{Value, json};

/// What the helper is asked for.
const PREPARE: &str = "You keep the working state of a coding session: what the task is, what has \
been asked for, planned, tried and finished, what was run and what it said, what the person put \
right, and what is still open.\n\
Carry everything still true into your answer. A check that failed stays failed until a row shows \
it run again. Work that is not finished stays on the list. Quote the person's corrections word \
for word.\n\
Cite rows by the cursor in square brackets. Never write that something passed without citing the \
row that says so, and never turn \"not run\" into \"passed\".";

/// How much of one row a preparation reads, and how much of the span altogether.
const OF_EACH: usize = 4_000;
const INPUT: usize = 120_000;

/// The shape the answer must take.
fn schema() -> Value {
    let strings = json!({ "type": "array", "items": { "type": "string" } });
    let cursors = json!({ "type": "array", "items": { "type": "integer", "minimum": 0 } });
    json!({
        "type": "object",
        "properties": {
            "goal": { "type": "string" },
            "work": { "type": "array", "items": { "type": "object",
                "required": ["what", "stage"],
                "properties": {
                    "what": { "type": "string" },
                    "stage": { "enum": ["requested", "planned", "attempted", "done"] },
                    "evidence": cursors } } },
            "checks": { "type": "array", "items": { "type": "object",
                "required": ["ran", "said"],
                "properties": {
                    "ran": { "type": "string" },
                    "passed": { "type": "boolean" },
                    "said": { "type": "string" },
                    "evidence": cursors } } },
            "corrections": { "type": "array", "items": { "type": "object",
                "required": ["said", "cursor"],
                "properties": {
                    "said": { "type": "string" },
                    "cursor": { "type": "integer", "minimum": 0 } } } },
            "open": strings.clone(),
            "references": strings,
        },
        "required": ["goal", "work", "checks", "corrections", "open", "references"],
    })
}

/// Queue a preparation of `from..=to`, folding in whatever is active.
///
/// One at a time, so a slow helper does not stack work behind it, and never for a span already
/// covered: a generation prepared late is activated at a later boundary instead.
pub(crate) fn queue(
    at: &Answering<'_>,
    session: &SessionId,
    span: (u64, u64),
    policy: &Value,
    hooks: &mut dyn crate::Hooks,
) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    let (from, to) = span;
    if from > to || pending(&scrollback.jobs_of(session)?, "working", at.now) {
        return Ok(());
    }
    let active = scrollback.active_generation(session)?;
    if active.as_ref().is_some_and(|held| held.covers.1 >= to) {
        return Ok(());
    }
    let mut input = String::new();
    if let Some(held) = &active {
        input.push_str(&format!("The working state so far:\n{}\n\n", held.content));
    }
    input.push_str(&format!(
        "Rows {from}\u{2013}{to} to fold in, each starting with its cursor in square brackets:\n"
    ));
    let rows = scrollback.read(
        session,
        &Want::Span { from, to },
        &Budget {
            tokens: usize::MAX,
            turns: usize::MAX,
        },
    )?;
    let mut end = None;
    let mut withheld = Vec::new();
    let room = crate::sizing::room(PREPARE, &schema(), INPUT);
    for turn in rows
        .turns
        .iter()
        .filter(|turn| (from..=to).contains(&turn.cursor))
    {
        // A row a configuration keeps from helpers is kept from this one, and what is built out
        // of the span says so rather than reading as a span with nothing in it.
        let (shown, kept_back) = crate::queue::guarded(turn, OF_EACH, hooks);
        if kept_back {
            withheld.push(turn.cursor);
        }
        if !crate::sizing::add(&mut input, &shown, room) {
            break;
        }
        end = Some(turn.cursor);
    }
    let Some(to) = end else {
        return Ok(());
    };
    let spec = json!({
        "kind": "working", "role": "working", "fallback": "skip",
        "instruction": PREPARE, "input": input, "schema": schema(),
        "thinking": "low", "max_tokens": 4_000,
        "blocking": false, "timeout_ms": 90_000, "covers": [from, to],
    });
    let context = json!({
        "from": from, "to": to, "policy": policy,
        "withheld": withheld,
        "parent": active.as_ref().map(|held| held.id.clone()),
    });
    scrollback.queue_job(session, "working", &spec, &context, at.now)?;
    Ok(())
}

/// Install what a preparation answered, as a generation that is ready but not active.
///
/// What is refused leaves the active state standing: the checks are [`Working::may_replace`]'s,
/// and a preparation that drops a failure, unfinished work or a correction is not one.
pub(crate) fn settle(
    at: &Answering<'_>,
    session: &SessionId,
    job: &Job,
    text: &str,
) -> Result<bool, StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(false);
    };
    let Some(proposed) = crate::queue::json_in(text).as_ref().and_then(Working::read) else {
        balthasar_model::noted!("working: {} answered no working state", job.id);
        return Ok(false);
    };
    let active = scrollback.active_generation(session)?;
    let was = active
        .as_ref()
        .and_then(|held| serde_json::from_str::<Working>(&held.content).ok());
    if let Err(why) = proposed.may_replace(was.as_ref()) {
        balthasar_model::noted!("working: {} was refused: {why:?}", job.id);
        return Ok(false);
    }
    let (Some(from), Some(to)) = (job.context["from"].as_u64(), job.context["to"].as_u64()) else {
        return Ok(false);
    };
    let held = Generation {
        id: format!("g-{}", job.id),
        session: session.clone(),
        state: Ready::Building,
        parent: job.context["parent"].as_str().map(str::to_owned),
        covers: (from, to),
        policy: job.context["policy"].clone(),
        provenance: json!({ "version": 1, "job": job.id, "from": from, "to": to,
            "withheld": job.context["withheld"].clone() }),
        content: serde_json::to_string(&proposed).unwrap_or_default(),
        made: at.now,
    };
    scrollback.put_generation(&held)?;
    // Ready only once the span it replaces has been read back. Queuing an extraction before a cut
    // is a promise to read those rows, not a reading of them, so it cannot be what promotes this.
    if observed(scrollback, session, to)? {
        scrollback.ready_generation(&held.id)?;
        balthasar_model::noted!("working: {} is ready over rows {from}\u{2013}{to}", held.id);
    } else {
        balthasar_model::noted!("working: {} waits on coverage of row {to}", held.id);
    }
    Ok(true)
}

/// Whether extraction has acknowledged every row through `to`.
fn observed(
    scrollback: &balthasar_store::Transcript,
    session: &SessionId,
    to: u64,
) -> Result<bool, StoreError> {
    Ok(scrollback
        .extraction_progress(session)?
        .through
        .is_some_and(|through| through >= to))
}

/// What a turn boundary does with what has been prepared, using no helper and no model.
///
/// Whatever was built ahead of its coverage is promoted once coverage reaches it, and the furthest
/// `Ready` generation that still fits the room now becomes active. A generation whose span was
/// amended since, or that was prepared against more room than the request has, is left alone:
/// [`balthasar_store::Transcript::activate_generation`] says so rather than this.
pub(crate) fn at_boundary(
    at: &Answering<'_>,
    session: &SessionId,
    limit_now: Option<u64>,
) -> Result<Option<String>, StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(None);
    };
    let mut held = scrollback.generations_of(session)?;
    for made in &held {
        if made.state == Ready::Building && observed(scrollback, session, made.covers.1)? {
            scrollback.ready_generation(&made.id)?;
        }
    }
    held = scrollback.generations_of(session)?;
    let amended = amended_through(scrollback, session)?;
    let active = held.iter().find(|made| made.state == Ready::Active);
    let reached = active.map_or(0, |made| made.covers.1);
    let mut candidates: Vec<&Generation> = held
        .iter()
        .filter(|made| made.state == Ready::Ready && made.covers.1 > reached)
        .collect();
    candidates.sort_by_key(|made| made.covers.1);
    for made in candidates.into_iter().rev() {
        match scrollback.activate_generation(&made.id, amended, limit_now)? {
            Ok(()) => {
                balthasar_model::noted!("working: {} is active", made.id);
                return Ok(Some(made.id.clone()));
            }
            Err(why) => balthasar_model::noted!("working: {} was not activated: {why:?}", made.id),
        }
    }
    Ok(None)
}

/// What the summary slot carries: the active working state where there is one, and the
/// summariser's own summary otherwise.
///
/// Projected here rather than kept as a second slot, so it is budgeted, counted and stubbed by
/// exactly the rules the summary already goes through. A state that renders to nothing, or that
/// reaches no further than the summary it would replace, leaves the summary alone.
pub(crate) fn projection(
    scrollback: &balthasar_store::Transcript,
    session: &SessionId,
    summary: Option<Summary>,
) -> Result<Option<Summary>, StoreError> {
    let Some(active) = scrollback.active_generation(session)? else {
        return Ok(summary);
    };
    let reaches = summary.as_ref().is_none_or(|was| active.covers.1 >= was.to);
    let Some(state) = serde_json::from_str::<Working>(&active.content)
        .ok()
        .filter(|_| reaches)
    else {
        return Ok(summary);
    };
    let text = state.rendered();
    if text.trim().is_empty() {
        return Ok(summary);
    }
    Ok(Some(Summary {
        from: active.covers.0,
        to: active.covers.1,
        text,
        at: active.made,
    }))
}

/// The furthest row that has been revised since it was first written.
fn amended_through(
    scrollback: &balthasar_store::Transcript,
    session: &SessionId,
) -> Result<Option<u64>, StoreError> {
    Ok(scrollback
        .outline(session)?
        .iter()
        .filter(|turn| turn.revisions > 0)
        .map(|turn| turn.cursor)
        .next_back())
}

#[cfg(test)]
#[path = "preparing/tests.rs"]
mod tests;
