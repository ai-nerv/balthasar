//! Sleep: what falls out of an idle machine.
//!
//! * **CALLUS** — something that recurred in unrelated runs.
//! * **SLEEP** — the pass itself: decay first, so a promotion is judged on current strength,
//!   then cluster, then promote, then sweep what is spent.

use crate::DistilError;
use balthasar_lua::Settings;
use balthasar_model::{
    Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
};
use balthasar_store::{Landing, Store, mint};

/// How many distinct sessions must have seen something before it belongs to the project.
pub const DISTINCT_SESSIONS: usize = 2;

/// How far back a consolidation pass looks for scratch worth corroborating.
pub const HORIZON: balthasar_model::Timestamp = 30 * 24 * 60 * 60;

/// How many runs one consolidation pass will open, newest first.
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
/// Decay runs before promotion, so a claim is judged on what it is worth now.
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

    let faded = if dry_run {
        store.decay_preview(now)?
    } else {
        store.weaken(now)?
    };
    report.decayed = faded.weakened.len();

    let mut scratch = scratch;
    // Every claim, not only the ones already repeated word for word: two runs wording one thing
    // differently are two clusters of one session each.
    // Both places scratch lives — the run's own file, and claims a pass held one witness short.
    let mut found = store.scratch_clusters(scope.as_str(), 1)?;
    if let Some(pad) = scratch.as_mut() {
        if !dry_run {
            report.decayed += pad.weaken_all(now)?;
        }
        found.extend(pad.recurring(scope.as_str(), 1, now - HORIZON, RUNS_PER_PASS)?);
    }

    // Fold the rewordings together, and only then ask which claims are corroborated.
    let groups = crate::merge(found)
        .into_iter()
        .map(one_witness_per_session)
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

    // Swept only now: everything left has had its chance at the ladder, and sweeping earlier
    // would take away the scratch this cycle reads for corroboration.
    if !dry_run {
        report.swept = store.sweep(now)?.swept.len();
        if let Some(pad) = scratch.as_mut() {
            report.swept += pad.sweep_all(now)?;
        }
    }
    Ok(report)
}

/// One session, one witness, however many agents of it said the thing.
///
/// A run's scratch is per agent, so one claim reaches this from as many files as the run had
/// subagents; CALLUS counts unrelated runs.
fn one_witness_per_session(mut akin: crate::Akin) -> crate::Akin {
    let mut distinct: Vec<SessionId> = Vec::with_capacity(akin.cluster.sessions.len());
    for session in akin.cluster.sessions.drain(..) {
        if !distinct.contains(&session) {
            distinct.push(session);
        }
    }
    akin.cluster.sessions = distinct;
    akin
}

/// Carry one recurring claim into the project's own memory, one witness per distinct session.
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
    // Dated from when it was first seen, not from when the pass ran.
    memory.temporal = balthasar_model::Temporal::recalled(now, group.first_seen);
    memory.session = group.sessions.first().cloned();
    memory.provenance = balthasar_model::Provenance {
        through: balthasar_model::Through::Sleep,
        who: None,
    };

    // What it was made from, recorded rather than inferred: a purge of the scratch would
    // otherwise let the next pass write the claim back.
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
    // Said in the note: "recurred" and "recurred, reworded" are different strengths of evidence.
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
        assert_eq!(DISTINCT_SESSIONS, 2);
    }

    fn cluster(sessions: &[&str]) -> crate::Akin {
        crate::Akin {
            cluster: balthasar_store::Cluster {
                sources: Vec::new(),
                text: "we always use make".to_owned(),
                hash: balthasar_model::content_hash("we always use make"),
                sessions: sessions.iter().map(|s| SessionId::new(*s)).collect(),
                first_seen: 0,
            },
            near: false,
        }
    }

    #[test]
    fn five_agents_of_one_run_are_one_witness() {
        // Scratch is per agent, so one claim arrives from as many files as the run had subagents.
        let one = one_witness_per_session(cluster(&["01ONE", "01ONE", "01ONE", "01ONE", "01ONE"]));
        assert_eq!(one.cluster.sessions.len(), 1);
        assert!(
            one.cluster.sessions.len() < DISTINCT_SESSIONS,
            "and one run is not corroboration"
        );
    }

    #[test]
    fn two_runs_still_corroborate_however_many_agents_they_had() {
        let two = one_witness_per_session(cluster(&["01ONE", "01TWO", "01ONE"]));
        assert_eq!(
            two.cluster.sessions,
            vec![SessionId::new("01ONE"), SessionId::new("01TWO")],
            "in the order they were first seen"
        );
        assert!(two.cluster.sessions.len() >= DISTINCT_SESSIONS);
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
