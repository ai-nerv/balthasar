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
use balthasar_lua::Settings;
use balthasar_model::{
    Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
};
use balthasar_store::{Landing, Store, mint};

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
pub const HORIZON: balthasar_model::Timestamp = 30 * 24 * 60 * 60;

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
    scratch: Option<&mut balthasar_store::Scratchpad>,
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
    // Every claim, not only the ones already repeated word for word. Two runs wording one thing
    // differently are two clusters of one session each, and asking the store for corroborated
    // clusters would drop both before anything could notice they agree.
    // Both places scratch lives, always. A run's own file holds what it observed; the project
    // store holds what a pass *held* — a claim one witness short of the floor, waiting for the
    // second. Reading only the run files made everything held invisible to the thing whose job
    // is to corroborate it.
    let mut found = store.scratch_clusters(scope.as_str(), 1)?;
    if let Some(pad) = scratch.as_mut() {
        if !dry_run {
            report.decayed += pad.weaken_all(now)?;
        }
        found.extend(pad.recurring(scope.as_str(), 1, now - HORIZON, RUNS_PER_PASS)?);
    }

    // Fold the rewordings together, and only then ask which claims are corroborated. The bar is
    // unchanged — what changed is that a claim gets to arrive at it in more than one wording.
    let groups = crate::merge(found)
        .into_iter()
        .filter(|akin| akin.cluster.sessions.len() >= DISTINCT_SESSIONS);

    for akin in groups {
        report.clusters += 1;
        if dry_run {
            report.promoted.push(akin.cluster.text);
            continue;
        }
        match promote(store, settings, scope, &akin, now)? {
            Landing::Added(_) | Landing::Superseded { .. } => {
                report.promoted.push(akin.cluster.text);
            }
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
    akin: &crate::Akin,
    now: Timestamp,
) -> Result<Landing, DistilError> {
    let group = &akin.cluster;
    let mut memory = Memory::new(
        mint(now),
        Tier::Fact,
        scope.clone(),
        Body::note(&group.text, NoteKind::Claim),
        now,
    );
    // Dated from when it was first seen, not from when the pass ran. A thing learned in March
    // and confirmed in August did not become true this afternoon.
    memory.temporal = balthasar_model::Temporal::recalled(now, group.first_seen);
    memory.session = group.sessions.first().cloned();
    memory.provenance = balthasar_model::Provenance {
        through: balthasar_model::Through::Sleep,
        who: None,
    };

    // What it was made from, recorded rather than inferred. Without this a purge of the scratch
    // leaves the fact standing with nothing pointing at where it came from — and the next pass
    // writes the claim back out of a record that was supposed to be gone.
    memory.links = group
        .sources
        .iter()
        .map(|source| balthasar_model::Link {
            to: source.clone(),
            rel: balthasar_model::LinkRelation::DerivedFrom,
            at: now,
        })
        .collect();

    let first = witness_for(akin, 0, scope, settings, now);
    let landing = store.remember(memory, first, now)?;

    // Every other session that saw it is its own witness. Attaching them one at a time is what
    // makes `balthasar why` able to print the argument rather than a number.
    for (index, _) in group.sessions.iter().enumerate().skip(1) {
        store.attach(
            landing.id(),
            witness_for(akin, index, scope, settings, now),
            now,
        )?;
    }
    Ok(landing)
}

/// One session's testimony for a recurring claim.
fn witness_for(
    akin: &crate::Akin,
    index: usize,
    scope: &ScopeId,
    settings: &Settings,
    now: Timestamp,
) -> Witness {
    let group = &akin.cluster;
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
    // Said in the note, because it is the part a person might disagree with. "Recurred" and
    // "was said in different words and judged to be the same claim" are different strengths of
    // evidence, and `balthasar why` has to be able to tell them apart.
    .noted(if akin.near {
        "recurred across sessions, reworded (rules, not a model)"
    } else {
        "recurred across sessions (rules, not a model)"
    });
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
