//! `aeon ingest` — reading transcripts that already exist.
//!
//! What a source names is walked, converted through its adapter, run past the extractors, and
//! offered to the ladder. Nothing here knows what a harness's records look like; that is the
//! adapter's job and the adapter is Lua.

use crate::Which;
use crate::{now, open, render};
use aeon_distil::{Ingest, Report};
use aeon_model::ScopeId;
use clap::Parser;
use std::path::Path;

/// Read a source's transcripts.
#[derive(Debug, Parser)]
pub struct Args {
    /// Which registered source.
    #[arg(long)]
    source: String,

    /// Only sessions newer than this many days.
    #[arg(long, value_name = "DAYS")]
    since: Option<i64>,

    /// Say what would happen without writing anything.
    #[arg(long)]
    dry_run: bool,

    /// List what each candidate was and what became of it.
    #[arg(long)]
    explain: bool,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Walk a source and offer what it teaches.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let at = now();
    let mut store = open(store_path, scope, tool)?;
    let settings = loaded.settings().clone();

    let ask = Ingest {
        source: args.source.clone(),
        scope: scope.clone(),
        since: args.since.map(|days| at - days * 86_400),
        dry_run: args.dry_run,
        now: at,
    };

    // Naming what is available beats "no source called that". A source is a Lua file somebody
    // has to have installed, so the usual cause is that they have not run `make configs` yet.
    let declared = loaded.sources();
    if !declared.contains(&args.source) {
        anyhow::bail!(
            "no source called '{}'{}",
            args.source,
            if declared.is_empty() {
                " — none are declared; `aeon configs` installs the ones aeon ships".to_owned()
            } else {
                format!(" — declared: {}", declared.join(", "))
            }
        );
    }

    let report = loaded.ingest(&mut store, &settings, &ask)?;

    if args.json {
        crate::say!("{}", as_json(&report));
        return Ok(());
    }
    say(&report, args.explain);
    Ok(())
}

/// Report what was read.
fn say(report: &Report, explain: bool) {
    if report.sessions == 0 {
        crate::say!("{}", render::dim("that source named no sessions"));
        return;
    }

    crate::say!(
        "{} session(s), {} turn(s), {} candidate(s)",
        render::bold(&report.sessions.to_string()),
        report.observations,
        report.proposed
    );
    if report.already_read > 0 {
        crate::say!(
            "{}",
            render::dim(&format!(
                "{} already read by this extractor",
                report.already_read
            ))
        );
    }

    crate::say!();
    crate::say!("  {:>5}  kept", report.promoted);
    crate::say!(
        "  {:>5}  reinforced something already held",
        report.reinforced
    );
    crate::say!("  {:>5}  replaced something", report.superseded);
    crate::say!(
        "  {:>5}  {}",
        report.held,
        render::dim("held, waiting for a second witness")
    );
    crate::say!("  {:>5}  refused", report.refused.len());

    if explain && !report.refused.is_empty() {
        crate::say!();
        for (text, why) in &report.refused {
            crate::say!("  {}  {}", text, render::dim(why));
        }
    }

    if report.dry_run {
        crate::say!();
        crate::say!(
            "{}",
            render::dim("nothing was written — drop --dry-run to apply")
        );
    }
}

/// The same report, for a script.
fn as_json(report: &Report) -> String {
    serde_json::json!({
        "dry_run": report.dry_run,
        "sessions": report.sessions,
        "already_read": report.already_read,
        "observations": report.observations,
        "proposed": report.proposed,
        "promoted": report.promoted,
        "reinforced": report.reinforced,
        "superseded": report.superseded,
        "held": report.held,
        "refused": report.refused.len(),
    })
    .to_string()
}
