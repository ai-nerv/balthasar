//! `recall` and `context`: search rows and assembled context for a harness or peer.

use crate::saying::{describe, session_names};
use crate::{Answering, Door};
use balthasar_ipc::{Reply, Request};
use balthasar_model::SessionId;
use balthasar_store::{Recall, Scored};

/// Search.
pub(crate) fn recall(
    at: &mut Answering<'_>,
    door: &Door,
    request: &Request,
    hooks: &mut dyn crate::Hooks,
) -> Reply {
    let started = std::time::Instant::now();
    let query = request.args.first().and_then(|v| v.as_str()).unwrap_or("");
    let opts = request.args.get(1);
    let limit = opts
        .and_then(|o| o.get("limit"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(10)
        .min(100) as usize;

    let embedding = hooks.embed(query);
    let has_vectors = embedding.is_some();
    let budget = hooks.budget();
    let mut ask = Recall::of(query, at.now);
    ask.limit = limit;
    ask.floor = at.live_floor;
    ask.near = true;
    ask.scope_name = at.scope.to_string();
    ask.weights = hooks.weights(has_vectors);
    ask.embedding = embedding;
    ask.max_candidates = budget.candidates;
    ask.remote = opts
        .and_then(|o| o.get("remote"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
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
    let policy = {
        let held = balthasar_recall::Policy::for_shape(balthasar_recall::shape_of(query));
        if has_vectors {
            held
        } else {
            held.without_vectors()
        }
    };
    let hops = policy.hops.min(budget.hops);
    if !policy.families.is_empty() && hops > 0 && !found.is_empty() {
        let seeds: Vec<balthasar_model::MemoryId> =
            found.iter().take(5).map(|h| h.memory.id.clone()).collect();
        let reach = balthasar_store::Reach {
            hops,
            ..balthasar_store::Reach::default()
        };
        if let Ok(edges) = at.store.traverse(&seeds, &policy.families, &reach) {
            for (id, edge) in edges {
                if found.iter().any(|h| h.memory.id == id) {
                    continue;
                }
                let Ok(Some(memory)) = at.store.get(&id) else {
                    continue;
                };
                if !passes_edge(&memory, &ask, at.now) {
                    continue;
                }
                found.push(Scored {
                    score: edge.weight * 0.5,
                    semantic: None,
                    lexical: 0.0,
                    entity: 0.0,
                    frecency: 0.0,
                    confidence: memory.confidence,
                    strength: memory.strength.at_tier(memory.tier, at.now),
                    near: true,
                    memory,
                });
            }
        }
    }
    if let Some(run) = opts
        .and_then(|o| o.get("session"))
        .and_then(serde_json::Value::as_str)
        .map(SessionId::new)
    {
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
    }
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    if let Some(ranker) = hooks.ranker() {
        let store = &*at.store;
        ranker.rerank(&mut found, has_vectors, |id| evidence_counts(store, id));
    }
    found.truncate(limit);
    if !matches!(door, Door::Owner) {
        let standing = crate::supply::Standing::of(at);
        found.retain(|hit| !standing.withholds(at, &hit.memory));
    }
    let names = session_names(at);
    let mut redacted: Vec<Option<String>> = vec![None; found.len()];
    if !matches!(door, Door::Owner) {
        let mut kept = Vec::with_capacity(found.len());
        redacted.clear();
        for hit in found {
            if let Some(text) = hooks.redact(&hit.memory.text(), &hit.memory) {
                redacted.push(Some(text));
                kept.push(hit);
            }
        }
        found = kept;
    }
    let described: Vec<serde_json::Value> = found
        .iter()
        .zip(&redacted)
        .map(|(hit, text)| {
            let mut row = describe(&hit.memory, at.inject_floor, at.now, &names);
            if let (Some(text), Some(fields)) = (text, row.as_object_mut()) {
                fields.insert("text".to_owned(), serde_json::json!(text));
            }
            row
        })
        .collect();

    let latency_us = started.elapsed().as_micros() as u64;
    let injection = if at.capture {
        let meta = ServedMeta {
            query,
            fingerprint: hooks.fingerprint(),
            vectors: has_vectors,
            latency_us,
            remote: ask.remote,
            policy: policy.name,
        };
        match note_served(at, &meta, &found, opts) {
            Ok(id) => Some(id),
            Err(why) => return Reply::refused(why.to_string()),
        }
    } else {
        None
    };

    match injection {
        None => Reply::rows(described),
        Some(id) => Reply::one(serde_json::json!({
            "injection": id,
            "memories": described,
        })),
    }
}

/// Assemble sectioned context over the socket using the configured sections.
pub(crate) fn context(
    at: &mut Answering<'_>,
    door: &Door,
    request: &Request,
    hooks: &mut dyn crate::Hooks,
) -> Reply {
    let opts = request.args.first();
    let turn = opts
        .and_then(|o| o.get("turn").or_else(|| o.get("query")))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let tokens = opts
        .and_then(|o| o.get("budget").or_else(|| o.get("tokens")))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1000) as usize;
    let remote = opts
        .and_then(|o| o.get("remote"))
        .and_then(serde_json::Value::as_bool)
        .or_else(|| {
            opts.and_then(|o| o.get("for"))
                .and_then(serde_json::Value::as_str)
                .map(|w| w == "remote")
        })
        .unwrap_or(!matches!(door, Door::Owner));

    let sections = hooks.sections();
    if sections.is_empty() {
        return Reply::refused(
            "no sections are declared — run `balthasar configs` or declare `balthasar.section`",
        );
    }

    let embedding = hooks.embed(&turn);
    let ask = balthasar_recall::Ask {
        turn,
        tokens,
        bound: if remote {
            balthasar_recall::Bound::Remote
        } else {
            balthasar_recall::Bound::Local
        },
        floor: at.inject_floor,
        weights: hooks.weights(embedding.is_some()),
        now: at.now,
        scope: at.scope.to_string(),
        embedding,
    };

    let assembled =
        balthasar_recall::assemble_refs(&[(&*at.store, true)], &sections, &ask, |text, memory| {
            if remote {
                hooks.redact(text, memory)
            } else {
                Some(text.to_owned())
            }
        });
    match assembled {
        Ok(held) => Reply::one(serde_json::json!({
            "tokens": held.tokens,
            "deduplicated": held.deduplicated,
            "redacted": held.redacted,
            "text": held.text(),
            "sections": held.sections.iter().map(|s| serde_json::json!({
                "id": s.id,
                "lines": s.lines,
                "tokens": s.tokens,
            })).collect::<Vec<_>>(),
        })),
        Err(why) => Reply::refused(why.to_string()),
    }
}

fn passes_edge(
    memory: &balthasar_model::Memory,
    ask: &Recall,
    at: balthasar_model::Timestamp,
) -> bool {
    if memory.archived_at.is_some() || memory.tier == balthasar_model::Tier::Archive {
        return false;
    }
    if memory.temporal.valid_to.is_some() && !ask.history {
        return false;
    }
    memory.confidence >= ask.floor
        && memory.strength.at_tier(memory.tier, at) >= balthasar_model::floor::SPENT
        && memory.privacy.may_show(ask.remote, ask.history)
}

fn evidence_counts(
    store: &balthasar_store::Store,
    id: &balthasar_model::MemoryId,
) -> (usize, usize) {
    let Ok(witnesses) = store.witnesses_of(id) else {
        return (0, 0);
    };
    let domains: std::collections::BTreeSet<String> = witnesses
        .iter()
        .map(balthasar_model::Witness::domain_of)
        .collect();
    (witnesses.len(), domains.len())
}

struct ServedMeta<'a> {
    query: &'a str,
    fingerprint: String,
    vectors: bool,
    latency_us: u64,
    remote: bool,
    policy: &'static str,
}

fn note_served(
    at: &mut Answering<'_>,
    meta: &ServedMeta<'_>,
    found: &[Scored],
    opts: Option<&serde_json::Value>,
) -> Result<String, balthasar_store::StoreError> {
    let session = opts
        .and_then(|o| o.get("session"))
        .and_then(serde_json::Value::as_str)
        .map(SessionId::new);
    let query_hash = balthasar_model::content_hash(meta.query)[..16].to_owned();
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
            config_fingerprint: meta.fingerprint.clone(),
            vector_available: meta.vectors,
            result_limit: found.len(),
            latency_us: meta.latency_us,
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
            remote: meta.remote,
            policy: meta.policy.to_owned(),
        },
        &found
            .iter()
            .map(|hit| {
                let mode = if hit
                    .memory
                    .is_assertable(at.inject_floor, at.now, meta.remote)
                {
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
