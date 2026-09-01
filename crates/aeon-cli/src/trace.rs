//! `aeon trace` and `aeon utility` — the two views the ledger adds.
//!
//! Deliberately separate from `aeon why`. That one answers *why do you believe this*, from
//! witnesses. These answer *why do you believe using it helped*, from attributed outcomes. They
//! are different questions with different evidence, and a single command that mixed them would
//! let a memory look true because it was useful, or useless because it was uncertain.

use crate::{Which, now, open, render};
use aeon_model::{MemoryId, ScopeId};
use clap::Parser;
use std::path::Path;

/// Follow one search to whatever came of it.
#[derive(Debug, Parser)]
pub struct TraceArgs {
    /// The recall id, as `aeon recall` printed it.
    recall: String,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// What using a memory has actually led to.
#[derive(Debug, Parser)]
pub struct UtilityArgs {
    /// The memory, by handle or id.
    handle: String,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Print a recall, what it weighed, and what followed.
pub fn trace(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &TraceArgs,
) -> anyhow::Result<()> {
    let store = open(store_path, scope, tool)?;
    let held = store
        .trace_of(&args.recall)?
        .ok_or_else(|| anyhow::anyhow!("no recall called '{}'", args.recall))?;

    if args.json {
        crate::say!(
            "{}",
            serde_json::json!({
                "recall": held.recall,
                "query_hash": held.query_hash,
                "requested_at": held.requested_at,
                "latency_us": held.latency_us,
                "considered": held.considered.len(),
                "actions": held.actions.len(),
            })
        );
        return Ok(());
    }

    crate::say!("{}", render::bold(&held.recall));
    crate::say!(
        "     {}",
        render::dim(&format!(
            "query {} · {} considered · {:.1}ms",
            held.query_hash,
            held.considered.len(),
            held.latency_us as f64 / 1000.0
        ))
    );
    crate::say!();

    for (id, rank, selected, score) in &held.considered {
        let mark = if *selected { "→" } else { " " };
        let short: String = id.as_str().chars().take(8).collect();
        crate::say!(
            "  {mark} {short}  {score:.2}  {}",
            render::dim(&format!("rank {rank}"))
        );
    }

    if held.actions.is_empty() {
        crate::say!();
        crate::say!(
            "{}",
            render::dim(
                "nothing was reported as acted on — which is not the same as nothing having happened"
            )
        );
        return Ok(());
    }

    crate::say!();
    for action in &held.actions {
        crate::say!(
            "  {}  {}",
            render::bold(action.outcome.as_str()),
            render::dim(&format!(
                "{} · {} attribution{}",
                action.id,
                action.attribution,
                action
                    .tool
                    .as_ref()
                    .map(|t| format!(" · {t}"))
                    .unwrap_or_default()
            ))
        );
    }
    Ok(())
}

/// Print what a memory's use has led to.
pub fn utility(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &UtilityArgs,
) -> anyhow::Result<()> {
    let store = open(store_path, scope, tool)?;
    let id = crate::forget::resolve(&store, &args.handle)?;
    let held = store.utility_of(&id)?;
    let (considered, returned) = store.times_retrieved(&id)?;

    if args.json {
        crate::say!(
            "{}",
            serde_json::json!({
                "memory": id.to_string(),
                "verified_helpful": held.verified_helpful,
                "verified_harmful": held.verified_harmful,
                "ignored": held.ignored,
                "unknown": held.unknown,
                "proximal": held.proximal,
                "helpfulness": held.helpfulness(),
                "times_considered": considered,
                "times_returned": returned,
            })
        );
        return Ok(());
    }

    say_utility(&store, &id, &held, considered, returned, now())
}

/// The human view.
fn say_utility(
    store: &aeon_store::Store,
    id: &MemoryId,
    held: &aeon_model::Utility,
    considered: usize,
    returned: usize,
    _at: aeon_model::Timestamp,
) -> anyhow::Result<()> {
    if let Some(memory) = store.get(id)? {
        crate::say!("{}", render::bold(&memory.text()));
    }
    crate::say!();

    match held.helpfulness() {
        None => crate::say!(
            "  {}",
            render::dim("no attributed outcome — nothing has been reported either way")
        ),
        Some(share) => crate::say!(
            "  {} helpful  {}",
            render::bold(&format!("{:.0}%", share * 100.0)),
            render::dim(&format!(
                "{} helped, {} hurt",
                held.verified_helpful, held.verified_harmful
            ))
        ),
    }

    crate::say!();
    crate::say!(
        "  {}",
        render::dim(&format!(
            "{considered} considered · {returned} returned · {} ignored · {} unreported",
            held.ignored, held.unknown
        ))
    );
    if held.proximal > 0 {
        crate::say!(
            "  {}",
            render::dim(&format!(
                "{} proximal observation(s), which are not evidence and are not counted",
                held.proximal
            ))
        );
    }

    // The whole point of the milestone, said out loud when it applies.
    if considered > 4 && !held.is_verified() {
        crate::say!();
        crate::say!(
            "{}",
            render::dim("retrieved often, never verified — popularity, not usefulness")
        );
    }
    crate::say!();
    crate::say!(
        "{}",
        render::dim(
            "`aeon why` answers whether this is true. This answers whether using it helped."
        )
    );
    Ok(())
}
