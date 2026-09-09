//! `balthasar configs` — put the shipped configuration where balthasar reads it.

use crate::render;
use clap::Parser;
use std::path::PathBuf;

/// Install the shipped configuration.
#[derive(Debug, Parser)]
pub struct Args {
    /// Overwrite files that are already there.
    #[arg(long)]
    force: bool,

    /// Say what would be written without writing it.
    #[arg(long)]
    dry_run: bool,
}

// The files balthasar ships, compiled in by `build.rs`, which walks `config/`.
include!(concat!(env!("OUT_DIR"), "/shipped.rs"));

/// Write them out.
pub fn run(args: &Args) -> anyhow::Result<()> {
    let into = PathBuf::from(config_home()).join("balthasar");
    crate::say!("{}", render::dim(&into.display().to_string()));

    let mut written = 0;
    for (name, body) in SHIPPED {
        let path = into.join(name);
        if path.exists() && !args.force {
            crate::say!("  {} {}", render::dim("kept"), name);
            continue;
        }
        if args.dry_run {
            crate::say!("  {} {}", render::dim("would write"), name);
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, body)?;
        crate::say!("  {} {name}", render::bold("wrote"));
        written += 1;
    }

    if written > 0 {
        crate::say!();
        crate::say!(
            "{}",
            render::dim("nothing was turned on — every setting in there is commented out")
        );
    }
    Ok(())
}

/// `$XDG_CONFIG_HOME`, or the usual place.
fn config_home() -> String {
    std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{}/.config", std::env::var("HOME").unwrap_or_default()))
}
