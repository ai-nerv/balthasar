//! `aeon consolidate` — what falls out of an idle machine.
//!
//! Free compute. A daemon with nobody attached is a machine doing nothing, and the literature's
//! "sleep-time compute" is exactly what to spend it on. Shows first, like `decay`: a pass that
//! changes what a project believes should not be a surprise.

use crate::{now, open, render};
use aeon_distil::Consolidated;
use aeon_model::ScopeId;
use clap::Parser;
use std::path::Path;

/// Carry what recurred into the project's own memory.
#[derive(Debug, Parser)]
pub struct Args {
    /// Actually apply it. Without this, nothing is written.
    #[arg(long = "now")]
    commit: bool,

    /// Name every claim that crossed.
    #[arg(long)]
    explain: bool,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Run, or rehearse, one cycle.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    args: &Args,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let at = now();
    let mut store = open(store_path, scope)?;
    let settings = loaded.settings().clone();
    let report = aeon_distil::consolidate(&mut store, &settings, scope, at, !args.commit)?;

    for text in &report.promoted {
        loaded.tell("consolidate", &[serde_json::json!(text)]);
    }

    if args.json {
        crate::say!(
            "{}",
            serde_json::json!({
                "dry_run": report.dry_run,
                "decayed": report.decayed,
                "swept": report.swept,
                "clusters": report.clusters,
                "promoted": report.promoted.len(),
                "reinforced": report.reinforced,
            })
        );
        return Ok(());
    }
    say(&report, args.explain);
    Ok(())
}

/// Print what happened, or what would.
fn say(report: &Consolidated, explain: bool) {
    if report.is_empty() {
        crate::say!(
            "{}",
            render::dim("nothing has recurred since the last pass")
        );
        return;
    }

    let verb = if report.dry_run {
        "would cross"
    } else {
        "crossed"
    };
    if !report.promoted.is_empty() {
        crate::say!(
            "{} {verb} into this project's memory",
            render::bold(&report.promoted.len().to_string())
        );
        if explain || report.promoted.len() <= 5 {
            for text in &report.promoted {
                crate::say!("  {text}");
            }
        }
        crate::say!(
            "  {}",
            render::dim(&format!(
                "seen in {}+ separate sessions — one run repeating itself would not have",
                aeon_distil::DISTINCT_SESSIONS
            ))
        );
    }
    if report.reinforced > 0 {
        crate::say!();
        crate::say!(
            "{} {}",
            render::bold(&report.reinforced.to_string()),
            render::dim("reinforced something this project already believed")
        );
    }
    if report.decayed > 0 || report.swept > 0 {
        crate::say!();
        crate::say!(
            "{}",
            render::dim(&format!(
                "{} faded, {} left the live set",
                report.decayed, report.swept
            ))
        );
    }
    if report.dry_run {
        crate::say!();
        crate::say!(
            "{}",
            render::dim("nothing was written — `aeon consolidate --now` to apply")
        );
    }
}
