//! Claims that cannot both be true: found by a model, weighed by the evidence.
//!
//! The model names pairs of memories that already exist, by id: it cannot write a claim, change
//! one, pin one, or say how sure anything is. The force an edge carries is the contradicting
//! memory's own confidence out of its own witnesses, so the model supplies evidence, never a
//! verdict.

use crate::Answering;
use crate::queue::{can_run, json_in, pending};
use balthasar_model::{LinkRelation, Memory, MemoryId, SessionId};
use balthasar_store::StoreError;
use serde_json::{Value, json};

/// What the model is asked. Narrow on purpose: an unsure pair costs more than a missed one.
const INSTRUCTION: &str = "You are given claims a project believes, each with an id. Name only \
the pairs that cannot both be true at the same time about the same thing: one says the value is \
X and the other says it is Y, or one says a thing is so and the other says it is not.\n\
Not a contradiction: claims about different things; a general claim and a specific case of it; \
two steps of one process; things that merely sound similar; a claim and a fuller version of the \
same claim that agrees with it.\n\
When unsure, leave the pair out. A wrong pair takes confidence away from something true.\n\
Answer with JSON alone: {\"pairs\": [{\"a\": \"<id>\", \"b\": \"<id>\", \"why\": \"one line, \
naming what each one claims\"}]}. An empty list is the right answer when nothing clashes.";

/// How many claims are offered at once, and how much of each.
const CLAIMS: usize = 60;
const OF_EACH: usize = 200;
/// Below this many live claims there is not enough to disagree about.
const ENOUGH: usize = 4;
/// Edges taken from one answer. A model naming half the project has misunderstood the question.
const AT_MOST: usize = 12;
/// Where the claim count at the last sweep is kept; how many new ones are wanted is
/// `balthasar.memory`'s.
const OVER: &str = "clash_over";

/// The project's own counters, kept under the empty session.
fn project() -> SessionId {
    SessionId::new("")
}

/// Queue a sweep when one is due, there is enough to sweep, and the set has changed since the
/// last.
pub(crate) fn queue(
    at: &Answering<'_>,
    session: &SessionId,
    helpers: &[String],
    every: u32,
    hooks: &mut dyn crate::Hooks,
) -> Result<(), StoreError> {
    let Some(scrollback) = at.scrollback.as_ref() else {
        return Ok(());
    };
    if !can_run(helpers, "contradict") {
        return Ok(());
    }
    if pending(&scrollback.jobs_of(session)?, "contradict", at.now) {
        return Ok(());
    }
    let claims = claims(at, hooks);
    if claims.len() < ENOUGH {
        return Ok(());
    }
    let held = claims.len() as u64;
    let since = crate::cadence::since(scrollback, &project(), OVER, held, at.now)?;
    if since.progress == 0 || !crate::cadence::due(since, every) {
        return Ok(());
    }
    let mut input = String::from("The claims:\n");
    for (id, text) in &claims {
        input.push_str(&format!("- {id}: {text}\n"));
    }
    let spec = json!({
        "kind": "contradict", "role": "contradict", "fallback": "skip",
        "instruction": INSTRUCTION, "input": input,
        // No reasoning: allowed to, it fills any budget — 999 of 1000, 3120 of 3000 — and
        // answers nothing either time.
        "max_tokens": 1_000,
        "blocking": false, "timeout_ms": 60_000,
        "schema": { "type": "object", "required": ["pairs"], "properties": {
            "pairs": { "type": "array", "items": { "type": "object",
                "required": ["a", "b", "why"], "properties": {
                    "a": { "type": "string" }, "b": { "type": "string" },
                    "why": { "type": "string" } } } } } },
    });
    scrollback.queue_job(session, "contradict", &spec, &json!({}), at.now)?;
    crate::cadence::mark(scrollback, &project(), OVER, held, at.now)
}

