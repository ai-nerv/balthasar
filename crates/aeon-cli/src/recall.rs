//! `aeon recall` — search, and say why each answer is here.

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

    let mut found = Vec::new();
    // The project first, then what is true everywhere. A project answer shadows a global one,
    // which is what makes "we deploy to fly" and "I always use make" both sayable.
    // The project first, and it says so: a project answer outranks a global one in the same
    // slot, which is what makes "we deploy to fly" and "I always use make" both sayable.
    for (store, near) in stores(store_path, scope)? {
        ask.near = near;
        // Deliberately without reinforcing. Reading your own memory is not the same as an
        // agent needing it, and counting it as such meant nothing ever faded.
        found.extend(store.recall(&ask)?);
    }
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    found.truncate(args.limit);

    // Evidence is fetched for the results that survived, not for every candidate: a search
    // over a thousand memories should not read four thousand witnesses nobody will see.
    if args.explain || args.json {
        for (store, _) in stores(store_path, scope)? {
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

    if found.is_empty() {
        crate::say!("{}", render::dim("nothing yet"));
        return Ok(());
    }
    let project = project_name(scope);
    // Session names are resolved once for the whole result set rather than per line: a page of
    // results is a handful of distinct sessions, and one lookup each would be a query per row.
    let names = session_names(store_path, scope)?;
    for hit in &found {
        crate::say!("{}", render::line(&hit.memory, floors.inject, at));
        let named = hit
            .memory
            .session
            .as_ref()
            .and_then(|id| names.get(id.as_str()))
            .map(String::as_str);
        crate::say!(
            "     {}",
            render::dim(&render::origin(&hit.memory, project.as_deref(), named))
        );
        if args.explain {
            crate::say!("     {}", render::dim(&breakdown(hit)));
            if let Some(why) = render::withheld(&hit.memory, floors.inject, at) {
                crate::say!("     {}", render::dim(&format!("not asserted: {why}")));
            }
        }
    }
    Ok(())
}

/// Which stores answer a search: the scope asked for, and `global` underneath it unless it
/// already is `global`.
fn stores(store_path: Option<&Path>, scope: &ScopeId) -> anyhow::Result<Vec<(Store, bool)>> {
    if store_path.is_some() || scope.is_global() {
        return Ok(vec![(open(store_path, scope)?, true)]);
    }
    // A missing global store is not an error: it means nothing has been remembered
    // everywhere yet, which is the ordinary state of a fresh install.
    Ok(vec![
        (open(None, scope)?, true),
        (open(None, &ScopeId::global())?, false),
    ])
}

/// Every session's id and the name it is printed under.
fn session_names(
    store_path: Option<&Path>,
    scope: &ScopeId,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    let mut out = std::collections::HashMap::new();
    for (store, _) in stores(store_path, scope)? {
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
