//! `aeon context` — exactly what a model would be told.
//!
//! Memory that cannot be inspected before it reaches a model is memory nobody will trust. This
//! is the same code path a harness gets over the socket, printed instead of sent.

use crate::Which;
use crate::{now, open, render};
use aeon_model::ScopeId;
use aeon_recall::{Ask, Bound, Context, Section};
use clap::Parser;
use std::path::Path;

/// Show what would be injected.
#[derive(Debug, Parser)]
pub struct Args {
    /// The turn in hand, for sections that score against it.
    turn: Vec<String>,

    /// How many tokens memory may claim.
    #[arg(long, default_value_t = 1000)]
    budget: usize,

    /// Where it is going. A local model and somebody's API are not the same boundary.
    #[arg(long, value_name = "WHERE", default_value = "local")]
    r#for: String,

    /// Print the text a harness would inject, and nothing else.
    #[arg(long)]
    raw: bool,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Assemble and print.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let at = now();
    let bound = match args.r#for.as_str() {
        "local" => Bound::Local,
        "remote" => Bound::Remote,
        other => anyhow::bail!("'{other}' is not local or remote"),
    };

    let sections = Section::all(&loaded.config());
    anyhow::ensure!(
        !sections.is_empty(),
        "no sections are declared — `aeon configs` installs the ones aeon ships"
    );

    let stores = if store_path.is_some() || scope.is_global() {
        vec![(open(store_path, scope, tool)?, true)]
    } else {
        vec![
            (open(None, scope, tool)?, true),
            (open(None, &ScopeId::global(), tool)?, false),
        ]
    };

    let ask = Ask {
        turn: args.turn.join(" "),
        tokens: args.budget,
        bound,
        floor: loaded.settings().floors().inject,
        weights: crate::weights_of(loaded.settings(), loaded.embedder().is_some()),
        now: at,
        scope: scope.to_string(),
    };

    // Redaction is asked of the configuration, one line at a time, at the boundary where
    // memory leaves rather than where it is stored.
    let mut withheld = Vec::new();
    let context = aeon_recall::assemble(&stores, &sections, &ask, |text, memory| {
        loaded.redact(text, memory, bound.is_remote(), &mut withheld)
    })?;

    if args.raw {
        print!("{}", context.text());
        return Ok(());
    }
    if args.json {
        crate::say!("{}", as_json(&context));
        return Ok(());
    }
    say(&context, args.budget, &withheld);
    Ok(())
}

/// Print it the way a person reads it.
fn say(context: &Context, budget: usize, withheld: &[String]) {
    if context.is_empty() {
        crate::say!("{}", render::dim("nothing would be injected"));
        return;
    }

    for section in &context.sections {
        crate::say!("{}", render::bold(&section.id));
        for line in &section.lines {
            crate::say!("{line}");
        }
        crate::say!("{}", render::dim(&format!("  {} token(s)", section.tokens)));
        crate::say!();
    }

    crate::say!(
        "{}",
        render::dim(&format!(
            "{} of {budget} token(s) · {} restatement(s) dropped · {} withheld",
            context.tokens, context.deduplicated, context.redacted
        ))
    );
    for line in withheld {
        crate::say!("{}", render::dim(&format!("  withheld: {line}")));
    }
}

/// The same, for a harness that wants the parts.
fn as_json(context: &Context) -> String {
    serde_json::json!({
        "tokens": context.tokens,
        "deduplicated": context.deduplicated,
        "redacted": context.redacted,
        "text": context.text(),
        "sections": context.sections.iter().map(|s| serde_json::json!({
            "id": s.id,
            "lines": s.lines,
            "tokens": s.tokens,
        })).collect::<Vec<_>>(),
    })
    .to_string()
}
