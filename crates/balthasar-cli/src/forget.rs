//! `balthasar forget` — the two kinds of forgetting, which are not the same kind.
//!
//! Archiving keeps everything and stops asserting it. Purging removes it.

use crate::Which;
use crate::{now, open, render, scrollback};
use balthasar_model::{MemoryId, ScopeId};
use balthasar_store::Store;
use clap::Parser;
use std::io::Write;
use std::path::Path;

/// Stop asserting something, or remove it.
#[derive(Debug, Parser)]
pub struct Args {
    /// The memory, by id or by enough of one to be unambiguous.
    ///
    /// With `--session`, a run instead: its name, or enough of its id.
    id: String,

    /// Forget a whole run rather than one memory.
    ///
    /// A run lives in three places: what it promoted, what it said, and the scratch it kept.
    #[arg(long)]
    session: bool,

    /// Remove it entirely, rather than archiving it.
    ///
    /// Irreversible, and the only thing in balthasar that removes a row. Asks unless `--yes`.
    #[arg(long)]
    purge: bool,

    /// Do not ask before purging.
    #[arg(long)]
    yes: bool,

    #[command(flatten)]
    how: crate::render::How,
}

/// Archive or purge.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    if args.session {
        return run_session(store_path, scope, tool, args, loaded);
    }
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
        loaded.tell(
            "forget",
            &[described.clone(), serde_json::json!("archived")],
        );
        if args.how.framed() {
            args.how.one(went("archived", &described));
            return Ok(());
        }
        crate::say!("archived {}", render::dim(&memory.text()));
        crate::say!(
            "     {}",
            render::dim("still in the store, still findable with --archived")
        );
        return Ok(());
    }

    // What else goes, before anything goes.
    let closure = balthasar_store::closure_of(&store, &id)?;
    if closure.derived > 0 {
        crate::say!(
            "{}",
            render::dim(&format!(
                "this also removes {} thing(s) distilled out of it",
                closure.derived
            ))
        );
    }
    if !args.yes && !confirmed(&memory.text())? {
        crate::say!("{}", render::dim("left alone"));
        return Ok(());
    }
    let gone = balthasar_store::purge(&mut store, &id)?;
    anyhow::ensure!(gone == 1, "nothing was removed");
    loaded.tell("forget", &[described.clone(), serde_json::json!("purged")]);
    if args.how.framed() {
        args.how.one(went("purged", &described));
        return Ok(());
    }
    crate::say!("purged {}", render::dim(&memory.text()));
    crate::say!(
        "     {}",
        render::dim(&if closure.derived > 0 {
            format!(
                "gone, with its evidence, its edges and {} derived from it",
                closure.derived
            )
        } else {
            "gone, with its evidence and its edges".to_owned()
        })
    );
    Ok(())
}

/// What happened, for a caller that asked in an encoding.
fn went(what: &str, to: &serde_json::Value) -> serde_json::Value {
    let mut said = to.clone();
    said["forgotten"] = serde_json::json!(what);
    said
}

/// Archive or purge a whole run.
fn run_session(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let at = now();
    let mut store = open(store_path, scope, tool)?;
    let session = store
        .session(&args.id)?
        .map_or_else(|| balthasar_model::SessionId::new(&args.id), |s| s.id);
    let owned = store.owned_by(&session)?;
    let held = scrollback(store_path, scope, tool)?;
    let turns = held.replay(&session)?.len();
    // A handle that names no run is a typo, and must not report a successful purge of nothing.
    anyhow::ensure!(
        !owned.is_empty() || turns > 0,
        "no run called '{}'",
        args.id
    );

    let described = serde_json::json!({
        "session": session.to_string(),
        "memories": owned.len(),
    });

    if !args.purge {
        // Both files: a run's scratch is as much a thing it learned as what it promoted.
        let mut archived = 0;
        for id in &owned {
            store.archive(id, at)?;
            archived += 1;
        }
        let mut pad = balthasar_store::Scratchpad::at(crate::runs_under(store_path, scope, tool));
        // Every agent of the run.
        for agent in pad.agents_of(&session) {
            let Some(own) = pad.peek(&session, &agent)? else {
                continue;
            };
            for id in own.owned_by(&session)? {
                own.archive(&id, at)?;
                archived += 1;
            }
        }
        loaded.tell(
            "forget",
            &[described.clone(), serde_json::json!("archived")],
        );
        if args.how.framed() {
            args.how.one(went("archived", &described));
            return Ok(());
        }
        crate::say!(
            "archived {} from {}",
            render::bold(&format!("{archived} memor(y/ies)")),
            render::dim(&render::short(session.as_str()))
        );
        crate::say!(
            "     {}",
            render::dim("what it said and what it thought are both still there")
        );
        return Ok(());
    }

    if !args.yes
        && !confirmed(&format!(
            "run {} — {} memor(y/ies), {turns} turn(s)",
            render::short(session.as_str()),
            owned.len()
        ))?
    {
        crate::say!("{}", render::dim("left alone"));
        return Ok(());
    }

    // All three, or the run is not forgotten.
    let memories = balthasar_store::purge_session(&mut store, &session)?;
    let gone = balthasar_store::purge_run(&held, &session)?;
    let mut pad = balthasar_store::Scratchpad::at(crate::runs_under(store_path, scope, tool));
    let scratch = balthasar_store::purge_scratch(&mut pad, &session)?;

    loaded.tell("forget", &[described.clone(), serde_json::json!("purged")]);
    if args.how.framed() {
        args.how.one(went("purged", &described));
        return Ok(());
    }
    crate::say!("purged {}", render::bold(&render::short(session.as_str())));
    crate::say!(
        "     {}",
        render::dim(&format!(
            "{memories} memor(y/ies), {gone} turn(s), and {} its own scratch",
            if scratch { "" } else { "no" }
        ))
    );
    Ok(())
}

/// Ask, and take silence for no.
///
/// Reading from a pipe answers "no" rather than "yes": a script has consented to nothing.
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
/// Matched on the trailing handle (see [`render::short`]) first, then a full id or a leading
/// prefix. An ambiguous handle is an error rather than a guess.
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
