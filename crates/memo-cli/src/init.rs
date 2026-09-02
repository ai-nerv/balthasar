//! `memo init` — say that memory for this subtree belongs here.
//!
//! Not needed for an ordinary project: opening a store in a checkout creates its home at the
//! repository root. This is for the case the root gets wrong — a monorepo where each package
//! should remember separately, or a directory that is not a checkout at all and would otherwise
//! keep its memory in the data directory.

use crate::render;
use clap::Parser;
use std::path::PathBuf;

/// Make this directory the root of its own memory.
#[derive(Debug, Parser)]
pub struct Args {
    /// Where, if not here.
    #[arg(value_name = "DIR")]
    at: Option<PathBuf>,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Create the store home.
pub fn run(args: &Args) -> anyhow::Result<()> {
    let at = match &args.at {
        Some(said) => said.clone(),
        None => std::env::current_dir()?,
    };
    let home = at.join(memo_store::HOME);
    let existed =
        memo_store::project_home(&memo_model::ScopeId::new(at.to_string_lossy().into_owned()))
            .is_some_and(|_| home.join(".store").is_file());
    memo_store::make_home(&home)?;

    if args.json {
        crate::say!(
            "{}",
            serde_json::json!({ "home": home.to_string_lossy(), "existed": existed })
        );
        return Ok(());
    }
    crate::say!("{}", render::bold(&home.display().to_string()));
    crate::say!(
        "{}",
        render::dim(if existed {
            "already this subtree's own memory"
        } else {
            "this subtree keeps its own memory from here"
        })
    );
    Ok(())
}
