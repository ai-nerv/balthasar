//! `balthasar promote` — keep that one.
//!
//! The manual rung on the ladder. Everything else that crosses from a session into a project
//! does so because something was *observed*: a thing recurred in unrelated runs, a command was
//! repaired, a person typed an instruction. This is the case none of those cover — a person
//! looking at what a session holds and deciding.
//!
//! It is the strongest evidence there is, because it is somebody choosing rather than balthasar
//! inferring. So it crosses alone and it pins.

use crate::Which;
use crate::{now, open, render};
use balthasar_model::{ScopeId, SessionId, Tier, Witness, WitnessId, WitnessKind};
use clap::Parser;
use std::path::Path;

/// Carry something out of a session and into the project's memory.
#[derive(Debug, Parser)]
pub struct Args {
    /// The memory, by handle. Omit it with `--from` to be shown the choice.
    handle: Option<String>,

    /// List what a session holds that has not crossed.
    #[arg(long, value_name = "SESSION")]
    from: Option<String>,

    /// Keep it as a habit rather than a fact — something to do, not something true.
    #[arg(long)]
    habit: bool,

    /// Let it fade like anything else, rather than pinning it.
    #[arg(long)]
    no_pin: bool,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Promote, or show what could be.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let at = now();
    let mut store = open(store_path, scope, tool)?;

    if let Some(handle) = &args.handle {
        return keep(&mut store, scope, handle, args, at, loaded);
    }

    // Nothing named: show what is waiting, which is the question a person actually has.
    let session = match &args.from {
        Some(handle) => store
            .session(handle)?
            .ok_or_else(|| anyhow::anyhow!("no session called '{handle}'"))?,
        None => store
            .sessions(1)?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no sessions recorded yet"))?,
    };

    let waiting = store.uncrossed(&session.id)?;
    crate::say!("{}  {}", render::bold(&session.name), session.label());
    crate::say!(
        "{}",
        render::dim("what this run holds. it dies with the session unless something keeps it.")
    );
    crate::say!();
    if waiting.is_empty() {
        crate::say!("{}", render::dim("nothing waiting"));
        return Ok(());
    }
    for memory in &waiting {
        crate::say!(
            "{}  {}",
            render::dim(&render::short(memory.id.as_str())),
            memory.text()
        );
    }
    crate::say!();
    crate::say!(
        "{}",
        render::dim("  balthasar promote <handle>          keep one of them")
    );
    Ok(())
}

/// Carry one memory across.
fn keep(
    store: &mut balthasar_store::Store,
    scope: &ScopeId,
    handle: &str,
    args: &Args,
    at: balthasar_model::Timestamp,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let id = crate::forget::resolve(store, handle)?;
    let before = store
        .get(&id)?
        .ok_or_else(|| anyhow::anyhow!("no memory called '{handle}'"))?;

    let into = if args.habit { Tier::Habit } else { Tier::Fact };
    // A person choosing is the strongest evidence there is: it is somebody deciding rather
    // than balthasar inferring, so it crosses alone and it pins.
    let witness = Witness::new(
        WitnessId::new(format!("kept-{at}")),
        WitnessKind::Imperative,
        SessionId::new("cli"),
        scope.clone(),
        at,
    )
    .noted("kept by hand");

    let confidence = store.promote(&id, into, witness, at)?;
    if !args.no_pin {
        store.pin(&id, true, at)?;
    }

    let after = store
        .get(&id)?
        .ok_or_else(|| anyhow::anyhow!("the store lost what it just kept"))?;
    loaded.tell(
        "promote",
        &[serde_json::json!({
            "id": after.id.to_string(),
            "text": after.text(),
            "tier": after.tier.as_str(),
        })],
    );

    if args.json {
        crate::say!(
            "{}",
            serde_json::json!({
                "id": after.id.to_string(),
                "was": before.tier.as_str(),
                "now": after.tier.as_str(),
                "confidence": after.confidence,
                "pinned": after.strength.pinned,
            })
        );
        return Ok(());
    }

    crate::say!("kept  {}", after.text());
    crate::say!(
        "     {}",
        render::dim(&format!(
            "{} → {} · {} · {}",
            before.tier,
            after.tier,
            render::confidence(if args.no_pin {
                confidence
            } else {
                after.confidence
            }),
            if after.strength.pinned {
                "pinned, so it will not fade"
            } else {
                "unpinned, so it fades like anything else"
            }
        ))
    );
    crate::say!(
        "     {}",
        render::dim("the project's now — every session in it starts knowing this")
    );
    Ok(())
}
