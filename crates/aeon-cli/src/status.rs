//! `aeon` with nothing after it — what this scope remembers.

use crate::{now, open, render};
use aeon_lua::Floors;
use aeon_model::ScopeId;
use std::path::Path;

/// Say what is here.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    floors: Floors,
    loaded: &crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let at = now();
    let store = open(store_path, scope)?;

    crate::say!("{}", render::bold(scope.as_str()));
    crate::say!("{}", render::dim(&store.path().display().to_string()));
    crate::say!();

    let census = store.census()?;
    if census.is_empty() {
        crate::say!("{}", render::dim("nothing remembered yet"));
        crate::say!(
            "{}",
            render::dim("  aeon remember \"we run the tests with make test\"")
        );
        return Ok(());
    }

    let total: i64 = census.iter().map(|(_, n)| n).sum();
    for (tier, count) in &census {
        crate::say!("  {count:>5}  {tier}");
    }
    crate::say!("  {}", render::dim(&format!("{total:>5}  in all")));

    // Not a count but a claim about what the model would actually be told, because that is the
    // number a person is really asking about.
    let asserted = store
        .recall(&aeon_store::Recall {
            query: String::new(),
            limit: usize::MAX,
            tiers: vec![aeon_model::Tier::Fact, aeon_model::Tier::Habit],
            floor: floors.inject,
            include_archived: false,
            remote: false,
            relevance: 0.0,
            now: at,
            scope_name: scope.to_string(),
            weights: aeon_store::Weights::default().without_vectors(),
            embedding: None,
            near: true,
            reinforce: false,
        })?
        .len();
    crate::say!();
    crate::say!(
        "{} {}",
        render::bold(&asserted.to_string()),
        render::dim("would be asserted to a model right now")
    );

    // What is actually reachable, said plainly. A memory layer that silently ran without an
    // embedder or a distiller would leave somebody wondering why recall felt blunt.
    crate::say!();
    crate::say!(
        "{}  {}",
        render::dim("embedder "),
        loaded.embedder().map_or_else(
            || "none — recall is lexical".to_owned(),
            |e| e.model().to_owned()
        )
    );
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
