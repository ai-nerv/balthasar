//! `memo configs` — put the shipped configuration where memo reads it.
//!
//! The binary carries a copy, so a fresh install already behaves correctly. Installing gives
//! you the real files to edit; it does not turn anything on that was off.

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

// The files memo ships, compiled in so a binary is enough on its own.
//
// Built by `build.rs`, which walks `config/`. A new source adapter is then a file and nothing
// else: no list to add it to, and no Rust file naming the harness it reads.
include!(concat!(env!("OUT_DIR"), "/shipped.rs"));

/// Write them out.
pub fn run(args: &Args) -> anyhow::Result<()> {
    let into = PathBuf::from(config_home()).join("memo");
    crate::say!("{}", render::dim(&into.display().to_string()));

    let mut written = 0;
    for (name, body) in SHIPPED {
        let path = into.join(name);
        // An existing file is somebody's edits. Overwriting one because a new version shipped
        // is how a tool loses a person's configuration without ever reporting an error.
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
