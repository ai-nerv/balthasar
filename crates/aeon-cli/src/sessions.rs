//! `aeon sessions` — which runs this project has had, and what each left behind.
//!
//! A project has many sessions and they share its durable memory. What a session holds on its
//! own dies with it unless something on the ladder carries it across, so "which session" and
//! "which project" are different questions and both have to be answerable.

use crate::Which;
use crate::{now, open, render};
use aeon_model::ScopeId;
use clap::Parser;
use std::path::Path;

/// List the runs this project has had.
#[derive(Debug, Parser)]
pub struct Args {
    /// How many.
    #[arg(long, short, default_value_t = 20)]
    limit: usize,

    /// One session, by name or id, with what it contributed.
    session: Option<String>,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Show the sessions.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
) -> anyhow::Result<()> {
    let at = now();
    let store = open(store_path, scope, tool)?;

    if let Some(handle) = &args.session {
        let session = store
            .session(handle)?
            .ok_or_else(|| anyhow::anyhow!("no session called '{handle}'"))?;
        let kept = store.session_yield(&session.id)?;

        if args.json {
            crate::say!(
                "{}",
                serde_json::json!({
                    "id": session.id.to_string(),
                    "name": session.name,
                    "project": session.scope.to_string(),
                    "title": session.title,
                    "harness": session.harness,
                    "open": session.is_open(),
                    "kept": kept,
                })
            );
            return Ok(());
        }

        crate::say!("{}", render::bold(&session.name));
        crate::say!("{}", render::dim(session.id.as_str()));
        crate::say!();
        crate::say!("  project  {}", session.scope);
        crate::say!("  ran in   {}", session.cwd);
        crate::say!("  harness  {}", session.harness);
        crate::say!("  started  {}", render::ago(session.opened, at));
        crate::say!(
            "  state    {}",
            if session.is_open() { "open" } else { "closed" }
        );
        crate::say!("  kept     {kept} memory(s) this project still has");
        if let Some(title) = &session.title {
            crate::say!();
            crate::say!("  {}", render::dim(title));
        }
        return Ok(());
    }

    let sessions = store.sessions(args.limit)?;
    if args.json {
        for session in &sessions {
            crate::say!(
                "{}",
                serde_json::json!({
                    "id": session.id.to_string(),
                    "name": session.name,
                    "project": session.scope.to_string(),
                    "title": session.title,
                    "kept": store.session_yield(&session.id).unwrap_or(0),
                })
            );
        }
        return Ok(());
    }

    crate::say!("{}", render::bold(scope.as_str()));
    crate::say!(
        "{}",
        render::dim("the project. every session below shares its memory.")
    );
    crate::say!();

    if sessions.is_empty() {
        crate::say!("{}", render::dim("no sessions recorded yet"));
        crate::say!(
            "{}",
            render::dim("  aeon ingest --source <harness>   reads the ones that already happened")
        );
        return Ok(());
    }

    for session in &sessions {
        let kept = store.session_yield(&session.id)?;
        crate::say!("{}  {}", render::bold(&session.name), session.label());
        crate::say!(
            "     {}",
            render::dim(&format!(
                "{} · {} · {} kept{}",
                render::ago(session.opened, at),
                session.harness,
                kept,
                if session.is_open() { " · open" } else { "" }
            ))
        );
    }
    Ok(())
}
