//! Sleep: what falls out of an idle machine.
//!
//! The two paths that cannot be walked inside a single session, because both are about what
//! happens *across* them:
//!
//! * **CALLUS** — something that recurred in unrelated runs. One session repeating a thing is a
//!   person being emphatic; the same thing surfacing in runs that knew nothing of each other is
//!   a property of the world.
//! * **SLEEP** — the pass itself: decay first, so a promotion is judged on current strength,
//!   then cluster, then promote, then sweep what is spent.
//!
//! Costs no model. Clustering is by content hash, which is exact and free; a distiller makes it
//! better and its absence does not stop it.

use crate::DistilError;
use memo_lua::Settings;
use memo_model::{
    Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
};
use memo_store::{Cluster, Landing, Store, mint};

/// How many distinct sessions must have seen something before it belongs to the project.
///
/// Two, and it is the whole of CALLUS. One is emphasis; two is corroboration. Higher would make
/// a memory layer that learns nothing from a project worked on twice.
pub const DISTINCT_SESSIONS: usize = 2;

/// How far back a consolidation pass looks for scratch worth corroborating.
///
/// Thirty days. Older scratch has either crossed the ladder already or decayed out of the live
/// set, so opening its file would be paying to read what cannot be promoted. Bounding this is
/// what stops a pass from scaling with the whole history of a project.
pub const HORIZON: memo_model::Timestamp = 30 * 24 * 60 * 60;

/// How many runs one consolidation pass will open.
///
/// Newest first, so a project with ten thousand runs makes progress on every pass rather than
/// timing out on all of them. The number is a wall-clock budget rather than a correctness one:
/// anything missed this pass is found by the next.
pub const RUNS_PER_PASS: usize = 256;

/// What a pass did, or would do.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Consolidated {
    /// Memories whose strength fell.
    pub decayed: usize,
    /// Memories that left the live set.
    pub swept: usize,
    /// Groups of scratch found saying the same thing.
    pub clusters: usize,
    /// Claims that crossed into the project's own memory.
    pub promoted: Vec<String>,
    /// Claims that reinforced something the project already held.
    pub reinforced: usize,
    /// Whether anything was written.
    pub dry_run: bool,
}

impl Consolidated {
    /// Whether the pass found anything to do.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.decayed == 0 && self.swept == 0 && self.promoted.is_empty() && self.reinforced == 0
    }
}

/// Run one cycle.
///
/// The order matters and is cortex's: decay before promotion, so a claim is judged on what it
/// is worth now rather than on what it was worth when it was learned.
pub fn consolidate(
    store: &mut Store,
    scratch: Option<&mut memo_store::Scratchpad>,
    settings: &Settings,
    scope: &ScopeId,
    now: Timestamp,
    dry_run: bool,
) -> Result<Consolidated, DistilError> {
    let mut report = Consolidated {
        dry_run,
        ..Consolidated::default()
    };

    // Decay first, so a claim is judged on what it is worth now. Sweeping is held back to the
    // end: archiving here would take away the very scratch the cycle is about to look at for
    // corroboration, and the promotion it was going to make would silently never happen.
    let faded = if dry_run {
        store.decay_preview(now)?
    } else {
        store.weaken(now)?
    };
    report.decayed = faded.weakened.len();

    // Where the scratch is. A host that keeps each run in its own file has to be asked across
    // all of them; one that keeps everything in the project's store answers from a query.
    let mut scratch = scratch;
    let groups = match scratch.as_mut() {
        Some(pad) => {
            if !dry_run {
                report.decayed += pad.weaken_all(now)?;
            }
            pad.recurring(
                scope.as_str(),
                DISTINCT_SESSIONS,
                now - HORIZON,
                RUNS_PER_PASS,
            )?
        }
        None => store.scratch_clusters(scope.as_str(), DISTINCT_SESSIONS)?,
    };

    for group in groups {
        report.clusters += 1;
        if dry_run {
            report.promoted.push(group.text);
            continue;
        }
        match promote(store, settings, scope, &group, now)? {
            Landing::Added(_) | Landing::Superseded { .. } => report.promoted.push(group.text),
            Landing::Reinforced(_) => report.reinforced += 1,
        }
    }

    // Only now. Everything left has had its chance at the ladder.
    if !dry_run {
        report.swept = store.sweep(now)?.swept.len();
        if let Some(pad) = scratch.as_mut() {
            report.swept += pad.sweep_all(now)?;
        }
    }
    Ok(report)
}

/// Carry one recurring claim into the project's own memory.
///
/// One witness per distinct session, which is what makes diversity count: the confidence a
/// promoted claim lands with is a function of how many unrelated runs saw it, not of how loudly
/// any one of them said it.
fn promote(
    store: &mut Store,
    settings: &Settings,
    scope: &ScopeId,
    group: &Cluster,
    now: Timestamp,
) -> Result<Landing, DistilError> {
    let mut memory = Memory::new(
        mint(now),
        Tier::Fact,
        scope.clone(),
        Body::note(&group.text, NoteKind::Claim),
        now,
    );
    // Dated from when it was first seen, not from when the pass ran. A thing learned in March
    // and confirmed in August did not become true this afternoon.
    memory.temporal = memo_model::Temporal::recalled(now, group.first_seen);
    memory.session = group.sessions.first().cloned();
    memory.provenance = memo_model::Provenance {
        through: memo_model::Through::Sleep,
        who: None,
    };

    let first = witness_for(group, 0, scope, settings, now);
    let landing = store.remember(memory, first, now)?;

    // Every other session that saw it is its own witness. Attaching them one at a time is what
    // makes `memo why` able to print the argument rather than a number.
    for (index, _) in group.sessions.iter().enumerate().skip(1) {
        store.attach(
            landing.id(),
            witness_for(group, index, scope, settings, now),
            now,
        )?;
    }
    Ok(landing)
}

/// One session's testimony for a recurring claim.
fn witness_for(
    group: &Cluster,
    index: usize,
    scope: &ScopeId,
    settings: &Settings,
    now: Timestamp,
) -> Witness {
    let session = group
        .sessions
        .get(index)
        .cloned()
        .unwrap_or_else(|| SessionId::new("unknown"));
    let mut witness = Witness::new(
        WitnessId::new(format!("callus-{}-{}", &group.hash[..8], session)),
        WitnessKind::Repetition,
        session,
        scope.clone(),
        group.first_seen.min(now),
    )
    .noted("recurred across sessions (rules, not a model)");
    witness.weight = settings.weight(WitnessKind::Repetition);
    witness
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corroboration_takes_two_unrelated_runs() {
        // One session repeating a thing is a person being emphatic. Raising this would make a
        // memory layer that learns nothing from a project worked on twice.
        assert_eq!(DISTINCT_SESSIONS, 2);
    }

    #[test]
    fn a_pass_that_did_nothing_says_so() {
        assert!(Consolidated::default().is_empty());
        let did = Consolidated {
            promoted: vec!["a thing".into()],
            ..Consolidated::default()
        };
        assert!(!did.is_empty());
    }
}
