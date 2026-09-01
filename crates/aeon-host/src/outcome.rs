//! The verbs that close the loop: what was used, how it went, and what followed from what.
//!
//! A caller that never reports anything is a supported caller. Every verb here is optional, and
//! recall behaves identically whether or not any of them is ever called — which is the only way
//! a measurement of utility can be honest, because a system that needed the reports would only
//! ever hear from the callers that agree with it.

use crate::{Answering, Door};
use aeon_ipc::{Reply, Request};
use aeon_model::{Attribution, MemoryId, OutcomeKind, SessionId};
use aeon_store::{Use, Verdict};

/// `used(injection, {memories, tool, action, attribution, session})`.
///
/// A caller reporting that it acted on something it was given. The memories it names are the
/// ones it says it followed; naming none is allowed and means the action had nothing to do with
/// what was injected, which is a useful thing to be able to say.
pub fn used(at: &mut Answering<'_>, door: &Door, request: &Request) -> Reply {
    let Some(injection) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("used needs an injection id");
    };
    let said = request.args.get(1);

    let memories: Vec<MemoryId> = said
        .and_then(|s| s.get("memories"))
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(MemoryId::new)
                .collect()
        })
        .unwrap_or_default();

    // A caller may not promote its own attribution above what it can support. Saying "I
    // followed this memory" is explicit; a peer claiming a structural match aeon did not
    // observe would be asserting an analysis rather than reporting an action.
    let asked: Attribution = said
        .and_then(|s| s.get("attribution"))
        .and_then(serde_json::Value::as_str)
        .and_then(|text| text.parse().ok())
        .unwrap_or(Attribution::Proximal);
    let attribution = match (asked, memories.is_empty()) {
        (_, true) => Attribution::Proximal,
        (Attribution::Structural, _) if !door.may_evaluate_as_user() => Attribution::Explicit,
        (held, _) => held,
    };

    // What aeon can see for itself. A caller reporting "I ran `make test`" after being handed
    // a memory that says to run `make test` is a structural match — and working it out here
    // rather than asking is the difference between an observation and a claim.
    let ran = text(said, "action").unwrap_or_default();
    let matched = if memories.is_empty() && !ran.trim().is_empty() {
        structural(at, injection, &ran)
    } else {
        Vec::new()
    };
    let (memories, attribution) = if matched.is_empty() {
        (memories, attribution)
    } else {
        (matched, Attribution::Structural)
    };

    let action = format!("use-{}-{}", at.now, short(injection));
    let held = Use {
        id: action.clone(),
        injection: Some(injection.to_owned()),
        session: text(said, "session").map(SessionId::new),
        reported_at: at.now,
        tool: text(said, "tool"),
        // Hashed here rather than trusted from the caller, so the ledger cannot be used to
        // smuggle a command line into a table that promises not to hold one.
        action_hash: aeon_model::content_hash(&ran)[..16].to_owned(),
        attribution,
        memories,
    };

    match at.store.note_use(&held) {
        Ok(()) => Reply::one(serde_json::json!({ "action": action })),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// `outcome(action, {kind, evaluator, cursor, score, note})`.
///
/// How it went. `unknown` is the default and a real answer: an action nobody evaluated must
/// never drift into being a failed one.
pub fn outcome(at: &mut Answering<'_>, door: &Door, request: &Request) -> Reply {
    let Some(action) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("outcome needs an action id");
    };
    let said = request.args.get(1);

    let kind: OutcomeKind = text(said, "kind")
        .and_then(|t| t.parse().ok())
        .unwrap_or(OutcomeKind::Unknown);

    // Who says so. A peer reports as itself; only the owner's door may record that the person
    // judged it, because those two carry different weight everywhere they are read.
    let asked = text(said, "evaluator").unwrap_or_else(|| "caller".to_owned());
    let evaluator = if asked == "user" && !door.may_evaluate_as_user() {
        match door.who() {
            Some(who) => who,
            None => "caller".to_owned(),
        }
    } else {
        asked
    };

    let held = Verdict {
        // Derived from the action, so a caller replaying its log updates one row rather than
        // accumulating agreement with itself.
        id: format!("{action}-outcome"),
        action: action.to_owned(),
        observed_at: at.now,
        kind,
        score: said
            .and_then(|s| s.get("score"))
            .and_then(serde_json::Value::as_f64),
        evidence_cursor: said
            .and_then(|s| s.get("cursor"))
            .and_then(serde_json::Value::as_u64),
        evaluator,
        // Bounded on purpose. This is a label, not somewhere to put a transcript.
        note: text(said, "note").map(|t| t.chars().take(200).collect()),
    };

    match at.store.note_outcome(&held) {
        Ok(()) => Reply::one(serde_json::json!({ "outcome": held.id, "kind": kind.as_str() })),
        Err(why) => Reply::refused(why.to_string()),
    }
}

