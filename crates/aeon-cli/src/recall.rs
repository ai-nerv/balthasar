//! `aeon recall` — search, and say why each answer is here.

use crate::Which;
use crate::{now, open, render};
use aeon_lua::Floors;
use aeon_model::{ScopeId, Tier};
use aeon_store::{Recall, Scored, Store};
use clap::Parser;
use std::path::Path;

/// Search.
#[derive(Debug, Parser)]
pub struct Args {
    /// What to look for. Nothing means "whatever is most worth showing".
    query: Vec<String>,

    /// How many.
    #[arg(long, short, default_value_t = 10)]
    limit: usize,

    /// Only this tier.
    #[arg(long)]
    tier: Option<String>,

    /// Look in the archive as well.
    #[arg(long)]
    archived: bool,

    /// Score against a remote model's boundary, so local-only memories are withheld.
    #[arg(long)]
    remote: bool,

    /// Also compute this policy's answer, and report how it differed.
    ///
    /// The shadow's results are discarded. Nobody is served an experiment, and the data to judge
    /// one accumulates anyway — which is the only arrangement in which a retrieval change can be
    /// evaluated on real queries without anybody being the subject of it.
    #[arg(long, value_name = "POLICY")]
    shadow: Option<String>,

    /// Use the control policy: full-text search, no vectors, no traversal.
    ///
    /// The permanent floor, and what every retrieval experiment is measured against.
    #[arg(long)]
    lexical_only: bool,

    /// Show the score breakdown.
    #[arg(long)]
    explain: bool,

    /// One JSON object per result.
    #[arg(long)]
    json: bool,
}

