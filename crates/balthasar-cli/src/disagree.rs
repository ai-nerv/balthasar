//! `balthasar disagreements` and `balthasar settle` — the person's half of a contradiction.
//!
//! A sweep finds claims that cannot both be true and makes each of them less believed, which is
//! the right thing to do while nobody knows which is wrong. It is not an answer. These two are
//! where the answer comes from: the view says what is open, and `settle` closes one.

use crate::Which;
use crate::{now, open, render};
use balthasar_model::{Memory, MemoryId, ScopeId};
use clap::Parser;
use std::path::Path;

/// Claims that cannot both be true, still waiting on a person.
#[derive(Debug, Parser)]
pub struct ListArgs {
    #[command(flatten)]
    how: crate::render::How,
}

/// Say which of two disagreeing claims is right.
#[derive(Debug, Parser)]
pub struct Args {
    /// One of the two, by id or by enough of one to be unambiguous.
    a: String,
    /// The other.
    b: String,

    /// The one that is right. The other stops being current.
    #[arg(long, value_name = "ID")]
    keep: Option<String>,

    /// They do not disagree after all. Nothing raises the pair again.
    #[arg(long)]
    reconcile: bool,

    /// Both are wrong. Both are archived.
    #[arg(long)]
    neither: bool,

    #[command(flatten)]
    how: crate::render::How,
}

/// What is open.
pub fn list(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &ListArgs,
) -> anyhow::Result<()> {
    let store = open(store_path, scope, tool)?;
    let found = store.disagreements(&scope.to_string())?;

    if args.how.framed() {
        args.how.rows(
            found
                .iter()
                .map(|(a, b)| serde_json::json!({ "a": said(a), "b": said(b) }))
                .collect(),
        );
        return Ok(());
    }

    if found.is_empty() {
        crate::say!("{}", render::dim("nothing disagrees"));
        return Ok(());
    }
    crate::say!(
        "{}",
        render::bold(&format!(
            "{} disagreement{}",
            found.len(),
            if found.len() == 1 { "" } else { "s" }
        ))
    );
    crate::say!();
    for (n, (a, b)) in found.iter().enumerate() {
        for (mark, memory) in [(format!("{:>2}", n + 1), a), ("  ".to_owned(), b)] {
            crate::say!(
                "{} {}  {}  {}",
                render::dim(&mark),
                memory.id,
                render::confidence(memory.confidence),
                render::clip(&memory.text(), 60)
            );
        }
        crate::say!();
    }
    let (a, b) = found.first().map_or_else(
        || ("<id>".to_owned(), "<id>".to_owned()),
        |(a, b)| (render::short(a.id.as_str()), render::short(b.id.as_str())),
    );
    for (flag, what) in [
        ("--keep <id>", "one of them is right"),
        ("--reconcile", "they do not disagree"),
        ("--neither", "both are wrong"),
    ] {
        crate::say!(
            "{}",
            render::dim(&format!("  balthasar settle {a} {b} {flag:<12} {what}"))
        );
    }
    Ok(())
}

/// Close one.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let at = now();
    let mut store = open(store_path, scope, tool)?;
    let a = crate::forget::resolve(&store, &args.a)?;
    let b = crate::forget::resolve(&store, &args.b)?;
    anyhow::ensure!(a != b, "those are the same memory");
    let chosen =
        usize::from(args.keep.is_some()) + usize::from(args.reconcile) + usize::from(args.neither);
    anyhow::ensure!(
        chosen == 1,
        "say one of --keep <id>, --reconcile or --neither"
    );

    let went = if let Some(kept) = args.keep.as_deref() {
        let kept = crate::forget::resolve(&store, kept)?;
        let dropped = match (kept == a, kept == b) {
            (true, _) => b.clone(),
            (_, true) => a.clone(),
            _ => anyhow::bail!("--keep has to name one of the two"),
        };
        let text = text_of(&store, &dropped)?;
        store.settle(&kept, &dropped, at)?;
        crate::say!("kept {}", render::dim(&text_of(&store, &kept)?));
        crate::say!(
            "     {} {}",
            render::dim("no longer current:"),
            render::dim(&text)
        );
        serde_json::json!({ "kept": kept.to_string(), "retired": dropped.to_string() })
    } else if args.reconcile {
        store.reconcile(&a, &b, at)?;
        crate::say!(
            "{}",
            render::dim("taken as agreeing; nothing will raise them again")
        );
        serde_json::json!({ "reconciled": [a.to_string(), b.to_string()] })
    } else {
        store.archive(&a, at)?;
        store.archive(&b, at)?;
        crate::say!(
            "{}",
            render::dim("both archived; still findable with --archived")
        );
        serde_json::json!({ "retired": [a.to_string(), b.to_string()] })
    };

    loaded.tell("settle", std::slice::from_ref(&went));
    if args.how.framed() {
        args.how.one(went);
    }
    Ok(())
}

fn said(memory: &Memory) -> serde_json::Value {
    serde_json::json!({
        "id": memory.id.to_string(),
        "text": memory.text(),
        "confidence": memory.confidence,
    })
}

fn text_of(store: &balthasar_store::Store, id: &MemoryId) -> anyhow::Result<String> {
    Ok(store
        .get(id)?
        .map_or_else(|| id.to_string(), |memory| memory.text()))
}