/// `trace(recall)` — what a search considered, returned, and led to.
pub fn trace(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(recall) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("trace needs a recall id");
    };
    match at.store.trace_of(recall) {
        Ok(None) => Reply::refused(format!("no recall called '{recall}'")),
        Err(why) => Reply::refused(why.to_string()),
        Ok(Some(held)) => Reply::one(serde_json::json!({
            "recall": held.recall,
            "scope": held.scope.to_string(),
            "query_hash": held.query_hash,
            "requested_at": held.requested_at,
            "latency_us": held.latency_us,
            "considered": held.considered.iter().map(|(id, rank, selected, score)| {
                serde_json::json!({
                    "memory": id.to_string(),
                    "rank": rank,
                    "selected": selected,
                    "score": score,
                })
            }).collect::<Vec<_>>(),
            "actions": held.actions.iter().map(|a| serde_json::json!({
                "action": a.id,
                "tool": a.tool,
                "attribution": a.attribution.as_str(),
                "outcome": a.outcome.as_str(),
            })).collect::<Vec<_>>(),
        })),
    }
}

/// `utility(memory)` — what the ledger adds up to, beside how often it was merely retrieved.
pub fn utility(at: &mut Answering<'_>, request: &Request) -> Reply {
    let Some(id) = request.args.first().and_then(|v| v.as_str()) else {
        return Reply::refused("utility needs a memory id");
    };
    let id = MemoryId::new(id);
    let held = match at.store.utility_of(&id) {
        Ok(held) => held,
        Err(why) => return Reply::refused(why.to_string()),
    };
    let (considered, selected) = at.store.times_retrieved(&id).unwrap_or((0, 0));

    Reply::one(serde_json::json!({
        "memory": id.to_string(),
        "verified_helpful": held.verified_helpful,
        "verified_harmful": held.verified_harmful,
        "ignored": held.ignored,
        "unknown": held.unknown,
        "proximal": held.proximal,
        "last_verified_at": held.last_verified_at,
        "helpfulness": held.helpfulness(),
        // Beside, never folded in. The gap between how often something was retrieved and how
        // often it demonstrably helped is the number this whole milestone exists to expose.
        "times_considered": considered,
        "times_returned": selected,
    }))
}

/// A string field, when the caller sent one.
fn text(said: Option<&serde_json::Value>, name: &str) -> Option<String> {
    said.and_then(|s| s.get(name))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// Enough of an id to tell two apart in a generated name.
fn short(id: &str) -> String {
    id.chars().take(12).collect()
}

/// Which injected memories an action visibly followed.
///
/// The cheap, honest half of attribution. A memory that names `make test` and an action that
/// runs `make test` are related in a way aeon can check, so it checks rather than asking — which
/// matters because a caller claiming a structural match is asserting an analysis it did not
/// perform, and this is the analysis.
///
/// Deliberately strict. It looks for a distinctive run of the memory's own text inside the
/// action, not for shared words: "run the tests" and "make test" share a word and mean different
/// things, and a loose match here would attribute every outcome to everything.
fn structural(at: &mut Answering<'_>, injection: &str, action: &str) -> Vec<MemoryId> {
    let Ok(held) = at.store.injected_in(injection) else {
        return Vec::new();
    };
    let lowered = action.to_lowercase();

    held.into_iter()
        .filter(|id| {
            let Ok(Some(memory)) = at.store.get(id) else {
                return false;
            };
            // Backticked commands first: a memory that quotes a command is naming it exactly,
            // and that is the strongest signal available without a model.
            quoted(&memory.text())
                .iter()
                .any(|command| lowered.contains(&command.to_lowercase()))
        })
        .collect()
}

/// The commands a memory quotes.
fn quoted(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    let mut held = String::new();
    for c in text.chars() {
        if c == '`' {
            if inside && held.trim().len() > 2 {
                out.push(held.trim().to_owned());
            }
            held.clear();
            inside = !inside;
        } else if inside {
            held.push(c);
        }
    }
    out
}
