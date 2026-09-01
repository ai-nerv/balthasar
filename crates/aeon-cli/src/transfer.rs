//! `aeon export` and `aeon import` — the backup, and the way back.
//!
//! JSONL, one memory per line, witnesses and links included. A memory system without an export
//! is a memory system that owns you, and the export is also the only way to inspect what a
//! store holds without trusting the code that prints it.

use crate::Which;
use crate::{now, open};
use aeon_model::{Memory, ScopeId};
use clap::Parser;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// Write everything out.
#[derive(Debug, Parser)]
pub struct ExportArgs {
    /// Where to write. Standard output when not given.
    #[arg(long, short)]
    out: Option<PathBuf>,
}

/// Read something back.
#[derive(Debug, Parser)]
pub struct ImportArgs {
    /// The file to read. Standard input when not given.
    file: Option<PathBuf>,

    /// Say what would happen without writing anything.
    #[arg(long)]
    dry_run: bool,
}

/// Every memory, one JSON object per line, oldest first.
pub fn export(store_path: Option<&Path>, scope: &ScopeId, tool: &Which, args: &ExportArgs) -> anyhow::Result<()> {
    let store = open(store_path, scope, tool)?;
    let everything = store.all()?;

    let mut out: Box<dyn Write> = match &args.out {
        Some(path) => Box::new(std::fs::File::create(path)?),
        None => Box::new(std::io::stdout().lock()),
    };
    for memory in &everything {
        writeln!(out, "{}", serde_json::to_string(memory)?)?;
    }
    out.flush()?;
    if args.out.is_some() {
        eprintln!("{} memories", everything.len());
    }
    Ok(())
}

/// Read memories back in.
///
/// Each line goes through `remember`, not through a raw insert: an import is evidence arriving,
/// and it must land on the same ladder as everything else. Importing a store into itself
/// therefore reinforces rather than duplicating, which is the property that makes an import
/// safe to re-run.
pub fn import(store_path: Option<&Path>, scope: &ScopeId, tool: &Which, args: &ImportArgs) -> anyhow::Result<()> {
    let at = now();
    let source: Box<dyn BufRead> = match &args.file {
        Some(path) => Box::new(std::io::BufReader::new(std::fs::File::open(path)?)),
        None => Box::new(std::io::stdin().lock()),
    };

    let mut store = open(store_path, scope, tool)?;
    let (mut added, mut reinforced, mut superseded, mut skipped) = (0, 0, 0, 0);

    for (number, line) in source.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let memory: Memory =
            serde_json::from_str(&line).map_err(|e| anyhow::anyhow!("line {}: {e}", number + 1))?;
        // A memory with no evidence cannot be imported as a durable one: the whole design is
        // that a fact answers for itself, and an import is not an exception to that.
        let Some(witness) = memory.witnesses.first().cloned() else {
            skipped += 1;
            continue;
        };
        if args.dry_run {
            added += 1;
            continue;
        }
        match store.remember(memory, witness, at)? {
            aeon_store::Landing::Added(_) => added += 1,
            aeon_store::Landing::Reinforced(_) => reinforced += 1,
            aeon_store::Landing::Superseded { .. } => superseded += 1,
        }
    }

    let verb = if args.dry_run { "would add" } else { "added" };
    crate::say!(
        "{verb} {added}, reinforced {reinforced}, superseded {superseded}, skipped {skipped}"
    );
    Ok(())
}
