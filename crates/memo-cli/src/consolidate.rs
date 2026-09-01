//! `memo consolidate` — what falls out of an idle machine.
//!
//! Free compute. A daemon with nobody attached is a machine doing nothing, and the literature's
//! "sleep-time compute" is exactly what to spend it on. Shows first, like `decay`: a pass that
//! changes what a project believes should not be a surprise.

use crate::{Which, now, open, render, runs_under, scrollback};
use clap::Parser;
use memo_distil::Consolidated;
use memo_model::ScopeId;
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
    tool: &Which,
    args: &Args,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let at = now();
    let mut store = open(store_path, scope, tool)?;

    // What the runs here said, before what they had in common. A pass that clustered first
    // would be looking for corroboration among scratch it had not finished reading — and the
    // rules are where "the person told us" comes from, which is the strongest thing there is.
    let held = scrollback(store_path, scope, tool)?;
    let unread = memo_distil::undistilled(&store, &held, memo_distil::RUNS_PER_PASS)?;
    let read = crate::distil::pass(&mut store, &held, scope, &unread, at, !args.commit, loaded)?;

    let mut pad = memo_store::Scratchpad::at(runs_under(store_path, scope, tool));
    let settings = loaded.settings().clone();
    let report = memo_distil::consolidate(
        &mut store,
        Some(&mut pad),
        &settings,
        scope,
        at,
        !args.commit,
    )?;

    for text in &report.promoted {
        loaded.tell("consolidate", &[serde_json::json!(text)]);
    }

    if args.json {
        crate::say!(
            "{}",
            serde_json::json!({
                "dry_run": report.dry_run,
                "read": read.sessions,
                "read_promoted": read.promoted,
                "decayed": report.decayed,
                "swept": report.swept,
                "clusters": report.clusters,
                "promoted": report.promoted.len(),
                "reinforced": report.reinforced,
            })
        );
        return Ok(());
    }
    say_read(&read);
    say(&report, args.explain);
    Ok(())
}

/// What reading this project's own runs turned up, when it turned up anything.
///
/// Silent otherwise. `consolidate` is run on a timer, and a pass that announced "0 runs read"
/// every time would train people to stop reading its output.
fn say_read(read: &memo_distil::Report) {
    if read.sessions == 0 {
        return;
    }
    crate::say!(
        "{}",
        render::dim(&format!(
            "read {} run(s) of this project's own transcript — {} proposed, {} crossed, {} waiting",
            read.sessions, read.proposed, read.promoted, read.held
        ))
    );
    crate::say!();
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
                memo_distil::DISTINCT_SESSIONS
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
            render::dim("nothing was written — `memo consolidate --now` to apply")
        );
    }
}
