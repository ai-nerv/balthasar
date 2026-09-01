//! `aeon train` — fit the ranking policy, and say whether to believe it.
//!
//! Explicit and local. Nothing is downloaded, nothing is uploaded, no job runs in the
//! background. It reads the ledger this store already holds, fits twelve weights, and prints
//! what it learned so a person can disagree with it.
//!
//! It refuses more often than it succeeds, which is the point. Too little data, one-sided data,
//! or a model that does not beat the existing score by a real margin all end with nothing
//! written — because the alternative is a file that reorders results on the strength of forty
//! observations.

use crate::{Which, open, render};
use aeon_model::ScopeId;

use clap::Parser;
use std::path::{Path, PathBuf};

/// Fit the ranking policy from this store's ledger.
#[derive(Debug, Parser)]
pub struct Args {
    /// Read rows from a file instead of this store's ledger.
    #[arg(long, value_name = "FILE")]
    from: Option<PathBuf>,

    /// Write the weights here.
    #[arg(long, value_name = "FILE")]
    into: Option<PathBuf>,

    /// How much data to keep back for the honest number.
    #[arg(long, default_value_t = 0.3)]
    holdout: f64,

    /// How many passes of gradient descent.
    #[arg(long, default_value_t = 500)]
    passes: usize,

    /// Fit and report, writing nothing.
    #[arg(long)]
    dry_run: bool,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Where a fitted policy lives.
///
/// Beside the store rather than inside it: a model is not memory, it is derived from the ledger,
/// and deleting it must cost nothing but a refit.
#[must_use]
pub fn model_path(scope: &ScopeId, tool: &aeon_store::Tool) -> PathBuf {
    aeon_store::scope_path(scope, tool).with_extension("policy.json")
}

/// Fit, report, and write if it earned it.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
) -> anyhow::Result<()> {
    let rows = match &args.from {
        Some(path) => read_rows(path)?,
        None => open(store_path, scope, tool)?.training_rows(1_000_000)?,
    };
    let examples = aeon_testkit::label(&rows);

    let fitted = match aeon_testkit::fit(&examples, args.holdout, args.passes) {
        Ok(held) => held,
        Err(why) => {
            // A refusal, not a failure. Exit successfully and say what is missing, because a
            // command that returns an error status for "not enough data yet" is one people
            // wire into a script and then silence.
            if args.json {
                crate::say!(
                    "{}",
                    serde_json::json!({
                        "trained": false,
                        "rows": rows.len(),
                        "labelled": examples.len(),
                        "because": why.to_string(),
                    })
                );
            } else {
                crate::say!("{}", render::bold("nothing was fitted"));
                crate::say!("  {}", render::dim(&why.to_string()));
                crate::say!();
                crate::say!(
                    "{}",
                    render::dim(&format!(
                        "{} ledger row(s), {} with a countable outcome. A label needs a caller to \
                         report what it did and how it went — `used` then `outcome`.",
                        rows.len(),
                        examples.len()
                    ))
                );
            }
            return Ok(());
        }
    };

    let earned = fitted.beats_the_rules() && fitted.model.is_useful();
    let into = args
        .into
        .clone()
        .unwrap_or_else(|| model_path(scope, &tool.tool));

    if args.json {
        crate::say!(
            "{}",
            serde_json::json!({
                "trained": true,
                "written": earned && !args.dry_run,
                "path": into.to_string_lossy(),
                "examples": fitted.examples,
                "helpful": fitted.helpful,
                "trained_on": fitted.model.trained_on,
                "holdout_auc": fitted.model.holdout_auc,
                "baseline_auc": fitted.baseline_auc,
                "beats_the_rules": fitted.beats_the_rules(),
                "weights": fitted.model.explain()
                    .into_iter()
                    .map(|(name, w)| serde_json::json!({ "feature": name, "weight": w }))
                    .collect::<Vec<_>>(),
            })
        );
        if earned && !args.dry_run {
            fitted.model.save(&into)?;
        }
        return Ok(());
    }

    say(&fitted);

    if args.dry_run {
        crate::say!();
        crate::say!(
            "{}",
            render::dim("nothing was written — drop --dry-run to keep it")
        );
        return Ok(());
    }
    if !earned {
        crate::say!();
        crate::say!(
            "{}",
            render::bold("not written — it did not beat the rules by enough to matter")
        );
        crate::say!(
            "{}",
            render::dim(
                "the existing score is already a good ranker; a tie is not a reason to add a file"
            )
        );
        return Ok(());
    }

    fitted.model.save(&into)?;
    crate::say!();
    crate::say!("{}", render::dim(&format!("written to {}", into.display())));
    crate::say!(
        "{}",
        render::dim("it will not be used until you opt in — see `aeon recall --shadow`")
    );
    Ok(())
}

/// What it learned, and whether that is worth anything.
fn say(fitted: &aeon_testkit::Fitted) {
    crate::say!(
        "{} {}",
        render::bold(&format!("{} labelled example(s)", fitted.examples)),
        render::dim(&format!(
            "{} helpful, {} fitted on, the rest held back",
            fitted.helpful, fitted.model.trained_on
        ))
    );
    crate::say!();

    crate::say!("{}", render::bold("what it learned"));
    for (feature, weight) in fitted.model.explain() {
        // Below this a weight is describing the sample rather than retrieval, and printing it
        // as a finding would invite somebody to reason about noise.
        if weight.abs() < 0.01 {
            continue;
        }
        let bar = "▁▂▃▄▅▆▇█"
            .chars()
            .nth(((weight.abs() * 3.0).min(7.0)) as usize)
            .unwrap_or('▁');
        crate::say!("  {:>+6.2}  {bar}  {}", weight, render::dim(feature));
    }

    crate::say!();
    crate::say!("{}", render::bold("on data it never saw"));
    crate::say!(
        "  {:.3}  {}",
        fitted.model.holdout_auc,
        render::dim("this model")
    );
    crate::say!(
        "  {:.3}  {}",
        fitted.baseline_auc,
        render::dim("the existing score, ranking the same rows")
    );
    crate::say!();
    crate::say!(
        "{}",
        render::dim(
            "0.5 is a coin. The number is the chance it ranks a memory that helped above one \
             that did not."
        )
    );
}

/// Rows from a file `aeon dataset` wrote.
fn read_rows(path: &Path) -> anyhow::Result<Vec<aeon_store::TrainingRow>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        out.push(
            serde_json::from_str(line)
                .map_err(|e| anyhow::anyhow!("{}: line {}: {e}", path.display(), n + 1))?,
        );
    }
    Ok(out)
}
