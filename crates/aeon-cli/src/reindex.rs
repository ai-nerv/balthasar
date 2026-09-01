//! `aeon reindex` — giving memories their vectors.
//!
//! Off the critical path by design. Nothing waits for this: recall works lexically, and a
//! memory with no vector is scored on the signals it does have. A pass here makes the ranking
//! better; skipping it makes nothing fail.

use crate::{now, open, render};
use aeon_model::ScopeId;
use clap::Parser;
use std::path::Path;

/// Embed what has not been embedded.
#[derive(Debug, Parser)]
pub struct Args {
    /// How many at a time.
    #[arg(long, default_value_t = 64)]
    batch: usize,

    /// Stop after this many, rather than walking everything.
    #[arg(long)]
    limit: Option<usize>,

    /// Say what would happen without writing anything.
    #[arg(long)]
    dry_run: bool,
}

/// Walk the store and embed what needs it.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    args: &Args,
    loaded: &crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let _ = now();
    let Some(embedder) = loaded.embedder() else {
        crate::say!(
            "{}",
            render::dim(
                "no embedder is configured — recall is lexical, which is a supported state"
            )
        );
        return Ok(());
    };

    let mut store = open(store_path, scope)?;
    let model = embedder.model().to_owned();
    let ceiling = args.limit.unwrap_or(usize::MAX);
    let mut done = 0_usize;

    loop {
        let batch = args.batch.min(ceiling.saturating_sub(done)).max(1);
        // Only what has no vector, or one from a different model. A model change invalidates
        // every vector it did not produce, and comparing across them is meaningless.
        let waiting = store.unembedded(&model, batch)?;
        if waiting.is_empty() || done >= ceiling {
            break;
        }

        if args.dry_run {
            done += waiting.len();
            if waiting.len() < batch {
                break;
            }
            continue;
        }

        let texts: Vec<String> = waiting.iter().map(|(_, text)| text.clone()).collect();
        let vectors = embedder.embed(&texts)?;
        for ((id, _), vector) in waiting.iter().zip(&vectors) {
            store.embed(id, vector, &model)?;
        }
        done += waiting.len();
    }

    if done == 0 {
        crate::say!("{}", render::dim("everything is already embedded"));
        return Ok(());
    }
    let verb = if args.dry_run {
        "would embed"
    } else {
        "embedded"
    };
    crate::say!(
        "{verb} {} memory(s) with {}",
        render::bold(&done.to_string()),
        render::dim(&model)
    );
    Ok(())
}
