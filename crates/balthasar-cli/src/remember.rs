//! `balthasar remember` — the manual door onto the ladder.
//!
//! One of the three ways an imperative reaches the store, and the only one that exists at M0.
//! What is typed here is what a person chose to keep, so it arrives as evidence of the
//! strongest kind and pins unless told not to.

use crate::Which;
use crate::{now, open, render};
use balthasar_lua::Floors;
use balthasar_model::{
    Body, Importance, Memory, NoteKind, Privacy, ScopeId, SessionId, Tier, Witness, WitnessId,
    WitnessKind,
};
use balthasar_store::{Landing, mint};
use clap::Parser;
use std::path::Path;

/// Keep something.
#[derive(Debug, Parser)]
pub struct Args {
    /// What to remember.
    text: Vec<String>,

    /// What the claim is about. With `--predicate`, this makes it a slot.
    ///
    /// A slotted fact gets contradiction handling for free: the store refuses to hold two live
    /// answers to one slot, so a later `remember` for the same pair supersedes rather than
    /// piling up. Without a slot the claim is kept and found like any other and simply cannot
    /// be told apart from the claim it replaces.
    #[arg(long)]
    subject: Option<String>,

    /// Which property of the subject.
    #[arg(long, requires = "subject")]
    predicate: Option<String>,

    /// How fast it is allowed to fade.
    #[arg(long, default_value = "normal")]
    importance: String,

    /// Do not pin it — let it decay like anything else.
    #[arg(long)]
    no_pin: bool,

    /// Never send it to a remote model.
    #[arg(long)]
    local: bool,

    /// Attribute this to a session, and keep it only for that session.
    ///
    /// Without it, what is remembered belongs to the project and every session in it shares it
    /// — which is what a durable memory is for. With it, the memory is the session's own and
    /// stays there until something on the ladder carries it across.
    #[arg(long, value_name = "NAME")]
    session: Option<String>,

    /// Say what was done in machine-readable form.
    #[arg(long)]
    json: bool,
}

/// Keep what was typed.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
    floors: Floors,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let text = args.text.join(" ");
    let text = text.trim();
    anyhow::ensure!(!text.is_empty(), "nothing to remember");

    let importance: Importance = args.importance.parse().map_err(|_| {
        anyhow::anyhow!("'{}' is not critical, high, normal or low", args.importance)
    })?;

    let at = now();
    let mut store = open(store_path, scope, tool)?;

    let body = match (&args.subject, &args.predicate) {
        (Some(subject), Some(predicate)) => Body::fact(subject, predicate, text),
        // A claim nobody reduced to a slot is still a claim. Reducing it takes a person or a
        // model, and balthasar requires neither in order to work.
        _ => Body::note(text, NoteKind::Claim),
    };

    // A project's memory is shared by every session in it; a session's is its own. Which of
    // the two this is decides the tier, and the tier decides whether it outlives the run.
    let session = match &args.session {
        Some(handle) => Some(
            store
                .session(handle)?
                .ok_or_else(|| anyhow::anyhow!("no session called '{handle}'"))?,
        ),
        None => None,
    };
    let tier = if session.is_some() {
        Tier::Scratch
    } else {
        Tier::Fact
    };

    let mut memory = Memory::new(mint(at), tier, scope.clone(), body, at);
    memory.session = session.as_ref().map(|s| s.id.clone());
    memory.strength.importance = importance;
    // A session note is not a standing choice about the project, so it is not pinned by
    // default however emphatically it was typed.
    memory.strength.pinned = !args.no_pin && session.is_none();
    memory.privacy = if args.local {
        Privacy::Local
    } else {
        Privacy::Open
    };

    // A person at a keyboard is the strongest evidence there is, and the only kind that pins.
    let witness = Witness::new(
        WitnessId::new(format!(
            "cli-{at}-{}",
            memory.content_hash.get(..8).unwrap_or("0")
        )),
        WitnessKind::Imperative,
        SessionId::new("cli"),
        scope.clone(),
        at,
    )
    .noted("typed at the command line");

    let landing = store.remember(memory, witness, at)?;
    announce(&store, &landing, loaded)?;
    say(&store, &landing, args.json, at, floors)
}

/// Tell whatever the configuration registered.
///
/// A supersession is two events, not one: something became true, and something else stopped
/// being. A handler that only heard the first would have no way to notice a store changing
/// its mind.
fn announce(
    store: &balthasar_store::Store,
    landing: &Landing,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let described = |id: &balthasar_model::MemoryId| -> anyhow::Result<serde_json::Value> {
        Ok(match store.get(id)? {
            Some(memory) => serde_json::json!({
                "id": memory.id.to_string(),
                "text": memory.text(),
                "tier": memory.tier.as_str(),
                "confidence": memory.confidence,
            }),
            None => serde_json::json!({ "id": id.to_string() }),
        })
    };

    if let Landing::Superseded { was, now } = landing {
        loaded.tell("contradict", &[described(was)?, described(now)?]);
    }
    if let Landing::Added(id) | Landing::Superseded { now: id, .. } = landing {
        loaded.tell("promote", &[described(id)?]);
    }
    Ok(())
}

/// Report what the store decided, because the three outcomes are genuinely different.
fn say(
    store: &balthasar_store::Store,
    landing: &Landing,
    json: bool,
    at: balthasar_model::Timestamp,
    floors: Floors,
) -> anyhow::Result<()> {
    let id = landing.id();
    let memory = store
        .get(id)?
        .ok_or_else(|| anyhow::anyhow!("the store lost what it just wrote"))?;

    if json {
        let what = match landing {
            Landing::Added(_) => "added",
            Landing::Reinforced(_) => "reinforced",
            Landing::Superseded { .. } => "superseded",
        };
        let was = match landing {
            Landing::Superseded { was, .. } => Some(was.to_string()),
            _ => None,
        };
        crate::say!(
            "{}",
            serde_json::json!({
                "landing": what,
                "id": id.to_string(),
                "was": was,
                "confidence": memory.confidence,
            })
        );
        return Ok(());
    }

    match landing {
        Landing::Added(_) => crate::say!(
            "remembered {}",
            render::dim(&render::short(&id.to_string()))
        ),
        Landing::Reinforced(_) => crate::say!(
            "already remembered — {} now, from {} session(s)",
            render::confidence(memory.confidence),
            memory.distinct_sessions()
        ),
        Landing::Superseded { was, .. } => {
            let old = store.get(was)?;
            crate::say!(
                "replaced {}",
                old.as_ref().map_or_else(
                    || render::short(&was.to_string()),
                    balthasar_model::Memory::text
                )
            );
            crate::say!(
                "     {}",
                render::dim("kept, and still the answer for its own time")
            );
        }
    }
    // The second line of the rendered form is the standing: what tier, how old, how sure. The
    // first is the text, which the caller has just been told in its own words.
    let rendered = render::line(&memory, floors.inject, at);
    let standing = rendered.lines().nth(1).unwrap_or_default().trim();
    crate::say!("     {}", render::dim(standing));
    Ok(())
}
