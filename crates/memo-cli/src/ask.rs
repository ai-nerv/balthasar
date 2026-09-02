//! `memo why` — the evidence, and what it adds up to.
//!
//! The single best debugging tool a memory system can have, and the reason witnesses are a
//! table rather than a number. A confidence nobody can interrogate is a confidence nobody
//! should act on.

use crate::Which;
use crate::{now, open, render};
use clap::Parser;
use memo_lua::Floors;
use memo_model::{LinkRelation, ScopeId};
use std::path::Path;

/// Explain a memory.
#[derive(Debug, Parser)]
pub struct Args {
    /// The memory, by id or by enough of one to be unambiguous.
    id: String,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// Print the argument for a memory.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
    floors: Floors,
) -> anyhow::Result<()> {
    let at = now();
    let store = open(store_path, scope, tool)?;
    let id = crate::forget::resolve(&store, &args.id)?;
    let memory = store
        .get(&id)?
        .ok_or_else(|| anyhow::anyhow!("no memory called {}", args.id))?;

    if args.json {
        crate::say!("{}", serde_json::to_string(&memory)?);
        return Ok(());
    }

    crate::say!("{}", render::bold(&memory.text()));
    crate::say!(
        "{}",
        render::dim(&format!(
            "{}  {} · {}",
            memory.id,
            render::standing(&memory, floors.inject, at),
            memory.tier
        ))
    );
    crate::say!();

    // Which project, and which of its sessions. A project has many, and they share its
    // durable memory, so neither question answers the other.
    crate::say!("{}   {}", render::bold("where"), memory.scope);
    if let Some(session) = &memory.session {
        let named = store.session_by_id(session)?.map_or_else(
            || session.to_string(),
            |s| format!("{} — {}", s.name, s.label()),
        );
        crate::say!("{}  {}", render::bold("learned"), named);
    }
    crate::say!();

    crate::say!(
        "{} {:.2}  {}",
        render::bold("confidence"),
        memory.confidence,
        render::bar(memory.confidence)
    );
    crate::say!(
        "{}   {:.2}  {}  {}",
        render::bold("strength"),
        memory.strength.at(at),
        render::bar(memory.strength.at(at)),
        render::dim(&format!(
            "{}{}",
            memory.strength.importance,
            if memory.strength.pinned {
                ", pinned"
            } else {
                ""
            }
        ))
    );
    crate::say!();

    // The whole point. Newest first: what most recently convinced us, then what did before.
    crate::say!(
        "{} {}",
        render::bold("because"),
        render::dim(&format!(
            "{} witness(es) across {} session(s)",
            memory.witnesses.len(),
            memory.distinct_sessions()
        ))
    );
    // What the witnesses actually saw, when the scrollback still holds it. The difference
    // between naming a cursor at somebody and showing them the turn.
    let scrollback = crate::scrollback(store_path, scope, tool).ok();
    let names: std::collections::HashMap<String, String> = store
        .sessions(usize::MAX)?
        .into_iter()
        .map(|s| (s.id.to_string(), s.name))
        .collect();
    for witness in &memory.witnesses {
        let named = names.get(witness.session.as_str()).map(String::as_str);
        crate::say!("{}", render::evidence(witness, at, named));

        let quoted = witness.cursor.and_then(|cursor| {
            scrollback
                .as_ref()?
                .at(&witness.session, cursor)
                .ok()
                .flatten()
        });
        if let Some(turn) = quoted
            && !turn.text.trim().is_empty()
        {
            crate::say!(
                "{}",
                render::dim(&format!(
                    "                 “{}”",
                    render::clip(&turn.text, 96)
                ))
            );
        }
    }
    if memory.witnesses.is_empty() {
        crate::say!(
            "  {}",
            render::dim("nothing — which is why it is not asserted")
        );
    }

    // A pointer, not a merge. Whether this is credible and whether it is safe to place in a
    // context are different questions, and answering both here would let a memory look true
    // because its evidence came from somewhere trusted.
    if let Some(said) = crate::trust::one_line(&memory.witnesses) {
        crate::say!("  {}", render::dim(&said));
    }

    let about = store.entities_of(&memory.id)?;
    if !about.is_empty() {
        crate::say!();
        crate::say!(
            "{}   {}",
            render::bold("about"),
            about
                .iter()
                .map(|e| e.display.clone())
                .collect::<Vec<_>>()
                .join(" · ")
        );
    }

    let when = &memory.temporal;
    crate::say!();
    crate::say!(
        "{}  {} {}",
        render::bold("since"),
        render::ago(when.valid_from, at),
        match when.valid_to {
            Some(end) => render::dim(&format!("until {}", render::ago(end, at))),
            None => render::dim("still"),
        }
    );

    if !memory.links.is_empty() {
        crate::say!();
        crate::say!("{}", render::bold("related"));
        for link in &memory.links {
            let other = store.get(&link.to)?;
            let what = other.as_ref().map_or_else(
                || render::short(&link.to.to_string()),
                memo_model::Memory::text,
            );
            crate::say!("  {:<12} {}", relation(link.rel), what);
        }
    }
    Ok(())
}

/// A relation, in words a person reads rather than a column spelling.
fn relation(rel: LinkRelation) -> &'static str {
    match rel {
        LinkRelation::Supersedes => "replaced",
        LinkRelation::Contradicts => "contradicted by",
        LinkRelation::Supports => "supported by",
        LinkRelation::DerivedFrom => "distilled from",
        LinkRelation::About => "about",
    }
}
