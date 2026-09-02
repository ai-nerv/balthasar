//! `balthasar trust` — where a memory's evidence came from, and what that permits.
//!
//! The fourth explanation view, and deliberately not part of the other three. `balthasar why` answers
//! whether a memory is credible; this answers whether it is safe to place in a context, which is
//! a different question with different evidence. A memory can be perfectly well witnessed and
//! still be four readings of one untrusted page.

use crate::{Which, open, render};
use balthasar_model::{Channel, Presentation, ScopeId};
use clap::Parser;
use std::collections::BTreeMap;
use std::path::Path;

/// Where a memory came from, and what that permits.
#[derive(Debug, Parser)]
pub struct Args {
    /// The memory, by handle or id.
    handle: String,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Print the trust view.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
) -> anyhow::Result<()> {
    let store = open(store_path, scope, tool)?;
    let id = crate::forget::resolve(&store, &args.handle)?;
    let held = store
        .get(&id)?
        .ok_or_else(|| anyhow::anyhow!("no memory {}", args.handle))?;
    let witnesses = store.witnesses_of(&id)?;

    // Sessions and sources are counted separately, because the gap between them is the whole
    // point: ten runs quoting one page are ten sessions and one source.
    let mut sources: BTreeMap<String, usize> = BTreeMap::new();
    let mut channels: BTreeMap<String, usize> = BTreeMap::new();
    let mut sessions: Vec<String> = Vec::new();
    for witness in &witnesses {
        *sources.entry(witness.domain_of()).or_insert(0) += 1;
        *channels.entry(witness.channel.to_string()).or_insert(0) += 1;
        let run = witness.session.to_string();
        if !sessions.contains(&run) {
            sessions.push(run);
        }
    }

    // The strongest presentation any single channel here permits. A memory whose evidence is
    // all external content cannot be asserted on that evidence alone, however much of it there
    // is — which is a fact about the channel, not about the amount.
    let ceiling = witnesses
        .iter()
        .map(|w| w.channel.ceiling())
        .min()
        .unwrap_or(Presentation::Evidence);
    let external = witnesses.iter().any(|w| w.channel.is_untrusted());
    let inferred = witnesses.iter().all(|w| w.channel.is_inferred()) && !witnesses.is_empty();

    if args.json {
        crate::say!(
            "{}",
            serde_json::json!({
                "memory": id.to_string(),
                "witnesses": witnesses.len(),
                "sessions": sessions.len(),
                "sources": sources.len(),
                "channels": channels.keys().collect::<Vec<_>>(),
                "ceiling": ceiling.as_str(),
                "external": external,
                "inferred_only": inferred,
                "confidence": held.confidence,
            })
        );
        return Ok(());
    }

    crate::say!("{}", render::bold(&held.text()));
    crate::say!();
    crate::say!(
        "  {}  {}",
        render::bold(&format!("{:>2} source(s)", sources.len())),
        render::dim(&format!(
            "across {} session(s), {} witness(es)",
            sessions.len(),
            witnesses.len()
        ))
    );
    if sources.len() < sessions.len() {
        crate::say!(
            "  {}",
            render::dim(
                "fewer sources than sessions — repetition, not corroboration, is doing the work"
            )
        );
    }

    crate::say!();
    for (channel, count) in &channels {
        let note = channel
            .parse::<Channel>()
            .map(|c| {
                if c.may_be_imperative() {
                    "may carry an instruction"
                } else if c.is_untrusted() {
                    "arrived from outside"
                } else if c.is_inferred() {
                    "the agent's own reasoning"
                } else {
                    "observed locally"
                }
            })
            .unwrap_or("unknown channel");
        crate::say!("  {count:>2}  {channel}  {}", render::dim(note));
    }

    crate::say!();
    crate::say!(
        "  {} {}",
        render::bold("ceiling"),
        render::dim(&format!(
            "{} — the strongest presentation this evidence permits on its own",
            ceiling.as_str()
        ))
    );
    if external {
        crate::say!(
            "  {}",
            render::dim("some of it arrived from outside, so it is not stated on its own say-so")
        );
    }
    crate::say!();
    crate::say!(
        "{}",
        render::dim(
            "`balthasar why` answers whether this is credible. This answers whether it is safe to place in a context."
        )
    );
    Ok(())
}

/// Trust as `balthasar why` would want to mention it, in one line.
///
/// Kept here rather than in `why` so the two views stay separable: one sentence pointing at the
/// other command is a link, and merging them would let a memory look true because it came from
/// a trusted channel.
#[must_use]
pub fn one_line(witnesses: &[balthasar_model::Witness]) -> Option<String> {
    if witnesses.is_empty() {
        return None;
    }
    let sources: std::collections::BTreeSet<String> = witnesses
        .iter()
        .map(balthasar_model::Witness::domain_of)
        .collect();
    let sessions: std::collections::BTreeSet<&str> =
        witnesses.iter().map(|w| w.session.as_str()).collect();
    (sources.len() < sessions.len()).then(|| {
        format!(
            "{} session(s) but only {} source(s) — see `balthasar trust`",
            sessions.len(),
            sources.len()
        )
    })
}
