//! `memo decay` — what today's forgetting would take, before it takes it.
//!
//! The preview is not a convenience. Forgetting is the one thing memo does that a person
//! cannot undo by asking again, so it defaults to showing rather than doing: `memo decay`
//! rehearses, and `memo decay --now` commits.

use crate::Which;
use crate::{now, open, render};
use clap::Parser;
use memo_model::ScopeId;
use memo_store::Faded;
use std::path::Path;

/// Fade what has not been needed.
#[derive(Debug, Parser)]
pub struct Args {
    /// Actually apply it. Without this, nothing is written.
    #[arg(long = "now")]
    commit: bool,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Rehearse or run a decay pass.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
) -> anyhow::Result<()> {
    let at = now();
    let mut store = open(store_path, scope, tool)?;
    let report = if args.commit {
        store.decay(at)?
    } else {
        store.decay_preview(at)?
    };

    if args.json {
        crate::say!("{}", as_json(&report));
        return Ok(());
    }
    say(&report);
    Ok(())
}

/// Print what happened, or what would.
fn say(report: &Faded) {
    let verb = if report.preview {
        "would fade"
    } else {
        "faded"
    };
    let swept = if report.preview {
        "would leave the live set"
    } else {
        "left the live set"
    };

    if report.is_empty() {
        crate::say!("{}", render::dim("nothing has faded since the last pass"));
        if report.pinned > 0 {
            crate::say!(
                "{}",
                render::dim(&format!(
                    "{} pinned, and pinned does not fade",
                    report.pinned
                ))
            );
        }
        return;
    }

    if !report.weakened.is_empty() {
        crate::say!(
            "{} {}",
            render::bold(&report.weakened.len().to_string()),
            verb
        );
        for entry in &report.weakened {
            crate::say!(
                "  {:.2} → {:.2}  {}  {}",
                entry.was,
                entry.now,
                entry.text,
                render::dim(&format!("idle {:.0}d", entry.idle_days))
            );
        }
    }

    if !report.swept.is_empty() {
        crate::say!();
        crate::say!(
            "{} {}",
            render::bold(&report.swept.len().to_string()),
            swept
        );
        for entry in &report.swept {
            crate::say!("  {:.2} → {:.2}  {}", entry.was, entry.now, entry.text);
        }
        // The sentence that makes a sweep bearable. Nothing here is gone.
        crate::say!(
            "  {}",
            render::dim(
                "kept in full, searched with --archived, and still explained by `memo why`"
            )
        );
    }

    if report.pinned > 0 {
        crate::say!();
        crate::say!(
            "{}",
            render::dim(&format!(
                "{} pinned, and pinned does not fade",
                report.pinned
            ))
        );
    }

    if report.preview {
        crate::say!();
        crate::say!(
            "{}",
            render::dim("nothing was written — `memo decay --now` to apply")
        );
    }
}

/// The same report, for a script.
fn as_json(report: &Faded) -> String {
    let entries = |list: &[memo_store::Weakened]| {
        list.iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id.to_string(),
                    "text": e.text,
                    "was": e.was,
                    "now": e.now,
                    "idle_days": e.idle_days,
                })
            })
            .collect::<Vec<_>>()
    };
    serde_json::json!({
        "preview": report.preview,
        "weakened": entries(&report.weakened),
        "swept": entries(&report.swept),
        "pinned": report.pinned,
    })
    .to_string()
}