/// The live claims of this project, newest first, as the model is shown them. Each passes the
/// same redaction a recall or a supply passes; one withheld is left out rather than named.
fn claims(at: &Answering<'_>, hooks: &mut dyn crate::Hooks) -> Vec<(String, String)> {
    let mut found: Vec<Memory> = at
        .store
        .all()
        .unwrap_or_default()
        .into_iter()
        .filter(|memory| memory.scope == at.scope && holds(memory, at))
        .collect();
    found.sort_by(|a, b| b.id.as_str().cmp(a.id.as_str()));
    found
        .into_iter()
        .filter_map(|memory| {
            let said = hooks.redact(&memory.text(), &memory)?;
            let text: String = said.chars().take(OF_EACH).collect();
            (!text.trim().is_empty()).then(|| (memory.id.to_string(), text))
        })
        .take(CLAIMS)
        .collect()
}

/// An edge for every pair naming two live claims of this project, and nothing for the rest. Both
/// ways round: a disagreement with no way of telling which side is right discredits each.
pub(crate) fn settle(at: &mut Answering<'_>, text: &str) -> Result<bool, StoreError> {
    let Some(said) = json_in(text) else {
        return Ok(false);
    };
    let pairs = said["pairs"].as_array().cloned().unwrap_or_default();
    let named = |pair: &Value, side: &str| pair[side].as_str().map(MemoryId::new);
    let mut touched: Vec<MemoryId> = Vec::new();
    let mut linked = 0;
    for pair in pairs.iter().take(AT_MOST) {
        let (Some(a), Some(b)) = (named(pair, "a"), named(pair, "b")) else {
            continue;
        };
        if a == b || !live(at, &a)? || !live(at, &b)? || reconciled(at, &a, &b)? {
            continue;
        }
        at.store.link(&a, &b, LinkRelation::Contradicts, at.now)?;
        at.store.link(&b, &a, LinkRelation::Contradicts, at.now)?;
        for id in [a, b] {
            if !touched.contains(&id) {
                touched.push(id);
            }
        }
        linked += 1;
    }
    let rounds = settled(at, &touched)?;
    balthasar_model::noted!(
        "contradict: {linked} of {} pairs linked, settled in {rounds}",
        pairs.len()
    );
    Ok(true)
}

/// Rescore what the new edges touched until the numbers stop moving. Each one's confidence is a
/// function of the others', so one pass leaves whichever was read first scored against a figure
/// that has since changed; it converges because the base comes from the witnesses.
fn settled(at: &mut Answering<'_>, touched: &[MemoryId]) -> Result<usize, StoreError> {
    const ROUNDS: usize = 12;
    const STILL: f64 = 1e-9;
    for round in 1..=ROUNDS {
        let mut moved: f64 = 0.0;
        for id in touched {
            let was = at.store.get(id)?.map_or(0.0, |memory| memory.confidence);
            let now = at.store.rescore(id, at.now)?;
            moved = moved.max((now - was).abs());
        }
        if moved < STILL {
            return Ok(round);
        }
    }
    Ok(ROUNDS)
}

/// Whether a person has settled this pair already. Their word stands, silently.
fn reconciled(at: &Answering<'_>, a: &MemoryId, b: &MemoryId) -> Result<bool, StoreError> {
    Ok(at.store.get(a)?.is_some_and(|memory| {
        memory
            .links
            .iter()
            .any(|link| link.rel == LinkRelation::Reconciled && &link.to == b)
    }))
}

/// A model naming something archived, superseded, another project's, or invented gets nothing.
fn live(at: &Answering<'_>, id: &MemoryId) -> Result<bool, StoreError> {
    Ok(at
        .store
        .get(id)?
        .is_some_and(|memory| memory.scope == at.scope && holds(&memory, at)))
}

/// Live enough to be argued about: still here, still current, and not already spent.
fn holds(memory: &Memory, at: &Answering<'_>) -> bool {
    memory.archived_at.is_none() && memory.temporal.is_live() && memory.confidence >= at.live_floor
}

#[cfg(test)]
mod tests;
