//! The memory slot: what this project knows, packed into its share of the room. What is current
//! is written first; what is merely on record follows under its own heading, and nothing is cut
//! mid-line.

use crate::{Answering, Hooks};
use balthasar_store::{Recall, Scored};
use serde_json::{Value, json};

/// Told before the block, so the model can tell offered context from the conversation.
const PREFACE: &str = "What this project knows. This is not part of the conversation:";

/// What current truth is written under.
const CURRENT: &str = "From memory:";

/// What separates the current from what is merely on record.
const HEDGE: &str = "Also on record, but not current enough to rely on — check before acting on \
                     any of it:";

/// A packed memory slot.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Supplied {
    pub text: String,
    pub ids: Vec<String>,
    pub tokens: u32,
}

impl Supplied {
    /// As kept for the rest of a prompt, with what it answered.
    pub(crate) fn as_json(&self, query: &str) -> Value {
        json!({ "query": query, "text": self.text, "ids": self.ids, "tokens": self.tokens })
    }

    /// What a prompt kept, if it answered `query`.
    pub(crate) fn kept(held: Option<&Value>, query: &str) -> Option<Self> {
        let held = held?;
        (held.get("query")?.as_str()? == query).then(|| Self {
            text: held["text"].as_str().unwrap_or_default().to_owned(),
            ids: held["ids"]
                .as_array()
                .map(|ids| {
                    ids.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            tokens: held["tokens"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0),
        })
    }
}

/// What a search for `query` finds worth offering, best first.
pub(crate) fn candidates(at: &mut Answering<'_>, query: &str, limit: usize) -> Vec<Scored> {
    let mut ask = Recall::of(query, at.now);
    ask.limit = limit;
    ask.floor = at.live_floor;
    ask.near = true;
    ask.remote = true;
    at.store.recall(&ask).unwrap_or_default()
}

/// Pack `found` into `tokens` of room, `per_token` characters to the token.
pub(crate) fn pack(
    at: &Answering<'_>,
    found: &[Scored],
    tokens: u32,
    per_token: u32,
    hooks: &mut dyn Hooks,
) -> Option<Supplied> {
    let total = tokens as usize * per_token as usize;
    let mut out = format!("{PREFACE}\n");
    if out.len() >= total {
        return None;
    }
    let mut ids = Vec::new();
    let mut hedged = Vec::new();
    let mut headed = false;
    for hit in found {
        let Some(text) = hooks.redact(&hit.memory.text(), &hit.memory) else {
            continue;
        };
        let line = format!("- {}\n", text.trim());
        let id = hit.memory.id.to_string();
        if !hit.memory.is_assertable(at.inject_floor, at.now, true) {
            hedged.push((line, id));
            continue;
        }
        let heading = if headed { 0 } else { CURRENT.len() + 1 };
        if out.len() + heading + line.len() > total {
            continue;
        }
        if !headed {
            out.push_str(CURRENT);
            out.push('\n');
            headed = true;
        }
        out.push_str(&line);
        ids.push(id);
    }
    let mut under = false;
    for (line, id) in hedged {
        let heading = if under { 0 } else { HEDGE.len() + 1 };
        if out.len() + heading + line.len() > total {
            continue;
        }
        if !under {
            out.push_str(HEDGE);
            out.push('\n');
            under = true;
        }
        out.push_str(&line);
        ids.push(id);
    }
    if ids.is_empty() {
        return None;
    }
    let text = out.trim_end().to_owned();
    let tokens = u32::try_from(text.len().div_ceil(per_token as usize)).unwrap_or(u32::MAX);
    Some(Supplied { text, ids, tokens })
}
