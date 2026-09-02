//! `balthasar replay` — everything a run said, back out again.
//!
//! For a harness that keeps no journal of its own this is how a session is restored, and the
//! `raw` field is the point: what comes back is exactly what was written, byte for byte, in
//! whatever shape the harness uses. balthasar stores those records and never parses them.

use crate::Which;
use crate::{open, render, scrollback};
use balthasar_model::{ScopeId, SessionId};
use clap::Parser;
use std::path::Path;

/// Show, or hand back, what a run said.
#[derive(Debug, Parser)]
pub struct Args {
    /// The run, by name or by id. Omit it to list what there is.
    session: Option<String>,

    /// Hand back the harness's own records, one JSON object per line.
    ///
    /// What a harness restoring a session reads. Without it this prints the turns for a person.
    #[arg(long)]
    raw: bool,

    /// Say where a resuming harness should carry on from.
    #[arg(long)]
    resume: bool,

    /// The most recent turns, within a token budget, instead of the whole run.
    ///
    /// A long run has no upper bound — balthasar is the only copy of it — so reading all of one to
    /// look at the end is the wrong shape as soon as a session gets long.
    #[arg(long, value_name = "TOKENS")]
    tail: Option<usize>,

    /// Only this cursor range. What an episode is.
    #[arg(long, value_name = "FROM..TO")]
    span: Option<String>,

    /// The turns either side of this one.
    #[arg(long, value_name = "CURSOR")]
    around: Option<u64>,

    /// Only turns mentioning all of these.
    #[arg(long, value_name = "WORD", num_args = 1..)]
    matching: Vec<String>,
}

/// Which bounded read the flags asked for, if any.
fn wanted(args: &Args) -> anyhow::Result<Option<balthasar_store::Want>> {
    if let Some(range) = &args.span {
        let (from, to) = range
            .split_once("..")
            .ok_or_else(|| anyhow::anyhow!("a span reads `from..to`, as in `--span 10..40`"))?;
        return Ok(Some(balthasar_store::Want::Span {
            from: from.trim().parse()?,
            to: to.trim().parse()?,
        }));
    }
    if let Some(cursor) = args.around {
        return Ok(Some(balthasar_store::Want::Around { cursor }));
    }
    if !args.matching.is_empty() {
        return Ok(Some(balthasar_store::Want::Matching {
            terms: args.matching.clone(),
        }));
    }
    Ok(args.tail.map(|_| balthasar_store::Want::Tail))
}

/// Replay, or list.
pub fn run(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &Args,
) -> anyhow::Result<()> {
    let held = scrollback(store_path, scope, tool)?;

    let Some(handle) = &args.session else {
        let (runs, turns) = held.census()?;
        crate::say!("{}", render::bold(scope.as_str()));
        crate::say!(
            "{}",
            render::dim(&format!(
                "{}  ·  {runs} run(s), {turns} turn(s)",
                held.path().display()
            ))
        );
        crate::say!();
        for run in held.runs(30)? {
            crate::say!(
                "{}  {:>5} turns  {}",
                render::dim(&render::short(run.session.as_str())),
                run.turns,
                if run.closed.is_some() { "" } else { "open" }
            );
        }
        return Ok(());
    };

    // A name is what gets printed, so a name is what somebody will type. The memory store is
    // what knows them.
    let store = open(store_path, scope, tool)?;
    let session = store
        .session(handle)?
        .map_or_else(|| SessionId::new(handle), |s| s.id);

    if args.resume {
        let next = held.next_cursor(&session)?;
        let turns = held.replay(&session)?.len();
        crate::say!("{}", serde_json::json!({ "next": next, "turns": turns }));
        return Ok(());
    }

    // Bounded when asked, whole otherwise. Restoring a session needs every turn and truncating
    // it would hand back a run quietly missing its beginning; looking at one needs a slice.
    let (turns, note) = match wanted(args)? {
        None => (held.replay(&session)?, None),
        Some(want) => {
            let budget = balthasar_store::Budget {
                tokens: args.tail.unwrap_or(4_000),
                ..balthasar_store::Budget::default()
            };
            let read = held.read(&session, &want, &budget)?;
            (read.turns.clone(), read.note())
        }
    };
    if turns.is_empty() {
        crate::say!("{}", render::dim("nothing was recorded for that run"));
        return Ok(());
    }

    for turn in &turns {
        if args.raw {
            // Exactly what the harness wrote. A turn balthasar was given no record for is skipped
            // rather than invented — a replay that made something up would be worse than a
            // short one.
            if let Some(raw) = &turn.raw {
                crate::say!("{raw}");
            }
            continue;
        }
        crate::say!(
            "{} {}  {}",
            render::dim(&format!("{:>5}", turn.cursor)),
            render::bold(&format!("{:<9}", turn.role)),
            render::clip(&turn.text, 92)
        );
        if turn.revisions > 0 {
            crate::say!(
                "{}",
                render::dim(&format!("        revised {}×", turn.revisions))
            );
        }
    }
    // Said last, where it is read after the turns rather than before them. A slice that did
    // not say it was a slice would be indistinguishable from the whole run.
    if let Some(said) = note {
        crate::say!();
        crate::say!("{}", render::dim(&said));
    }
    Ok(())
}
