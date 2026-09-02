//! `balthasar relate` — derive the relationship index, and say what it found.
//!
//! Rebuildable by design. Everything this writes can be thrown away and recomputed from the
//! memories that are still there, which is what makes changing a derivation a thing you can try
//! rather than a thing you have to get right first time.

use crate::{Which, now, open, render};
use balthasar_distil::{Thresholds, entities, overlap, temporal};
use balthasar_model::{Derivation, ScopeId};
use clap::Parser;
use std::path::Path;

/// Work out which memories are related.
#[derive(Debug, Parser)]
pub struct Args {
    /// Retire the previous derivation before writing this one.
    ///
    /// Retiring marks the old edges stale rather than removing them, so the two versions can be
    /// compared on the same store.
    #[arg(long)]
    rebuild: bool,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Derive, and report.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
) -> anyhow::Result<()> {
    let at = now();
    let mut store = open(store_path, scope, tool)?;
    let rules = Thresholds::default();

    if args.rebuild {
        store.retire_relations(Derivation::Rule, balthasar_distil::RELATION_DERIVATION, at)?;
        store.retire_relations(
            Derivation::Structure,
            balthasar_distil::RELATION_DERIVATION,
            at,
        )?;
    }

    let held = store.all()?;
    let mut edges = temporal(&held, &rules, at);

    // Rarity comes from the entity index the store already keeps, so a term everything mentions
    // is discounted without this command having to count anything itself.
    let named: Vec<(balthasar_model::MemoryId, Vec<String>)> = held
        .iter()
        .map(|m| {
            let names = balthasar_store::entities_in(&m.text())
                .into_iter()
                .map(|e| e.name)
                .collect();
            (m.id.clone(), names)
        })
        .collect();
    let counts = store.entity_counts(scope.as_str())?;
    let rarity = |name: &str| balthasar_store::rarity(counts.get(name).copied().unwrap_or(0));
    edges.extend(entities(&named, &rarity, &rules, at));

    let texts: Vec<(balthasar_model::MemoryId, String)> =
        held.iter().map(|m| (m.id.clone(), m.text())).collect();
    edges.extend(overlap(&texts, &rules, at));

    let written = store.relate(&edges)?;
    let census = store.relation_census()?;

    if args.json {
        crate::say!(
            "{}",
            serde_json::json!({
                "memories": held.len(),
                "written": written,
                "kinds": census.iter().map(|(view, n)| {
                    serde_json::json!({ "kind": view.as_str(), "count": n })
                }).collect::<Vec<_>>(),
            })
        );
        return Ok(());
    }

    crate::say!(
        "{} {}",
        render::bold(&written.to_string()),
        render::dim(&format!("edge(s) over {} memories", held.len()))
    );
    if census.is_empty() {
        crate::say!("{}", render::dim("nothing was related"));
        return Ok(());
    }
    crate::say!();
    for (view, count) in &census {
        crate::say!(
            "  {:>5}  {}",
            count,
            render::dim(&format!("{view}  ({})", view.family()))
        );
    }
    crate::say!();
    crate::say!(
        "{}",
        render::dim("every edge is derived and disposable — `--rebuild` retires and recomputes")
    );
    Ok(())
}
