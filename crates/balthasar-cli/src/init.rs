//! `balthasar init` — say that memory for this subtree belongs here.
//!
//! Not needed in an ordinary checkout, where opening a store creates a home at the root.

use crate::render;
use clap::Parser;
use std::path::PathBuf;

/// Make this directory the root of its own memory.
#[derive(Debug, Parser)]
pub struct Args {
    /// Where, if not here.
    #[arg(value_name = "DIR")]
    at: Option<PathBuf>,

    #[command(flatten)]
    how: crate::render::How,
}

/// Create the store home.
pub fn run(args: &Args) -> anyhow::Result<()> {
    let at = match &args.at {
        Some(said) => said.clone(),
        None => std::env::current_dir()?,
    };
    let home = at.join(balthasar_store::HOME);
    let existed = balthasar_store::project_home(&balthasar_model::ScopeId::new(
        at.to_string_lossy().into_owned(),
    ))
    .is_some_and(|_| home.join(".store").is_file());
    balthasar_store::make_home(&home)?;

    if args.how.framed() {
        args.how
            .one(serde_json::json!({ "home": home.to_string_lossy(), "existed": existed }));
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
