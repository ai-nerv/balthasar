//! What a run is called, once it has been going long enough to say.
//!
//! A run's title is the first thing asked, which is the best that can be had for nothing and is
//! often no use at all: four runs opened with "hi" are four runs called "hi". This asks a model
//! what the run turned out to be about, every so often, and writes that instead.
//!
//! It never touches a run somebody has named themselves — [`Store::retitle_session`] refuses
//! that — so `:rename` is the end of the matter.

use crate::Answering;
use crate::queue::{can_run, json_in, pending};
use balthasar_model::SessionId;
use balthasar_store::StoreError;
use serde_json::json;

const INSTRUCTION: &str = "You are given the opening of a conversation between a person and a \
coding agent. Answer with a title for it: what the person is trying to get done, in three to six \
words, in their own vocabulary where they used any.\n\
No punctuation at the end, no quotes, no preamble, and never the word \"conversation\" or \
\"session\". Prefer the concrete thing over the general one: \"rename sessions from the model\" \
beats \"a feature request\".\n\
Answer with JSON alone: {\"title\": \"…\"}.";

/// Turns shown, and how much of each. The opening says what a run is for; the rest of it is how
/// that went, which is not what a title is.
const TURNS: usize = 12;
const OF_EACH: usize = 400;
/// Below this many turns there is nothing a title could say that the first prompt does not.
const ENOUGH: usize = 4;
/// Where the row count at the last retitle is kept, against this run rather than the project.
const ASKED: &str = "title_asked";

/// Queue a retitle when one is due and this run is still the model's to name.
pub(crate) fn queue(
    at: &Answering<'_>,
    session: &SessionId,
    helpers: &[String],
    every: u32,
) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    if !can_run(helpers, "summary") {
        return Ok(());
    }
    if pending(&scrollback.jobs_of(session)?, "retitle", at.now) {
        return Ok(());
    }
    // Nothing to do for a run a person has named: asked and thrown away is worse than not asked.
    if at.store.is_named(session)? {
        return Ok(());
    }
    let turns = scrollback.replay(session)?;
    if turns.len() < ENOUGH {
        return Ok(());
    }
    // What the run has said since the last title, not how often this was offered a turn.
    let grown = turns.len() as u64;
    let since = crate::cadence::since(scrollback, session, ASKED, grown, at.now)?;
    if !crate::cadence::due(since, every) {
        return Ok(());
    }
    let mut input = String::new();
    for turn in turns.iter().take(TURNS) {
        input.push_str(&crate::queue::line(turn, OF_EACH));
    }
    let spec = json!({
        "kind": "retitle", "role": "summary", "fallback": "skip",
        "instruction": INSTRUCTION, "input": input, "max_tokens": 200,
        "blocking": false, "timeout_ms": 30_000,
        "schema": { "type": "object", "required": ["title"], "properties": {
            "title": { "type": "string" } } },
    });
    scrollback.queue_job(session, "retitle", &spec, &json!({}), at.now)?;
    crate::cadence::mark(scrollback, session, ASKED, grown, at.now)
}

/// Take the title the model offered, unless a person has named the run since it was asked.
pub(crate) fn settle(
    at: &mut Answering<'_>,
    session: &SessionId,
    text: &str,
) -> Result<bool, StoreError> {
    let Some(said) = json_in(text) else {
        return Ok(false);
    };
    let Some(title) = said["title"]
        .as_str()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return Ok(false);
    };
    let took = at.store.retitle_session(session, title)?;
    balthasar_model::noted!(
        "retitle: {title:?} {}",
        if took { "taken" } else { "refused" }
    );
    Ok(true)
}

#[cfg(test)]
mod tests;
