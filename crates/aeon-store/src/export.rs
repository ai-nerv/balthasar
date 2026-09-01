//! Turning the ledger into something a policy could be trained on.
//!
//! Explicit and never automatic. aeon does not start training jobs, does not phone anywhere,
//! and does not accumulate a dataset in the background — this runs when somebody asks for it,
//! and writes where they say.
//!
//! **Nothing here carries content.** A row is features and outcomes: how a candidate scored,
//! how it was presented, what happened next. No query text, no memory text, no tool arguments,
//! no secrets. That is not a courtesy — a training set is the artefact most likely to leave the
//! machine, so it is the one that must hold the least.

use crate::{Store, StoreError};
use aeon_model::Timestamp;
use rusqlite::params;

/// One decision, as a policy would see it.
///
/// Deliberately flat and numeric. Anything a model needs to learn from is here as a number; a
/// feature that could only be expressed as text is a feature that would carry content with it.
/// Deserialisable as well as serialisable: `aeon dataset` writes these and `aeon train` reads
/// them back, which is what lets a dataset be inspected, edited, or fitted on a different
/// machine from the one that produced it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Row {
    /// Which search, so rows from one decision stay together.
    pub recall: String,
    /// A digest of the query. Groups repeats; reveals nothing.
    pub query_hash: String,
    /// The settings in force, so rows from different configurations are separable.
    pub config: String,
    /// Where this candidate placed.
    pub rank: usize,
    /// Whether it was returned.
    pub selected: bool,
    /// What it scored.
    pub score: f64,
    /// The per-signal breakdown.
    pub semantic: f64,
    /// See [`Row::semantic`].
    pub lexical: f64,
    /// See [`Row::semantic`].
    pub entity: f64,
    /// See [`Row::semantic`].
    pub frecency: f64,
    /// See [`Row::semantic`].
    pub confidence: f64,
    /// See [`Row::semantic`].
    pub strength: f64,
    /// See [`Row::semantic`].
    pub scope_signal: f64,
    /// Whether vectors were available to the search that produced this.
    pub vectors: bool,
    /// How it was shown, when it was shown at all.
    pub presentation: Option<String>,
    /// How the action that used it went, when anybody said.
    pub outcome: Option<String>,
    /// How sure we are it had anything to do with that outcome.
    pub attribution: Option<String>,
    /// How many pieces of evidence the memory had.
    pub witnesses: usize,
    /// How many distinct sources those came from — the trust summary, without the sources.
    pub domains: usize,
    /// When.
    pub at: Timestamp,
}

