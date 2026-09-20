//! Ending a disagreement, which only a person may end.
//!
//! [`crate::clashing`] finds claims that cannot both be true and makes each of them less
//! believed, which is right while nobody knows which is wrong. It is not an answer. These two
//! verbs are where the answer comes from.

use crate::saying::{describe, session_names};
use crate::{Answering, Door};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{Memory, MemoryId};

/// Every disagreement still open, newest first.
pub(crate) fn disagreements(at: &mut Answering<'_>) -> Reply {
    let scope = at.scope.to_string();
    let found = match at.store.disagreements(&scope) {
        Ok(found) => found,
        Err(why) => return Reply::refused(why.to_string()),
    };
    let names = session_names(at);
    let said = |memory: &Memory| describe(memory, at.inject_floor, at.now, &names);
    Reply::rows(
        found
            .iter()
            .map(|(a, b)| serde_json::json!({ "a": said(a), "b": said(b) }))
            .collect(),
    )
}

/// Which of two disagreeing claims is right, or that they do not disagree.
///
/// The person's word, so nothing here weighs it against anything: `keep` retires the other
/// outright, and `reconcile` overrides the edge for good.
pub(crate) fn settle(at: &mut Answering<'_>, door: &Door, request: &Request) -> Reply {
    let id = |at: usize| {
        request
            .args
            .get(at)
            .and_then(|v| v.as_str())
            .map(MemoryId::new)
    };
    let (Some(a), Some(b)) = (id(0), id(1)) else {
        return Reply::refused("settle needs the two memories that disagree");
    };
    if a == b {
        return Reply::refused("a memory does not disagree with itself");
    }
    if !door.may_settle() {
        return Reply::refused("settling a disagreement is the person's — ask the owner");
    }
    let said = request.args.get(2);
    let asked = |name: &str| {
        said.and_then(|s| s.get(name))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    // Both have to be here and open, or the answer would be about something else.
    for id in [&a, &b] {
        match at.store.get(id) {
            Ok(Some(memory)) if memory.scope == at.scope => {}
            Ok(Some(_)) => return Reply::refused(format!("'{id}' belongs to another project")),
            Ok(None) => return Reply::refused(format!("no memory called '{id}'")),
            Err(why) => return Reply::refused(why.to_string()),
        }
    }
    let keep = said.and_then(|s| s.get("keep")).and_then(|v| v.as_str());
    let done = match (keep, asked("reconcile"), asked("neither")) {
        (Some(kept), false, false) => {
            let kept = MemoryId::new(kept);
            let dropped = match (kept == a, kept == b) {
                (true, _) => b.clone(),
                (_, true) => a.clone(),
                _ => return Reply::refused("`keep` has to name one of the two"),
            };
            at.store
                .settle(&kept, &dropped, at.now)
                .map(|()| serde_json::json!({ "kept": kept.to_string(), "retired": dropped.to_string() }))
        }
        (None, true, false) => at
            .store
            .reconcile(&a, &b, at.now)
            .map(|()| serde_json::json!({ "reconciled": [a.to_string(), b.to_string()] })),
        (None, false, true) => at
            .store
            .archive(&a, at.now)
            .and_then(|()| at.store.archive(&b, at.now))
            .map(|()| serde_json::json!({ "retired": [a.to_string(), b.to_string()] })),
        _ => {
            return Reply::refused("settle takes exactly one of `keep`, `reconcile` or `neither`");
        }
    };
    match done {
        Ok(said) => Reply::one(said),
        Err(why) => Reply::refused(why.to_string()),
    }
}
