//! `aeon forget` — the two kinds of forgetting, which are not the same kind.
//!
//! Archiving keeps everything and stops asserting it. Purging removes it. The first is what
//! "I do not need this any more" means; the second is what "delete the key I pasted" means, and
//! conflating them would make one of those two sentences unanswerable.

use crate::Which;
use crate::{now, open, render};
use aeon_model::{MemoryId, ScopeId};
use aeon_store::Store;
use clap::Parser;
use std::io::Write;
use std::path::Path;

/// Stop asserting something, or remove it.
#[derive(Debug, Parser)]
pub struct Args {
    /// The memory, by id or by enough of one to be unambiguous.
    id: String,

    /// Remove it entirely, rather than archiving it.
    ///
    /// Irreversible, and the only thing in aeon that removes a row. Asks first unless
    /// `--yes` is given.
    #[arg(long)]
    purge: bool,

    /// Do not ask before purging.
    #[arg(long)]
    yes: bool,
}

/// Archive or purge.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let at = now();
    let mut store = open(store_path, scope, tool)?;
    let id = resolve(&store, &args.id)?;
    let memory = store
        .get(&id)?
        .ok_or_else(|| anyhow::anyhow!("no memory called {}", args.id))?;

    let described = serde_json::json!({
        "id": memory.id.to_string(),
        "text": memory.text(),
        "tier": memory.tier.as_str(),
    });

    if !args.purge {
        store.archive(&id, at)?;
        loaded.tell("forget", &[described, serde_json::json!("archived")]);
        crate::say!("archived {}", render::dim(&memory.text()));
        crate::say!(
            "     {}",
            render::dim("still in the store, still findable with --archived")
        );
        return Ok(());
    }

    if !args.yes && !confirmed(&memory.text())? {
        crate::say!("{}", render::dim("left alone"));
        return Ok(());
    }
    let gone = aeon_store::purge(&mut store, &id)?;
    anyhow::ensure!(gone == 1, "nothing was removed");
    loaded.tell("forget", &[described, serde_json::json!("purged")]);
    crate::say!("purged {}", render::dim(&memory.text()));
    crate::say!(
        "     {}",
        render::dim("gone, with its evidence and its edges")
    );
    Ok(())
}

/// Ask, and take silence for no.
///
/// Reading from a pipe answers "no" rather than "yes": a script that pipes into `aeon forget
/// --purge` without `--yes` has not consented to anything.
fn confirmed(what: &str) -> anyhow::Result<bool> {
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        anyhow::bail!("purging needs --yes when there is nobody to ask");
    }
    print!("purge \"{what}\"? this cannot be undone [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

/// A memory from as much of its id as somebody was willing to type.
///
/// A ULID is twenty-six characters and nobody types one. What is printed is the trailing
/// handle (see [`render::short`]), so that is what this matches first — and a pasted full id
/// or leading prefix works too, because somebody who copied one should not be told it is not
/// an id.
///
/// An ambiguous handle is an error rather than a guess. Acting on the wrong memory is worse
/// than being asked again, and `--purge` makes that difference permanent.
pub fn resolve(store: &Store, handle: &str) -> anyhow::Result<MemoryId> {
    anyhow::ensure!(!handle.is_empty(), "which memory?");
    let wanted = handle.to_uppercase();
    let exact = MemoryId::new(wanted.clone());
    if store.get(&exact)?.is_some() {
        return Ok(exact);
    }

    let matches: Vec<MemoryId> = store
        .all()?
        .into_iter()
        .filter(|m| m.id.as_str().ends_with(&wanted) || m.id.as_str().starts_with(&wanted))
        .map(|m| m.id)
        .collect();

    match matches.len() {
        0 => anyhow::bail!("no memory called '{handle}'"),
        1 => matches
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no memory called '{handle}'")),
        n => anyhow::bail!(
            "'{handle}' names {n} memories — {}",
            matches
                .iter()
                .take(3)
                .map(|m| render::short(m.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