/// Search this scope, and the global store underneath it.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
    floors: Floors,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let embedding = loaded.embed_query(&args.query.join(" "));
    let weights = crate::weights_of(loaded.settings(), embedding.is_some());
    let at = now();
    let tiers = match &args.tier {
        Some(name) => vec![
            name.parse::<Tier>()
                .map_err(|_| anyhow::anyhow!("'{name}' is not a tier aeon knows"))?,
        ],
        None => Vec::new(),
    };

    let mut ask = Recall::of(args.query.join(" "), at);
    ask.floor = floors.live;
    ask.weights = weights;
    ask.embedding = embedding;
    ask.scope_name = scope.to_string();
    ask.limit = args.limit;
    ask.tiers = tiers;
    ask.include_archived = args.archived;
    ask.remote = args.remote;

    let started = std::time::Instant::now();
    let mut found = Vec::new();
    // Which tool each answer came from, so a result set spanning several can say so. Kept here
    // rather than on the memory: which store a memory is in is a fact about this search, not
    // about the memory.
    let mut from: std::collections::HashMap<String, aeon_store::Tool> =
        std::collections::HashMap::new();
    // The project first, and it says so: a project answer outranks a global one in the same
    // slot, which is what makes "we deploy to fly" and "I always use make" both sayable.
    for (store, near, named) in stores(store_path, scope, tool)? {
        ask.near = near;
        // Deliberately without reinforcing. Reading your own memory is not the same as an
        // agent needing it, and counting it as such meant nothing ever faded.
        for hit in store.recall(&ask)? {
            from.insert(hit.memory.id.to_string(), named.clone());
            found.push(hit);
        }
    }
    // Which retrieval behaviour this query gets. The policy decides where candidates come from
    // and how far to walk; it decides nothing about what may be asserted or injected, because
    // those are constraints rather than choices and sit outside every policy.
    let shape = aeon_recall::shape_of(&ask.query);
    let policy = if args.lexical_only {
        aeon_recall::Policy::lexical_only()
    } else {
        let held = aeon_recall::Policy::for_shape(shape);
        // A missing embedder degrades the policy rather than swapping it, so a store with no
        // vectors behaves like the same strategy doing less — not like a different one.
        if ask.embedding.is_some() {
            held
        } else {
            held.without_vectors()
        }
    };

    let mut paths: std::collections::HashMap<String, aeon_model::Relation> =
        std::collections::HashMap::new();
    if !policy.families.is_empty() {
        let seeds: Vec<aeon_model::MemoryId> =
            found.iter().take(5).map(|h| h.memory.id.clone()).collect();
        for (store, _, named) in stores(store_path, scope, tool)? {
            for (hit, edge) in reached(&store, &seeds, &policy, &ask, at)? {
                if found.iter().any(|h| h.memory.id == hit.memory.id) {
                    continue;
                }
                paths.insert(hit.memory.id.to_string(), edge);
                from.insert(hit.memory.id.to_string(), named.clone());
                found.push(hit);
            }
        }
    }

    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    found.truncate(args.limit);
    let latency_us = started.elapsed().as_micros() as u64;

    // Computed after the answer and thrown away. The comparison is kept; the candidates are
    // not, and nothing here can reach the results above.
    let shadowed = match &args.shadow {
        Some(name) => shadow(store_path, scope, tool, name, &ask, &found, at)?,
        None => None,
    };

    // After the answer, never before it. The ledger is instrumentation: it records that a
    // search happened and what it weighed, and there is no path from here back into the result.
    let traced = if loaded.settings().ledger().capture && store_path.is_none() {
        let mut into = open(None, scope, tool)?;
        Some(capture(
            &mut into,
            &Capturing {
                scope,
                ask: &ask,
                found: &found,
                at,
                latency_us,
                vectors: ask.embedding.is_some(),
                fingerprint: loaded.settings().fingerprint(),
            },
        )?)
    } else {
        None
    };

    // Evidence is fetched for the results that survived, not for every candidate: a search
    // over a thousand memories should not read four thousand witnesses nobody will see.
    if args.explain || args.json {
        for (store, _, _) in stores(store_path, scope, tool)? {
            for hit in &mut found {
                if let Ok(witnesses) = store.witnesses_of(&hit.memory.id)
                    && !witnesses.is_empty()
                {
                    hit.memory.witnesses = witnesses;
                }
            }
        }
    }

    loaded.tell(
        "recall",
        &[serde_json::json!(ask.query), serde_json::json!(found.len())],
    );

    if args.json {
        for hit in &found {
            crate::say!("{}", serde_json::to_string(&hit.memory)?);
        }
        return Ok(());
    }

    // The handle that makes this search answerable later. Printed rather than logged, because
    // an explanation id a caller has to reconstruct from a log file is one nobody uses.
    if let Some(id) = &traced {
        crate::say!("{}", render::dim(&format!("recall {id}")));
    }

    // Every policy decision appears in `--explain`. A retrieval that behaved differently and
    // could not say why is one nobody can debug and nobody can compare against another.
    if args.explain {
        crate::say!(
            "{}",
            render::dim(&format!(
                "policy {} · {} · {}",
                policy.name,
                if policy.families.is_empty() {
                    "no traversal".to_owned()
                } else {
                    format!(
                        "walks {}",
                        policy
                            .families
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
                policy.because
            ))
        );
    }

    if let Some(held) = &shadowed {
        crate::say!(
            "{}",
            render::dim(&format!(
                "shadow {} · {:.0}% overlap · {} result(s) · {} token(s) · {:.1}ms{}",
                held.shadowed,
                held.overlap * 100.0,
                held.returned,
                held.tokens,
                held.micros as f64 / 1000.0,
                if held.differs() {
                    ""
                } else {
                    " · would have changed nothing"
                }
            ))
        );
    }

    if found.is_empty() {
        crate::say!("{}", render::dim("nothing yet"));
        return Ok(());
    }
    let project = project_name(scope);
    // Session names are resolved once for the whole result set rather than per line: a page of
    // results is a handful of distinct sessions, and one lookup each would be a query per row.
    let names = session_names(store_path, scope, tool)?;
    // Only worth saying when the answers came from more than one place. On a single-tool
    // project every line would carry the same word, which is noise rather than provenance.
    let spans_tools = {
        let mut seen: Vec<&aeon_store::Tool> = found
            .iter()
            .filter_map(|hit| from.get(&hit.memory.id.to_string()))
            .collect();
        seen.sort();
        seen.dedup();
        seen.len() > 1
    };
    for hit in &found {
        crate::say!("{}", render::line(&hit.memory, floors.inject, at));
        let named = hit
            .memory
            .session
            .as_ref()
            .and_then(|id| names.get(id.as_str()))
            .map(String::as_str);
        let whose = if spans_tools {
            from.get(&hit.memory.id.to_string())
                .map(|t| format!(" · {t}"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        crate::say!(
            "     {}",
            render::dim(&format!(
                "{}{whose}",
                render::origin(&hit.memory, project.as_deref(), named)
            ))
        );
        if let Some(edge) = paths.get(&hit.memory.id.to_string()) {
            crate::say!(
                "     {}",
                render::dim(&format!("reached by {}", edge.explain()))
            );
        }
        // §10.10: security labels appear in recall explanations. A result whose evidence is
        // all external content looks exactly like one a person typed, unless the line says so.
        if args.explain && !hit.memory.witnesses.is_empty() {
            let ceiling = hit
                .memory
                .witnesses
                .iter()
                .map(|w| w.channel.ceiling())
                .min()
                .unwrap_or(aeon_model::Presentation::Asserted);
            let sources: std::collections::BTreeSet<String> = hit
                .memory
                .witnesses
                .iter()
                .map(aeon_model::Witness::domain_of)
                .collect();
            let runs: std::collections::BTreeSet<&str> = hit
                .memory
                .witnesses
                .iter()
                .map(|w| w.session.as_str())
                .collect();
            if ceiling != aeon_model::Presentation::Asserted || sources.len() < runs.len() {
                crate::say!(
                    "     {}",
                    render::dim(&format!(
                        "trust {} · {} source(s) across {} run(s)",
                        ceiling.as_str(),
                        sources.len(),
                        runs.len()
                    ))
                );
            }
        }
        if args.explain {
            crate::say!("     {}", render::dim(&breakdown(hit)));
            if let Some(why) = render::withheld(&hit.memory, floors.inject, at) {
                crate::say!("     {}", render::dim(&format!("not asserted: {why}")));
            }
        }
    }
    Ok(())
}

/// Record what this search did, when the configuration asked for it.
///
/// Off by default and after the fact by construction: the ledger is written once the answer is
/// already decided, so it cannot change what a search returns. F1's acceptance is that recall
/// behaves identically with capture disabled, and the shape of this function is the argument —
/// there is no path from here back into the result.
///
/// The query is hashed. A ledger that held what people search for would be a different and much
/// worse thing than one that holds that a search happened.
struct Capturing<'a> {
    scope: &'a ScopeId,
    ask: &'a Recall,
    found: &'a [Scored],
    at: aeon_model::Timestamp,
    latency_us: u64,
    vectors: bool,
    fingerprint: String,
}

fn capture(store: &mut Store, said: &Capturing<'_>) -> anyhow::Result<String> {
    let Capturing {
        scope,
        ask,
        found,
        at,
        latency_us,
        vectors,
        fingerprint,
    } = said;
    let (at, latency_us, vectors) = (*at, *latency_us, *vectors);
    let id = format!("recall-{at}-{}", &aeon_model::content_hash(&ask.query)[..8]);
    let candidates: Vec<aeon_store::Candidate> = found
        .iter()
        .enumerate()
        .map(|(rank, hit)| aeon_store::Candidate {
            memory: hit.memory.id.clone(),
            rank,
            selected: true,
            score: hit.score,
            signals: aeon_store::Signals {
                semantic: hit.semantic.unwrap_or(0.0),
                lexical: hit.lexical,
                entity: hit.entity,
                frecency: hit.frecency,
                confidence: hit.confidence,
                strength: hit.strength,
                scope: f64::from(u8::from(hit.near)),
            },
        })
        .collect();

    store.note_recall(
        &aeon_store::RecallRun {
            id: id.clone(),
            scope: (*scope).clone(),
            session: None,
            query_hash: aeon_model::content_hash(&ask.query)[..16].to_owned(),
            requested_at: at,
            config_fingerprint: fingerprint.clone(),
            vector_available: vectors,
            result_limit: ask.limit,
            latency_us,
        },
        &candidates,
    )?;
    Ok(id)
}

/// Candidates the relationship views reach that search alone did not.
///
/// The query's shape decides which families are walked; classification changes what is
/// *generated* and never what relevance means, so these arrive as ordinary candidates and are
/// scored by the same function as everything else. A shape with no families walks nothing and
/// costs nothing.
///
/// Returns each memory with the edge that reached it, so `--explain` can show the path rather
/// than asserting a relevance nobody can check.
fn reached(
    store: &Store,
    seeds: &[aeon_model::MemoryId],
    policy: &aeon_recall::Policy,
    ask: &Recall,
    at: aeon_model::Timestamp,
) -> anyhow::Result<Vec<(Scored, aeon_model::Relation)>> {
    if policy.families.is_empty() || seeds.is_empty() {
        return Ok(Vec::new());
    }
    let reach = aeon_store::Reach {
        hops: policy.hops,
        ..aeon_store::Reach::default()
    };
    let found = store.traverse(seeds, &policy.families, &reach)?;

    let mut out = Vec::new();
    for (id, edge) in found {
        let Some(memory) = store.get(&id)? else {
            continue;
        };
        // The same gates as any other candidate. A traversal must not be a way past the live
        // floor, the archive filter, or the privacy boundary.
        if memory.archived_at.is_some() && !(ask.include_archived || policy.archive) {
            continue;
        }
        if memory.strength.at(at) < ask.floor {
            continue;
        }
        out.push((
            Scored {
                score: edge.weight * 0.5,
                semantic: None,
                lexical: 0.0,
                entity: 0.0,
                frecency: 0.0,
                confidence: memory.confidence,
                strength: memory.strength.at(at),
                near: true,
                memory,
            },
            edge,
        ));
    }
    Ok(out)
}
/// Which stores answer a search: the scope asked for, and `global` underneath it unless it
/// already is `global`.
///
/// With no tool named, every tool in the project answers. A search is a question about what is
/// known here, not about what one program happened to file, and a person who has to remember
/// which tool wrote something in order to find it does not have a memory layer.
fn stores(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
) -> anyhow::Result<Vec<(Store, bool, aeon_store::Tool)>> {
    if store_path.is_some() {
        return Ok(vec![(
            open(store_path, scope, tool)?,
            true,
            tool.tool.clone(),
        )]);
    }
    let mut out = Vec::new();
    for named in searched(scope, tool) {
        let which = Which {
            tool: named.clone(),
            named: true,
        };
        if !scope.is_global() {
            out.push((open(None, scope, &which)?, true, named.clone()));
        }
        // A missing global store is not an error: it means nothing has been remembered
        // everywhere yet, which is the ordinary state of a fresh install. Not opened when it is
        // missing, either — searching several tools should not leave an empty file behind for
        // each one, for the same reason looking at a run does not create its directory.
        let everywhere = ScopeId::global();
        if aeon_store::scope_path(&everywhere, &named).is_file() {
            out.push((open(None, &everywhere, &which)?, scope.is_global(), named));
        }
    }
    Ok(out)
}

/// Which tools a search covers.
///
/// The one named, or every tool with memory in this project. A project nothing has written to
/// yet has no tools at all, and answering with none would make the first search after the first
/// `remember` find nothing — so the default stands in.
fn searched(scope: &ScopeId, tool: &Which) -> Vec<aeon_store::Tool> {
    if tool.named {
        return vec![tool.tool.clone()];
    }
    let present = aeon_store::tools_in(scope);
    if present.is_empty() {
        vec![tool.tool.clone()]
    } else {
        present
    }
}

/// Every session's id and the name it is printed under.
fn session_names(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    for (store, _, _) in stores(store_path, scope, tool)? {
        for session in store.sessions(usize::MAX)? {
            out.insert(session.id.to_string(), session.name);
        }
    }
    Ok(out)
}

/// The project's short name, for saying which store a result came from.
fn project_name(scope: &ScopeId) -> Option<String> {
    if scope.is_global() {
        return None;
    }
    scope
        .as_str()
        .rsplit('/')
        .find(|part| !part.is_empty())
        .map(str::to_owned)
}

/// Where a result's score came from.
///
/// Every term, including the ones that contributed nothing. A breakdown that hid an absent
/// signal would leave somebody wondering whether it was zero or never consulted.
fn breakdown(hit: &Scored) -> String {
    let semantic = hit
        .semantic
        .map_or_else(|| "semantic —".to_owned(), |v| format!("semantic {v:.2}"));
    format!(
        "score {:.2} = {semantic} · lexical {:.2} · entity {:.2} · frecency {:.2} · \
         confidence {:.2} · strength {:.2}{}",
        hit.score,
        hit.lexical,
        hit.entity,
        hit.frecency,
        hit.confidence,
        hit.strength,
        if hit.near { " · project" } else { "" }
    )
}

/// Run a policy beside the real one, and keep only the comparison.
///
/// The shadow's candidates are computed and dropped. What survives is bounded — overlap, count,
/// token cost, latency — because storing both result sets would be a second copy of the store
/// keyed by query, which is a privacy problem wearing a research hat.
fn shadow(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    name: &str,
    ask: &Recall,
    served: &[Scored],
    at: aeon_model::Timestamp,
) -> anyhow::Result<Option<aeon_recall::Shadow>> {
    let policy = match name {
        "lexical-only" => aeon_recall::Policy::lexical_only(),
        "balanced" => aeon_recall::Policy::balanced(),
        other => {
            // Named by shape, so `--shadow temporal` means "what would the temporal policy have
            // done". An unknown name is refused rather than silently ignored: a shadow that did
            // not run would report perfect agreement, which is the most misleading answer.
            let shape = match other {
                "temporal" => aeon_recall::Shape::Temporal,
                "repair" | "causal" => aeon_recall::Shape::Causal,
                "entity" => aeon_recall::Shape::Entity,
                "similar" | "semantic" => aeon_recall::Shape::Semantic,
                "current-fact" | "current" => aeon_recall::Shape::Current,
                "procedure" | "procedural" => aeon_recall::Shape::Procedural,
                _ => anyhow::bail!(
                    "no policy called '{other}' — try balanced, lexical-only, temporal, \
                     repair, entity, similar, current-fact or procedure"
                ),
            };
            aeon_recall::Policy::for_shape(shape)
        }
    };

    let started = std::time::Instant::now();
    let mut candidates: Vec<Scored> = Vec::new();
    let mut ask = ask.clone();
    for (store, near, _) in stores(store_path, scope, tool)? {
        ask.near = near;
        candidates.extend(store.recall(&ask)?);
    }
    if !policy.families.is_empty() {
        let seeds: Vec<aeon_model::MemoryId> = candidates
            .iter()
            .take(5)
            .map(|h| h.memory.id.clone())
            .collect();
        for (store, _, _) in stores(store_path, scope, tool)? {
            for (hit, _) in reached(&store, &seeds, &policy, &ask, at)? {
                if !candidates.iter().any(|h| h.memory.id == hit.memory.id) {
                    candidates.push(hit);
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    candidates.truncate(ask.limit);
    let micros = started.elapsed().as_micros() as u64;

    let tokens: usize = candidates
        .iter()
        .map(|h| h.memory.text().len().div_ceil(4))
        .sum();
    let served_ids: Vec<String> = served.iter().map(|h| h.memory.id.to_string()).collect();
    let shadow_ids: Vec<String> = candidates.iter().map(|h| h.memory.id.to_string()).collect();

    Ok(Some(aeon_recall::Shadow::of(
        &aeon_recall::Policy::balanced(),
        &policy,
        &served_ids,
        &shadow_ids,
        tokens,
        micros,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeon_model::{Body, Memory, Tier as T};

    #[test]
    fn a_breakdown_names_every_signal_that_moved_the_score() {
        // `--explain` exists so a ranking can be argued with. One missing term and it cannot.
        let memory = Memory::new(
            aeon_model::MemoryId::new("x"),
            T::Fact,
            ScopeId::global(),
            Body::fact("a", "b", "c"),
            0,
        );
        let text = breakdown(&Scored {
            memory,
            score: 0.5,
            semantic: Some(0.7),
            lexical: 0.4,
            entity: 0.55,
            frecency: 0.3,
            confidence: 0.6,
            strength: 0.9,
            near: true,
        });
        for term in [
            "lexical",
            "confidence",
            "strength",
            "score",
            "frecency",
            "semantic",
        ] {
            assert!(text.contains(term), "{text} is missing {term}");
        }
    }
}
