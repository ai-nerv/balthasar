//! `balthasar consolidate` — what falls out of an idle machine.
//!
//! Free compute. A daemon with nobody attached is a machine doing nothing, and the literature's
//! "sleep-time compute" is exactly what to spend it on. Shows first, like `decay`: a pass that
//! changes what a project believes should not be a surprise.

use crate::{Which, now, open, render, runs_under, scrollback};
use balthasar_distil::Consolidated;
use balthasar_model::ScopeId;
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

    #[command(flatten)]
    how: crate::render::How,
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
    let unread = balthasar_distil::undistilled(&store, &held, balthasar_distil::RUNS_PER_PASS)?;
    let read = crate::distil::pass(&mut store, &held, scope, &unread, at, !args.commit, loaded)?;

    let mut pad = balthasar_store::Scratchpad::at(runs_under(store_path, scope, tool));
    let settings = loaded.settings().clone();
    let report = loaded.consolidate(&mut store, Some(&mut pad), scope, at, !args.commit)?;

    for text in &report.promoted {
        loaded.tell("consolidate", &[serde_json::json!(text)]);
    }
    let retired = retire(&held, &mut pad, &mut store, &settings, at, args.commit)?;

    if args.how.framed() {
        args.how.one(serde_json::json!({
            "dry_run": report.dry_run,
            "read": read.sessions,
            "read_promoted": read.promoted,
            "decayed": report.decayed,
            "swept": report.swept,
            "clusters": report.clusters,
            "promoted": report.promoted.len(),
            "reinforced": report.reinforced,
            "retired": {
                "sessions": retired.sessions,
                "turns": retired.turns,
                "mirrors": retired.mirrors,
            },
        }));
        return Ok(());
    }
    say_read(&read);
    say(&report, args.explain);
    say_retired(&retired, settings.retain_days, report.dry_run);
    Ok(())
}

/// Turns of quiet sessions past `retain_days`, removed with `commit` and otherwise only counted.
/// Nothing at all when the setting is unset.
fn retire(
    held: &balthasar_store::Transcript,
    pad: &mut balthasar_store::Scratchpad,
    store: &mut balthasar_store::Store,
    settings: &balthasar_lua::Settings,
    at: balthasar_model::Timestamp,
    commit: bool,
) -> anyhow::Result<balthasar_store::Pruned> {
    let Some(before) = settings.retained_since(at) else {
        return Ok(balthasar_store::Pruned::default());
    };
    if commit {
        return Ok(balthasar_store::prune_turns(
            held,
            Some(pad),
            Some(store),
            before,
            &[],
        )?);
    }
    let expired = balthasar_store::expired(held, before, &[])?;
    Ok(balthasar_store::Pruned {
        sessions: expired.len(),
        turns: expired
            .iter()
            .map(|e| usize::try_from(e.turns.saturating_sub(e.cited)).unwrap_or(usize::MAX))
            .sum(),
        mirrors: 0,
    })
}

/// What retention took, or would. Silent when it took nothing.
fn say_retired(retired: &balthasar_store::Pruned, days: Option<u32>, dry_run: bool) {
    let (Some(days), true) = (days, retired.sessions > 0) else {
        return;
    };
    let verb = if dry_run { "would go" } else { "went" };
    crate::say!();
    crate::say!(
        "{} turn(s) of {} session(s) quiet for over {days} days {verb}, every copy of each",
        render::bold(&retired.turns.to_string()),
        retired.sessions
    );
    if dry_run {
        crate::say!(
            "{}",
            render::dim("nothing was written — `balthasar consolidate --now` to apply")
        );
    }
}

/// What reading this project's own runs turned up, when it turned up anything.
///
/// Silent otherwise. `consolidate` is run on a timer, and a pass that announced "0 runs read"
/// every time would train people to stop reading its output.
fn say_read(read: &balthasar_distil::Report) {
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
                balthasar_distil::DISTINCT_SESSIONS
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
            render::dim("nothing was written — `balthasar consolidate --now` to apply")
        );
    }
}
