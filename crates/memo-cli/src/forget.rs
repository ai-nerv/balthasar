//! `memo forget` — the two kinds of forgetting, which are not the same kind.
//!
//! Archiving keeps everything and stops asserting it. Purging removes it. The first is what
//! "I do not need this any more" means; the second is what "delete the key I pasted" means, and
//! conflating them would make one of those two sentences unanswerable.

use crate::Which;
use crate::{now, open, render, scrollback};
use clap::Parser;
use memo_model::{MemoryId, ScopeId};
use memo_store::Store;
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
    /// A run lives in three places — what it promoted into the project, what it said, and the
    /// scratch it never promoted — and forgetting one of those is not forgetting the run.
    #[arg(long)]
    session: bool,

    /// Remove it entirely, rather than archiving it.
    ///
    /// Irreversible, and the only thing in memo that removes a row. Asks first unless
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
        loaded.tell("forget", &[described, serde_json::json!("archived")]);
        crate::say!("archived {}", render::dim(&memory.text()));
        crate::say!(
            "     {}",
            render::dim("still in the store, still findable with --archived")
        );
        return Ok(());
    }

    // What else goes, before anything goes. A person asked to confirm removing one claim, and
    // being told afterwards that it took three beliefs with it is the wrong order.
    let closure = memo_store::closure_of(&store, &id)?;
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
    let gone = memo_store::purge(&mut store, &id)?;
    anyhow::ensure!(gone == 1, "nothing was removed");
    loaded.tell("forget", &[described, serde_json::json!("purged")]);
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
        .map_or_else(|| memo_model::SessionId::new(&args.id), |s| s.id);
    let owned = store.owned_by(&session)?;
    let held = scrollback(store_path, scope, tool)?;
    let turns = held.replay(&session)?.len();
    // A handle that names no run is a typo, and a typo must not report a successful purge of
    // nothing — the next thing somebody does is stop looking for the run they meant.
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
        // Both files. Archiving keeps every word, so the turns stay — but a run's scratch is
        // as much a thing it learned as what it promoted, and archiving one and not the other
        // would leave half the run still asserting itself.
        let mut archived = 0;
        for id in &owned {
            store.archive(id, at)?;
            archived += 1;
        }
        let mut pad = memo_store::Scratchpad::at(crate::runs_under(store_path, scope, tool));
        if let Some(own) = pad.peek(&session)? {
            for id in own.owned_by(&session)? {
                own.archive(&id, at)?;
                archived += 1;
            }
        }
        loaded.tell("forget", &[described, serde_json::json!("archived")]);
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

    // All three, or the run is not forgotten. A purge that cleared the project store and left
    // the transcript would answer "delete the key I pasted" with the key still in the file.
    let memories = memo_store::purge_session(&mut store, &session)?;
    let gone = memo_store::purge_run(&held, &session)?;
    let mut pad = memo_store::Scratchpad::at(crate::runs_under(store_path, scope, tool));
    let scratch = memo_store::purge_scratch(&mut pad, &session)?;

    loaded.tell("forget", &[described, serde_json::json!("purged")]);
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
/// Reading from a pipe answers "no" rather than "yes": a script that pipes into `memo forget
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