impl Store {
    /// Every scored decision the ledger holds, as training rows.
    ///
    /// One row per candidate per search, joined to whatever happened afterwards. A candidate
    /// nobody acted on still produces a row: "was returned and nothing followed" is a label,
    /// and a dataset containing only the successes teaches a model that everything works.
    pub fn training_rows(&self, limit: usize) -> Result<Vec<Row>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT r.id, r.query_hash, r.config_fingerprint, r.vector_available, r.requested_at, \
                    c.memory_id, c.rank, c.selected, c.score, c.semantic, c.lexical, c.entity, \
                    c.frecency, c.confidence, c.strength, c.scope_signal, \
                    im.presentation_mode, o.kind, a.attribution_kind \
             FROM recall_candidate c \
             JOIN recall_run r ON r.id = c.recall_id \
             LEFT JOIN injection i ON i.recall_id = r.id \
             LEFT JOIN injection_memory im \
                    ON im.injection_id = i.id AND im.memory_id = c.memory_id \
             LEFT JOIN action_memory am ON am.memory_id = c.memory_id \
             LEFT JOIN action_use a ON a.id = am.action_id AND a.injection_id = i.id \
             LEFT JOIN outcome o ON o.action_id = a.id \
             ORDER BY r.requested_at DESC, c.rank \
             LIMIT ?1",
        )?;

        let rows = statement
            .query_map(params![limit as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)? != 0,
                    r.get::<_, Timestamp>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)? != 0,
                    r.get::<_, f64>(8)?,
                    [
                        r.get::<_, f64>(9)?,
                        r.get::<_, f64>(10)?,
                        r.get::<_, f64>(11)?,
                        r.get::<_, f64>(12)?,
                        r.get::<_, f64>(13)?,
                        r.get::<_, f64>(14)?,
                        r.get::<_, f64>(15)?,
                    ],
                    r.get::<_, Option<String>>(16)?,
                    r.get::<_, Option<String>>(17)?,
                    r.get::<_, Option<String>>(18)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut out = Vec::with_capacity(rows.len());
        for (
            recall,
            query_hash,
            config,
            vectors,
            at,
            memory,
            rank,
            selected,
            score,
            signals,
            presentation,
            outcome,
            attribution,
        ) in rows
        {
            // The trust summary: how much evidence, and from how many places. The places
            // themselves stay in the store — a dataset that named sources would be a map of
            // what somebody had read.
            let id = aeon_model::MemoryId::new(memory);
            let witnesses = self.witnesses_of(&id)?;
            let domains: std::collections::BTreeSet<String> = witnesses
                .iter()
                .map(aeon_model::Witness::domain_of)
                .collect();

            out.push(Row {
                recall,
                query_hash,
                config,
                rank: rank.max(0) as usize,
                selected,
                score,
                semantic: signals[0],
                lexical: signals[1],
                entity: signals[2],
                frecency: signals[3],
                confidence: signals[4],
                strength: signals[5],
                scope_signal: signals[6],
                vectors,
                presentation,
                outcome,
                attribution,
                witnesses: witnesses.len(),
                domains: domains.len(),
                at,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aeon_model::{
        Attribution, Body, Memory, NoteKind, OutcomeKind, Presentation, ScopeId, SessionId, Tier,
        Witness, WitnessId, WitnessKind,
    };

    const NOW: Timestamp = 1_756_000_000;
    const SECRET: &str = "the token is hunter2-never-share-this";

    /// A store with one search, one injection, one action and one outcome.
    fn a_full_trail() -> (Store, aeon_model::MemoryId) {
        let mut store = Store::ephemeral().expect("store");
        let held = Memory::new(
            crate::mint(NOW),
            Tier::Fact,
            ScopeId::new("/w/thing"),
            Body::note(SECRET, NoteKind::Claim),
            NOW,
        );
        let id = held.id.clone();
        store
            .remember(
                held,
                Witness::new(
                    WitnessId::new("w1"),
                    WitnessKind::Imperative,
                    SessionId::new("01RUN"),
                    ScopeId::new("/w/thing"),
                    NOW,
                ),
                NOW,
            )
            .expect("remember");

        store
            .note_recall(
                &crate::RecallRun {
                    id: "r1".to_owned(),
                    scope: ScopeId::new("/w/thing"),
                    session: Some(SessionId::new("01RUN")),
                    query_hash: "abc123".to_owned(),
                    requested_at: NOW,
                    config_fingerprint: "cfg".to_owned(),
                    vector_available: false,
                    result_limit: 10,
                    latency_us: 200,
                },
                &[crate::Candidate {
                    memory: id.clone(),
                    rank: 0,
                    selected: true,
                    score: 0.8,
                    signals: crate::Signals {
                        lexical: 0.6,
                        ..crate::Signals::default()
                    },
                }],
            )
            .expect("recall");
        store
            .note_injection(
                &crate::Injection {
                    id: "i1".to_owned(),
                    recall: Some("r1".to_owned()),
                    session: Some(SessionId::new("01RUN")),
                    created_at: NOW,
                    token_count: 12,
                    remote: false,
                    policy: "balanced".to_owned(),
                },
                &[(id.clone(), Presentation::Asserted)],
            )
            .expect("inject");
        store
            .note_use(&crate::Use {
                id: "a1".to_owned(),
                injection: Some("i1".to_owned()),
                session: Some(SessionId::new("01RUN")),
                reported_at: NOW + 10,
                tool: Some("shell".to_owned()),
                action_hash: "hashed".to_owned(),
                attribution: Attribution::Explicit,
                memories: vec![id.clone()],
            })
            .expect("use");
        store
            .note_outcome(&crate::Verdict {
                id: "o1".to_owned(),
                action: "a1".to_owned(),
                observed_at: NOW + 20,
                kind: OutcomeKind::Succeeded,
                score: None,
                evidence_cursor: None,
                evaluator: "caller".to_owned(),
                note: None,
            })
            .expect("outcome");
        (store, id)
    }

    #[test]
    fn a_decision_comes_back_with_what_followed_it() {
        let (store, _) = a_full_trail();
        let rows = store.training_rows(100).expect("export");

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert!(row.selected);
        assert_eq!(row.presentation.as_deref(), Some("asserted"));
        assert_eq!(row.outcome.as_deref(), Some("succeeded"));
        assert_eq!(row.attribution.as_deref(), Some("explicit"));
    }

    #[test]
    fn the_dataset_carries_no_content_at_all() {
        // The artefact most likely to leave the machine is the one that must hold the least.
        let (store, _) = a_full_trail();
        let rows = store.training_rows(100).expect("export");
        let json = serde_json::to_string(&rows).expect("serialize");

        assert!(
            !json.contains("hunter2"),
            "the memory's text is in the dataset"
        );
        assert!(!json.contains("never-share"), "{json}");
        assert!(!json.contains("shell"), "a tool name leaked");
        assert_eq!(rows[0].query_hash, "abc123", "the query is a digest");
    }

    #[test]
    fn the_trust_summary_counts_sources_without_naming_them() {
        // A dataset that named its sources would be a map of what somebody had read.
        let (store, _) = a_full_trail();
        let rows = store.training_rows(100).expect("export");

        assert_eq!(rows[0].witnesses, 1);
        assert_eq!(rows[0].domains, 1);
        let json = serde_json::to_string(&rows).expect("serialize");
        assert!(
            !json.contains("session:"),
            "a domain identifier leaked: {json}"
        );
    }

    #[test]
    fn a_candidate_nobody_acted_on_is_still_a_row() {
        // A dataset containing only the successes teaches a model that everything works.
        let mut store = Store::ephemeral().expect("store");
        store
            .note_recall(
                &crate::RecallRun {
                    id: "r1".to_owned(),
                    scope: ScopeId::new("/w/thing"),
                    session: None,
                    query_hash: "abc".to_owned(),
                    requested_at: NOW,
                    config_fingerprint: "cfg".to_owned(),
                    vector_available: false,
                    result_limit: 10,
                    latency_us: 100,
                },
                &[crate::Candidate {
                    memory: aeon_model::MemoryId::new("never-used"),
                    rank: 3,
                    selected: false,
                    score: 0.1,
                    signals: crate::Signals::default(),
                }],
            )
            .expect("recall");

        let rows = store.training_rows(100).expect("export");
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].selected);
        assert_eq!(rows[0].outcome, None, "unknown, not failed");
    }

    #[test]
    fn an_empty_ledger_exports_nothing_rather_than_failing() {
        let store = Store::ephemeral().expect("store");
        assert!(store.training_rows(100).expect("export").is_empty());
    }
}
