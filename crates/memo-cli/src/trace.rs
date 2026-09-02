//! `memo trace` and `memo utility` — the two views the ledger adds.
//!
//! Deliberately separate from `memo why`. That one answers *why do you believe this*, from
//! witnesses. These answer *why do you believe using it helped*, from attributed outcomes. They
//! are different questions with different evidence, and a single command that mixed them would
//! let a memory look true because it was useful, or useless because it was uncertain.

use crate::{Which, now, open, render};
use clap::Parser;
use memo_model::{MemoryId, ScopeId};
use std::path::Path;
use std::path::PathBuf;

/// Follow one search to whatever came of it.
#[derive(Debug, Parser)]
pub struct TraceArgs {
    /// The recall id, as `memo recall` printed it.
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
    store: &memo_store::Store,
    id: &MemoryId,
    held: &memo_model::Utility,
    considered: usize,
    returned: usize,
    _at: memo_model::Timestamp,
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
            "`memo why` answers whether this is true. This answers whether using it helped."
        )
    );
    Ok(())
}

/// Export the ledger as training rows.
#[derive(Debug, Parser)]
pub struct DatasetArgs {
    /// How many rows at most.
    #[arg(long, default_value_t = 10_000)]
    limit: usize,

    /// Write here instead of to standard output.
    #[arg(long, value_name = "FILE")]
    into: Option<PathBuf>,
}

/// Write what a policy could be trained on.
///
/// Explicit, and never automatic. memo starts no training jobs, accumulates no dataset in the
/// background, and sends nothing anywhere — this runs when somebody asks and writes where they
/// say. The rows carry features and outcomes and no content at all.
pub fn dataset(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &DatasetArgs,
) -> anyhow::Result<()> {
    let store = open(store_path, scope, tool)?;
    let rows = store.training_rows(args.limit)?;

    let mut out = String::new();
    for row in &rows {
        out.push_str(&serde_json::to_string(row)?);
        out.push('\n');
    }

    match &args.into {
        Some(path) => {
            std::fs::write(path, &out)?;
            crate::say!(
                "{} {}",
                render::bold(&rows.len().to_string()),
                render::dim(&format!("row(s) → {}", path.display()))
            );
            crate::say!(
                "{}",
                render::dim(
                    "features and outcomes only — no queries, no memory text, no arguments"
                )
            );
        }
        None => print!("{out}"),
    }
    Ok(())
}

/// What a session reported, and how it went.
#[derive(Debug, Parser)]
pub struct OutcomesArgs {
    /// Which run, by name or id.
    #[arg(long)]
    session: Option<String>,

    /// How many.
    #[arg(long, default_value_t = 20)]
    limit: usize,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Print the actions a run reported and what came of them.
pub fn outcomes(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &OutcomesArgs,
) -> anyhow::Result<()> {
    let store = open(store_path, scope, tool)?;

    // Named or most recent. A person asking "how did that go" almost always means the run they
    // just finished, and making them find its id first is a papercut with no upside.
    let session = match &args.session {
        Some(handle) => store
            .session(handle)?
            .ok_or_else(|| anyhow::anyhow!("no session called '{handle}'"))?,
        None => store
            .sessions(1)?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no sessions yet"))?,
    };

    let uses = store.uses_in(&session.id, args.limit)?;
    let mut rows = Vec::with_capacity(uses.len());
    for used in &uses {
        let verdict = store.outcome_of(&used.id)?;
        rows.push((used, verdict));
    }

    if args.json {
        crate::say!(
            "{}",
            serde_json::json!({
                "session": session.id.to_string(),
                "name": session.name,
                "actions": rows.iter().map(|(used, verdict)| serde_json::json!({
                    "action": used.id,
                    "tool": used.tool,
                    "attribution": used.attribution.as_str(),
                    "memories": used.memories.len(),
                    "outcome": verdict.as_ref().map(|v| v.kind.as_str()),
                    "evaluator": verdict.as_ref().map(|v| v.evaluator.clone()),
                })).collect::<Vec<_>>(),
            })
        );
        return Ok(());
    }

    crate::say!("{}", render::bold(session.label()));
    if rows.is_empty() {
        crate::say!();
        crate::say!(
            "{}",
            render::dim("nothing was reported — which is the ordinary case, not a failure")
        );
        return Ok(());
    }

    crate::say!();
    for (used, verdict) in &rows {
        let kind = verdict.as_ref().map_or("unknown", |v| v.kind.as_str());
        crate::say!(
            "  {:<10}  {}",
            render::bold(kind),
            render::dim(&format!(
                "{} memor{} · {} attribution{}",
                used.memories.len(),
                if used.memories.len() == 1 { "y" } else { "ies" },
                used.attribution,
                used.tool
                    .as_ref()
                    .map(|t| format!(" · {t}"))
                    .unwrap_or_default()
            ))
        );
    }

    let unknown = rows.iter().filter(|(_, v)| v.is_none()).count();
    if unknown > 0 {
        crate::say!();
        crate::say!(
            "{}",
            render::dim(&format!(
                "{unknown} unreported — unknown is a real state and never becomes a failure"
            ))
        );
    }
    Ok(())
}
