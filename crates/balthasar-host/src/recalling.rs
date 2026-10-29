//! `recall`: what a search finds, as rows a harness can show or inject.

use crate::saying::{describe, session_names};
use crate::{Answering, Door};
use balthasar_ipc::{Reply, Request};
use balthasar_model::SessionId;
use balthasar_store::Recall;

/// Search.
pub(crate) fn recall(at: &mut Answering<'_>, door: &Door, request: &Request) -> Reply {
    let query = request.args.first().and_then(|v| v.as_str()).unwrap_or("");
    let opts = request.args.get(1);
    let limit = opts
        .and_then(|o| o.get("limit"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(10)
        .min(100) as usize;

    let mut ask = Recall::of(query, at.now);
    ask.limit = limit;
    ask.floor = at.live_floor;
    ask.near = true;
    // A peer asking for memory is about to send it somewhere, so the remote boundary is assumed.
    ask.remote = opts
        .and_then(|o| o.get("remote"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);

    // Closed memories are the owner's to browse; anything a peer asks for may reach a prompt.
    ask.history = matches!(door, Door::Owner)
        && opts
            .and_then(|o| o.get("history"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
    let search = |store: &balthasar_store::Store| {
        if ask.history {
            store.recall(&ask)
        } else {
            store.recall_current(&ask).map(|found| {
                found
                    .into_iter()
                    .map(balthasar_store::Current::into_scored)
                    .collect()
            })
        }
    };
    let mut found = match search(at.store) {
        Ok(found) => found,
        Err(why) => return Reply::refused(why.to_string()),
    };
    // A run searches its own scratch too, which now lives in its own file.
    if let Some(run) = opts
        .and_then(|o| o.get("session"))
        .and_then(serde_json::Value::as_str)
        .map(SessionId::new)
    {
        // This agent's own scratch and no sibling's.
        let agent = at.agent.clone();
        let own = match at.scratch.as_mut() {
            Some(pad) => pad.peek(&run, &agent).and_then(|held| match held {
                Some(store) => search(store),
                None => Ok(Vec::new()),
            }),
            None => Ok(Vec::new()),
        };
        match own {
            Ok(mine) => found.extend(mine),
            Err(why) => return Reply::refused(why.to_string()),
        }
        found.sort_by(|a, b| b.score.total_cmp(&a.score));
        found.truncate(limit);
    }
    // The owner sees everything on record; a session is not shown a rule nobody stood behind.
    if !matches!(door, Door::Owner) {
        let standing = crate::supply::Standing::of(at);
        found.retain(|hit| !standing.withholds(at, &hit.memory));
    }
    // Names are resolved once for the whole result set.
    let names = session_names(at);
    let described: Vec<serde_json::Value> = found
        .iter()
        .map(|hit| describe(&hit.memory, at.inject_floor, at.now, &names))
        .collect();

    // Handing memories to a caller about to put them in a context is an injection.
    let injection = if at.capture {
        match note_served(at, query, &found, opts) {
            Ok(id) => Some(id),
            Err(why) => return Reply::refused(why.to_string()),
        }
    } else {
        None
    };

    // One row per memory. With a ledger on, the answer is instead one record: the search and
    // what it served, which is a single thing rather than a listing wrapped in a row.
    match injection {
        None => Reply::rows(described),
        Some(id) => Reply::one(serde_json::json!({
            "injection": id,
            "memories": described,
        })),
    }
}

/// Record that a search happened and that its results were handed over.
///
/// After the answer is decided, never before it: the ledger is instrumentation.
fn note_served(
    at: &mut Answering<'_>,
    query: &str,
    found: &[balthasar_store::Scored],
    opts: Option<&serde_json::Value>,
) -> Result<String, balthasar_store::StoreError> {
    let session = opts
        .and_then(|o| o.get("session"))
        .and_then(serde_json::Value::as_str)
        .map(SessionId::new);
    let query_hash = balthasar_model::content_hash(query)[..16].to_owned();
    // Two searches in one second, even for the same words, are two records.
    let unique =
        balthasar_model::content_hash(&format!("{query_hash}:{}", balthasar_store::mint(at.now)));
    let stamp = format!("{}-{}", at.now, &unique[..8]);

    at.store.note_recall(
        &balthasar_store::RecallRun {
            id: format!("recall-{stamp}"),
            scope: at.scope.clone(),
            session: session.clone(),
            query_hash,
            requested_at: at.now,
            config_fingerprint: String::new(),
            vector_available: false,
            result_limit: found.len(),
            latency_us: 0,
        },
        &found
            .iter()
            .enumerate()
            .map(|(rank, hit)| balthasar_store::Candidate {
                memory: hit.memory.id.clone(),
                rank,
                selected: true,
                score: hit.score,
                signals: balthasar_store::Signals {
                    semantic: hit.semantic.unwrap_or(0.0),
                    lexical: hit.lexical,
                    entity: hit.entity,
                    frecency: hit.frecency,
                    confidence: hit.confidence,
                    strength: hit.strength,
                    scope: f64::from(u8::from(hit.near)),
                },
            })
            .collect::<Vec<_>>(),
    )?;

    let injection = format!("inject-{stamp}");
    let tokens: usize = found
        .iter()
        .map(|h| h.memory.text().len().div_ceil(4))
        .sum();
    at.store.note_injection(
        &balthasar_store::Injection {
            id: injection.clone(),
            recall: Some(format!("recall-{stamp}")),
            session,
            created_at: at.now,
            token_count: tokens,
            remote: true,
            policy: "balanced".to_owned(),
        },
        &found
            .iter()
            .map(|hit| {
                let mode = if hit.memory.is_assertable(at.inject_floor, at.now, true) {
                    balthasar_model::Presentation::Asserted
                } else {
                    balthasar_model::Presentation::Evidence
                };
                (hit.memory.id.clone(), mode)
            })
            .collect::<Vec<_>>(),
    )?;
    Ok(injection)
}
