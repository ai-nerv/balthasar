//! `balthasar` with nothing after it — what this scope remembers.

use crate::Which;
use crate::{now, open, render};
use balthasar_lua::Floors;
use balthasar_model::ScopeId;
use std::path::Path;

/// Say what is here.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    floors: Floors,
    loaded: &crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let at = now();
    let store = open(store_path, scope, tool)?;
    let held = crate::scrollback(store_path, scope, tool).ok();
    let mut checks = balthasar_store::health(&store, held.as_ref())?;
    let said = loaded.log();
    checks.push(balthasar_store::Check {
        name: "config",
        state: if said.is_empty() {
            balthasar_store::HealthState::Ok
        } else {
            balthasar_store::HealthState::Cold
        },
        detail: said.first().cloned().unwrap_or_else(|| "loaded".to_owned()),
        fix: None,
    });

    crate::say!("{}", balthasar_store::health_summary(&checks));
    crate::say!("{}", render::bold(scope.as_str()));
    crate::say!("{}", render::dim(&store.path().display().to_string()));
    crate::say!();

    let census = store.census()?;
    if census.is_empty() {
        crate::say!("{}", render::dim("nothing remembered yet"));
        crate::say!(
            "{}",
            render::dim("  balthasar remember \"we run the tests with make test\"")
        );
        return Ok(());
    }

    let total: i64 = census.iter().map(|(_, n)| n).sum();
    for (tier, count) in &census {
        crate::say!("  {count:>5}  {tier}");
    }
    crate::say!("  {}", render::dim(&format!("{total:>5}  in all")));

    let mut ask = balthasar_store::Recall::of("", at);
    ask.limit = usize::MAX;
    ask.tiers = vec![balthasar_model::Tier::Fact, balthasar_model::Tier::Habit];
    ask.floor = floors.inject;
    ask.scope_name = scope.to_string();
    ask.near = true;
    let asserted = store.recall(&ask)?.len();
    crate::say!();
    crate::say!(
        "{} {}",
        render::bold(&asserted.to_string()),
        render::dim("would be asserted to a model right now")
    );

    crate::say!();
    crate::say!(
        "{}  {}",
        render::dim("embedder "),
        loaded.embedder().map_or_else(
            || "none — recall is lexical".to_owned(),
            |e| e.model().to_owned()
        )
    );
    if let Some(note) = loaded.embedder_note() {
        crate::say!("            {}", render::dim(note));
    }
    let (backends, unavailable) = loaded.distillers();
    let reachable: Vec<String> = backends
        .iter()
        .filter(|b| b.reachable())
        .map(|b| b.name())
        .collect();
    crate::say!(
        "{}  {}",
        render::dim("distiller"),
        if reachable.is_empty() {
            "none — extraction is by rule, which always works".to_owned()
        } else {
            reachable.join(", ")
        }
    );
    for line in unavailable {
        crate::say!("            {}", render::dim(&line));
    }
    Ok(())
}
