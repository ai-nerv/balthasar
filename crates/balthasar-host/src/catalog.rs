//! The catalog: what this project has on record, one line per memory, so a model can tell
//! "nothing is held about this" from "the search missed it".

use crate::Answering;
use crate::supply::Standing;
use balthasar_model::{Memory, Presentation, offered};

/// How many memories one catalog considers.
const PAGE: usize = 120;

/// How much of a claim's first line a catalog line keeps, in characters.
const LINE: usize = 72;

const HEADING: &str =
    "What this project has on record, newest first. Ask `why` with a handle for its evidence:";
const WHOLE: &str = "That is everything on record.";
const MORE: &str = "More is on record than fits here; a subject missing from this list may still \
                    be held — use recall.";

/// A packed catalog.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Catalogued {
    pub text: String,
    pub ids: Vec<String>,
    pub tokens: u32,
}

/// The catalog within `tokens` of room, `per_token` characters to the token.
pub(crate) fn catalog(at: &Answering<'_>, tokens: u32, per_token: u32) -> Option<Catalogued> {
    let total = tokens as usize * per_token as usize;
    let found = at
        .store
        .catalogued(&at.scope.to_string(), at.inject_floor, PAGE + 1)
        .ok()?;
    let standing = Standing::of(at);
    let mut out = format!("{HEADING}\n");
    let mut ids = Vec::new();
    let mut cut = found.len() > PAGE;
    for hit in found.iter().take(PAGE) {
        let memory = hit.memory();
        if !offerable(at, &standing, memory) {
            continue;
        }
        let line = line_of(at, memory);
        if out.len() + line.len() + MORE.len() > total {
            cut = true;
            break;
        }
        out.push_str(&line);
        ids.push(memory.id.to_string());
    }
    if ids.is_empty() {
        return None;
    }
    out.push_str(if cut { MORE } else { WHOLE });
    let tokens = u32::try_from(out.len().div_ceil(per_token.max(1) as usize)).unwrap_or(u32::MAX);
    Some(Catalogued {
        text: out,
        ids,
        tokens,
    })
}

/// Whether a memory may be listed: asserted, on a channel that permits it, and not withheld.
fn offerable(at: &Answering<'_>, standing: &Standing, memory: &Memory) -> bool {
    if !memory.is_assertable(at.inject_floor, at.now, true)
        || standing.withholds(at, memory)
        || balthasar_model::looks_like_injection(&memory.text())
    {
        return false;
    }
    let witnesses = at.store.witnesses_of(&memory.id).unwrap_or_default();
    balthasar_store::presented(memory, &witnesses) == Presentation::Asserted
}

/// `- [handle] tier · "first line" · replaces [h] · disputed with [h]`, the claim JSON-quoted.
fn line_of(at: &Answering<'_>, memory: &Memory) -> String {
    let text = memory.text();
    let first = text.lines().next().unwrap_or_default().trim();
    let clipped: String = if first.chars().count() > LINE {
        let mut kept: String = first.chars().take(LINE).collect();
        kept.push('…');
        kept
    } else {
        first.to_owned()
    };
    let says = serde_json::to_string(&clipped).unwrap_or_default();
    let mut line = format!(
        "- [{}] {} · {says}",
        offered::handle(memory.id.as_str()),
        memory.tier
    );
    let (replaced, disputed) = at.store.standing_of(memory).unwrap_or_default();
    for (label, ids) in [("replaces", replaced), ("disputed with", disputed)] {
        if !ids.is_empty() {
            let named: Vec<String> = ids
                .iter()
                .map(|id| format!("[{}]", offered::handle(id.as_str())))
                .collect();
            line.push_str(&format!(" · {label} {}", named.join(", ")));
        }
    }
    line.push('\n');
    line
}
