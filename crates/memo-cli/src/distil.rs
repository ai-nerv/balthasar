//! `memo distil` — read what the runs here actually said.
//!
//! The extractive rules used to run only on `memo ingest`, which reads a *harness's* own journal
//! files. A session streamed straight into memo's scrollback was never read by them: it earned
//! TIDE when its turns left the window and CALLUS when another run agreed with it, and nothing
//! was watching it for "remember that we deploy with fly.io".
//!
//! This is that pass. Same extractors, same floors, same gate — the only difference is which
//! file the turns were read out of.

use crate::{Which, now, open, render, scrollback};
use clap::Parser;
use memo_model::{ScopeId, SessionId};
use std::path::Path;

/// How many runs one pass will read when nobody named one.
///
/// Newest first, so a project with a long history makes progress every pass rather than timing
/// out on the first. Anything missed is found by the next.
const PER_PASS: usize = 256;

/// Run the extractors over what this project's runs said.
#[derive(Debug, Parser)]
pub struct Args {
    /// One run, by name or id. Omit it to read every run that has not been read.
    session: Option<String>,

    /// Actually apply it. Without this, nothing is written.
    #[arg(long = "now")]
    commit: bool,

    /// Name every claim that crossed, and every one that was refused.
    #[arg(long)]
    explain: bool,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Read, or rehearse reading.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let at = now();
    let mut store = open(store_path, scope, tool)?;
    let held = scrollback(store_path, scope, tool)?;

    let runs = match &args.session {
        Some(handle) => vec![
            store
                .session(handle)?
                .map_or_else(|| SessionId::new(handle), |s| s.id),
        ],
        None => memo_distil::undistilled(&store, &held, PER_PASS)?,
    };

    let waiting = runs.len();
    let total = pass(&mut store, &held, scope, &runs, at, !args.commit, loaded)?;

    if args.json {
        crate::say!(
            "{}",
            serde_json::json!({
                "dry_run": total.dry_run,
                "runs": total.sessions,
                "turns": total.observations,
                "proposed": total.proposed,
                "promoted": total.promoted,
                "reinforced": total.reinforced,
                "superseded": total.superseded,
                "held": total.held,
                "refused": total.refused.len(),
            })
        );
        return Ok(());
    }
    say(&total, waiting, args.explain);
    Ok(())
}

/// Read every run named, adding up what each one taught.
///
/// One run failing does not stop the others: a pass over a hundred sessions that gave up on the
/// first malformed one would never get to the ninety-nine that were fine.
pub(crate) fn pass(
    store: &mut memo_store::Store,
    held: &memo_store::Transcript,
    scope: &ScopeId,
    runs: &[SessionId],
    at: memo_model::Timestamp,
    dry_run: bool,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<memo_distil::Report> {
    let ask = memo_distil::Ingest {
        source: memo_distil::TRANSCRIPT_SOURCE.to_owned(),
        scope: scope.clone(),
        since: None,
        dry_run,
        now: at,
    };
    let mut total = memo_distil::Report {
        dry_run,
        ..memo_distil::Report::default()
    };

    for session in runs {
        let one = loaded.distil(store, held, session, &ask)?;
        total.sessions += one.sessions;
        total.already_read += one.already_read;
        total.observations += one.observations;
        total.proposed += one.proposed;
        total.promoted += one.promoted;
        total.reinforced += one.reinforced;
        total.superseded += one.superseded;
        total.held += one.held;
        total.refused.extend(one.refused);
    }
    Ok(total)
}

/// Print what was learned, or what would be.
fn say(report: &memo_distil::Report, waiting: usize, explain: bool) {
    if report.sessions == 0 {
        // Three different silences, and telling them apart is the difference between "working
        // as intended" and "why is this not doing anything".
        crate::say!(
            "{}",
            render::dim(if waiting == 0 {
                "no runs here have been read by these rules yet — nothing has been recorded"
            } else if report.already_read > 0 {
                "every run here has already been read by these rules"
            } else {
                "those runs hold no turns"
            })
        );
        return;
    }

    let verb = if report.dry_run {
        "would cross"
    } else {
        "crossed"
    };
    crate::say!(
        "{}",
        render::dim(&format!(
            "{} run(s), {} turn(s) — {} proposed",
            report.sessions, report.observations, report.proposed
        ))
    );
    crate::say!();

    if report.promoted > 0 {
        crate::say!(
            "{} {verb} into this project's memory",
            render::bold(&report.promoted.to_string())
        );
    }
    if report.reinforced > 0 {
        crate::say!(
            "{} {}",
            render::bold(&report.reinforced.to_string()),
            render::dim("reinforced something already held")
        );
    }
    if report.superseded > 0 {
        crate::say!(
            "{} {}",
            render::bold(&report.superseded.to_string()),
            render::dim("replaced something that had been true")
        );
    }
    // Held is not refused, and saying so is the point of having three outcomes rather than two.
    if report.held > 0 {
        crate::say!(
            "{} {}",
            render::bold(&report.held.to_string()),
            render::dim("waiting in scratch for a second witness")
        );
    }
    if report.promoted == 0 && report.reinforced == 0 && report.held == 0 {
        crate::say!("{}", render::dim("nothing in these runs earned a place"));
    }

    if explain && !report.refused.is_empty() {
        crate::say!();
        crate::say!("{}", render::dim("refused:"));
        for (text, why) in &report.refused {
            crate::say!("  {}  {}", render::clip(text, 62), render::dim(why));
        }
    }

    if report.dry_run && report.proposed > 0 {
        crate::say!();
        crate::say!(
            "{}",
            render::dim("nothing was written — `memo distil --now` to apply")
        );
    }
}
