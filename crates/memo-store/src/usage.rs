//! The use-and-outcome ledger.
//!
//! Four separate records — retrieved, injected, used, evaluated — rather than one counter,
//! because they answer different questions and only the last is evidence about a memory's
//! worth. Retrieving something ten times says a query matched it ten times, which is a fact
//! about queries.
//!
//! Three rules hold in every function below.
//!
//! **No row here is a witness.** Nothing in this module can change a confidence, and there is a
//! test that holds the whole store to it. Truth and utility are independent judgments: a fact
//! can be perfectly true and harmful to inject.
//!
//! **Content stays out.** Queries and actions arrive already hashed. The ledger records that
//! something happened and where in the transcript to look, never a copy of what was said.
//!
//! **Silence is not failure.** An action nobody evaluated is `unknown` for good. Nothing here
//! turns an unreported outcome into a bad one after a timeout.

use crate::{Store, StoreError};
use memo_model::{Attribution, MemoryId, OutcomeKind, Presentation, ScopeId, SessionId, Timestamp};
use rusqlite::params;

/// One search, as the ledger knows it.
#[derive(Debug, Clone)]
pub struct RecallRun {
    /// This search.
    pub id: String,
    /// Which project.
    pub scope: ScopeId,
    /// Which run asked, when one did.
    pub session: Option<SessionId>,
    /// A digest of the query. Never the query.
    pub query_hash: String,
    /// When.
    pub requested_at: Timestamp,
    /// The settings in force, so two searches under different weights are distinguishable.
    pub config_fingerprint: String,
    /// Whether vectors were available to it.
    pub vector_available: bool,
    /// How many it was allowed to return.
    pub result_limit: usize,
    /// How long it took.
    pub latency_us: u64,
}

/// One memory a search considered.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Which memory.
    pub memory: MemoryId,
    /// Where it placed.
    pub rank: usize,
    /// Whether it was returned rather than merely considered.
    pub selected: bool,
    /// What it scored, and where that came from.
    pub score: f64,
    /// The per-signal breakdown, in the scorer's own order.
    pub signals: Signals,
}

/// Why a candidate scored what it did.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Signals {
    /// Cosine similarity, when embeddings exist.
    pub semantic: f64,
    /// Full-text ranking.
    pub lexical: f64,
    /// Shared subject matter, rarity-weighted.
    pub entity: f64,
    /// How often and how recently it has been needed.
    pub frecency: f64,
    /// How sure.
    pub confidence: f64,
    /// How faded.
    pub strength: f64,
    /// Whether the project outranked the global store.
    pub scope: f64,
}

/// One assembled context.
#[derive(Debug, Clone)]
pub struct Injection {
    /// This context.
    pub id: String,
    /// The search it came from, when it came from one.
    pub recall: Option<String>,
    /// Which run it was handed to.
    pub session: Option<SessionId>,
    /// When.
    pub created_at: Timestamp,
    /// What it cost.
    pub token_count: usize,
    /// Whether it crossed a remote boundary.
    pub remote: bool,
    /// Which retrieval policy produced it.
    pub policy: String,
}

/// One action a caller reported.
#[derive(Debug, Clone)]
pub struct Use {
    /// This action.
    pub id: String,
    /// The context it followed, when there was one.
    pub injection: Option<String>,
    /// Which run.
    pub session: Option<SessionId>,
    /// When it was reported.
    pub reported_at: Timestamp,
    /// What did it.
    pub tool: Option<String>,
    /// A digest of the action. Never its arguments.
    pub action_hash: String,
    /// How sure we are the memories below had anything to do with it.
    pub attribution: Attribution,
    /// Which memories it used.
    pub memories: Vec<MemoryId>,
}

/// How an action turned out.
#[derive(Debug, Clone)]
pub struct Verdict {
    /// This observation.
    pub id: String,
    /// Which action.
    pub action: String,
    /// When it was observed.
    pub observed_at: Timestamp,
    /// What happened.
    pub kind: OutcomeKind,
    /// An evaluator's number, when one produced one.
    pub score: Option<f64>,
    /// Where in the transcript the evidence is.
    pub evidence_cursor: Option<u64>,
    /// Who says so — the caller, a tool's exit status, a named model.
    pub evaluator: String,
    /// A bounded label. Never a transcript excerpt.
    pub note: Option<String>,
}

impl Store {
    /// Record that a search happened, and what it considered.
    ///
    /// Written after the search rather than during it: the ledger must never be able to change
    /// what a search returns, and the simplest guarantee of that is that it does not run until
    /// the answer is already decided.
    pub fn note_recall(
        &mut self,
        run: &RecallRun,
        candidates: &[Candidate],
    ) -> Result<(), StoreError> {
        let tx = self.db_mut().transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO recall_run \
             (id, scope, session, query_hash, requested_at, config_fingerprint, \
              vector_available, result_limit, latency_us) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                run.id,
                run.scope.as_str(),
                run.session.as_ref().map(SessionId::as_str),
                run.query_hash,
                run.requested_at,
                run.config_fingerprint,
                i64::from(run.vector_available),
                run.result_limit as i64,
                run.latency_us as i64,
            ],
        )?;
        for held in candidates {
            tx.execute(
                "INSERT OR REPLACE INTO recall_candidate \
                 (recall_id, memory_id, rank, selected, score, semantic, lexical, entity, \
                  frecency, confidence, strength, scope_signal) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    run.id,
                    held.memory.as_str(),
                    held.rank as i64,
                    i64::from(held.selected),
                    held.score,
                    held.signals.semantic,
                    held.signals.lexical,
                    held.signals.entity,
                    held.signals.frecency,
                    held.signals.confidence,
                    held.signals.strength,
                    held.signals.scope,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Record that a context was assembled, and what went into it.
    pub fn note_injection(
        &mut self,
        held: &Injection,
        memories: &[(MemoryId, Presentation)],
    ) -> Result<(), StoreError> {
        let tx = self.db_mut().transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO injection \
             (id, recall_id, session, created_at, token_count, remote, policy_name) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                held.id,
                held.recall,
                held.session.as_ref().map(SessionId::as_str),
                held.created_at,
                held.token_count as i64,
                i64::from(held.remote),
                held.policy,
            ],
        )?;
        for (position, (memory, mode)) in memories.iter().enumerate() {
            tx.execute(
                "INSERT OR REPLACE INTO injection_memory \
                 (injection_id, memory_id, position, presentation_mode) VALUES (?1, ?2, ?3, ?4)",
                params![held.id, memory.as_str(), position as i64, mode.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Record that a caller acted, and which memories it says it used.
    pub fn note_use(&mut self, used: &Use) -> Result<(), StoreError> {
        // Checked before the write, so an action against a context this memo never assembled
        // comes back as a sentence rather than as a foreign-key failure. A caller reporting use
        // of an injection nobody made is confused or hostile, and either way deserves to be
        // told which.
        if let Some(injection) = &used.injection {
            let known: i64 = self.db().query_row(
                "SELECT count(*) FROM injection WHERE id = ?1",
                params![injection],
                |r| r.get(0),
            )?;
            if known == 0 {
                return Err(StoreError::Unknown(format!(
                    "injection called '{injection}' — this memo never assembled one"
                )));
            }
        }
        let tx = self.db_mut().transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO action_use \
             (id, injection_id, session, reported_at, tool, action_hash, attribution_kind) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                used.id,
                used.injection,
                used.session.as_ref().map(SessionId::as_str),
                used.reported_at,
                used.tool,
                used.action_hash,
                used.attribution.as_str(),
            ],
        )?;
        for memory in &used.memories {
            tx.execute(
                "INSERT OR IGNORE INTO action_memory (action_id, memory_id) VALUES (?1, ?2)",
                params![used.id, memory.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Record how an action turned out.
    ///
    /// Idempotent by id, so a caller that replays its event log does not double-count. Updating
    /// an existing verdict is allowed — an outcome genuinely can be revised when more is
    /// learned — but the id has to be the same one, so a second opinion cannot masquerade as
    /// independent corroboration.
    pub fn note_outcome(&mut self, said: &Verdict) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT INTO outcome \
             (id, action_id, observed_at, kind, score, evidence_cursor, evaluator, note) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(id) DO UPDATE SET \
               observed_at = excluded.observed_at, kind = excluded.kind, \
               score = excluded.score, evidence_cursor = excluded.evidence_cursor, \
               evaluator = excluded.evaluator, note = excluded.note",
            params![
                said.id,
                said.action,
                said.observed_at,
                said.kind.as_str(),
                said.score,
                said.evidence_cursor.map(|c| c as i64),
                said.evaluator,
                said.note,
            ],
        )?;
        Ok(())
    }

    /// What the ledger adds up to for one memory.
    ///
    /// Derived on every call rather than kept as a column. The counts are cheap, and a stored
    /// summary is a thing that can drift from the observations it claims to summarise.
    ///
    /// Only countable attributions move `verified_*`. Proximal evidence — the memory happened
    /// to be in the context — is kept apart, because "was on screen" is not "was used", and a
    /// memory that gained authority from the former would gain it from being popular.
    pub fn utility_of(&self, memory: &MemoryId) -> Result<memo_model::Utility, StoreError> {
        let mut held = memo_model::Utility::default();

        let mut statement = self.db().prepare(
            "SELECT a.attribution_kind, o.kind, o.observed_at \
             FROM action_memory m \
             JOIN action_use a ON a.id = m.action_id \
             LEFT JOIN outcome o ON o.action_id = a.id \
             WHERE m.memory_id = ?1",
        )?;
        let rows = statement
            .query_map(params![memory.as_str()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<Timestamp>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        for (how, what, at) in rows {
            let how: Attribution = how.parse().unwrap_or(Attribution::Proximal);
            let what: OutcomeKind = what
                .as_deref()
                .and_then(|k| k.parse().ok())
                .unwrap_or(OutcomeKind::Unknown);

            if !how.is_countable() {
                held.proximal += 1;
                continue;
            }
            match what {
                k if k.is_helpful() => {
                    held.verified_helpful += 1;
                    held.last_verified_at =
                        Some(held.last_verified_at.map_or(at.unwrap_or_default(), |was| {
                            was.max(at.unwrap_or_default())
                        }));
                }
                k if k.is_harmful() => {
                    held.verified_harmful += 1;
                    held.last_verified_at =
                        Some(held.last_verified_at.map_or(at.unwrap_or_default(), |was| {
                            was.max(at.unwrap_or_default())
                        }));
                }
                OutcomeKind::Ignored => held.ignored += 1,
                _ => held.unknown += 1,
            }
        }

        // Injected, never reported against. Distinct from used-and-unevaluated, and the larger
        // number in practice: most callers never report at all.
        let shown: i64 = self.db().query_row(
            "SELECT count(*) FROM injection_memory i \
             WHERE i.memory_id = ?1 \
               AND NOT EXISTS (SELECT 1 FROM action_use a \
                               JOIN action_memory m ON m.action_id = a.id \
                               WHERE a.injection_id = i.injection_id AND m.memory_id = i.memory_id)",
            params![memory.as_str()],
            |r| r.get(0),
        )?;
        held.unknown += shown.max(0) as usize;

        Ok(held)
    }

    /// How often a memory has merely been retrieved.
    ///
    /// Reported beside utility rather than as part of it, because the gap between the two is
    /// the thing worth seeing: a memory retrieved forty times with no attributed outcome is
    /// popular, and popularity is not evidence.
    pub fn times_retrieved(&self, memory: &MemoryId) -> Result<(usize, usize), StoreError> {
        let held: (i64, i64) = self.db().query_row(
            "SELECT count(*), coalesce(sum(selected), 0) FROM recall_candidate WHERE memory_id = ?1",
            params![memory.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((held.0.max(0) as usize, held.1.max(0) as usize))
    }

    /// The chain behind one recall: what it considered, what it returned, and what followed.
    pub fn trace_of(&self, recall: &str) -> Result<Option<Trace>, StoreError> {
        let run = self
            .db()
            .query_row(
                "SELECT scope, query_hash, requested_at, result_limit, latency_us \
                 FROM recall_run WHERE id = ?1",
                params![recall],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Timestamp>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .ok();
        let Some((scope, query_hash, requested_at, limit, latency_us)) = run else {
            return Ok(None);
        };

        let mut statement = self.db().prepare(
            "SELECT memory_id, rank, selected, score FROM recall_candidate \
             WHERE recall_id = ?1 ORDER BY rank",
        )?;
        let considered = statement
            .query_map(params![recall], |r| {
                Ok((
                    MemoryId::new(r.get::<_, String>(0)?),
                    r.get::<_, i64>(1)? as usize,
                    r.get::<_, i64>(2)? != 0,
                    r.get::<_, f64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut statement = self.db().prepare(
            "SELECT a.id, a.tool, a.attribution_kind, coalesce(o.kind, 'unknown') \
             FROM injection i \
             JOIN action_use a ON a.injection_id = i.id \
             LEFT JOIN outcome o ON o.action_id = a.id \
             WHERE i.recall_id = ?1 ORDER BY a.reported_at",
        )?;
        let actions = statement
            .query_map(params![recall], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|(id, tool, how, what)| TracedAction {
                id,
                tool,
                attribution: how.parse().unwrap_or(Attribution::Proximal),
                outcome: what.parse().unwrap_or(OutcomeKind::Unknown),
            })
            .collect();

        Ok(Some(Trace {
            recall: recall.to_owned(),
            scope: ScopeId::new(scope),
            query_hash,
            requested_at,
            result_limit: limit.max(0) as usize,
            latency_us: latency_us.max(0) as u64,
            considered,
            actions,
        }))
    }

    /// Which memories one injection carried, in the order they were placed.
    ///
    /// What structural attribution starts from: an action can only have followed something it
    /// was actually given.
    pub fn injected_in(&self, injection: &str) -> Result<Vec<MemoryId>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT memory_id FROM injection_memory WHERE injection_id = ?1 ORDER BY position",
        )?;
        let rows = statement
            .query_map(params![injection], |r| {
                Ok(MemoryId::new(r.get::<_, String>(0)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// One action, as the ledger recorded it.
    pub fn use_of(&self, action: &str) -> Result<Option<Use>, StoreError> {
        let held = self
            .db()
            .query_row(
                "SELECT injection_id, session, reported_at, tool, action_hash, attribution_kind \
                 FROM action_use WHERE id = ?1",
                params![action],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Timestamp>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                },
            )
            .ok();
        let Some((injection, session, reported_at, tool, action_hash, how)) = held else {
            return Ok(None);
        };

        let mut statement = self
            .db()
            .prepare("SELECT memory_id FROM action_memory WHERE action_id = ?1")?;
        let memories = statement
            .query_map(params![action], |r| {
                Ok(MemoryId::new(r.get::<_, String>(0)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Some(Use {
            id: action.to_owned(),
            injection,
            session: session.map(SessionId::new),
            reported_at,
            tool,
            action_hash,
            attribution: how.parse().unwrap_or(Attribution::Proximal),
            memories,
        }))
    }

    /// How one action was judged, if anybody said.
    pub fn outcome_of(&self, action: &str) -> Result<Option<Verdict>, StoreError> {
        Ok(self
            .db()
            .query_row(
                "SELECT id, observed_at, kind, score, evidence_cursor, evaluator, note \
                 FROM outcome WHERE action_id = ?1 ORDER BY observed_at DESC LIMIT 1",
                params![action],
                |r| {
                    Ok(Verdict {
                        id: r.get(0)?,
                        action: action.to_owned(),
                        observed_at: r.get(1)?,
                        kind: r
                            .get::<_, String>(2)?
                            .parse()
                            .unwrap_or(OutcomeKind::Unknown),
                        score: r.get(3)?,
                        evidence_cursor: r.get::<_, Option<i64>>(4)?.map(|c| c as u64),
                        evaluator: r.get(5)?,
                        note: r.get(6)?,
                    })
                },
            )
            .ok())
    }

    /// Every action a session reported, newest first.
    pub fn uses_in(&self, session: &SessionId, limit: usize) -> Result<Vec<Use>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT id FROM action_use WHERE session = ?1 ORDER BY reported_at DESC LIMIT ?2",
        )?;
        let ids = statement
            .query_map(params![session.as_str(), limit as i64], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(held) = self.use_of(&id)? {
                out.push(held);
            }
        }
        Ok(out)
    }
    /// Forget ledger rows older than `before`.
    ///
    /// The one place in this crate outside `purge` that removes rows, and it is a different
    /// kind of removal: the ledger is bounded telemetry with a retention policy, not memory. No
    /// memory, witness, transcript turn or confidence is touched, and a store whose whole
    /// ledger is dropped still knows everything it believes — it has only forgotten how the
    /// believing went.
    ///
    /// Ordered so that no row is orphaned mid-way: outcomes before actions, actions before
    /// injections, and both before the recalls they point at.
    pub fn forget_ledger_before(&mut self, before: Timestamp) -> Result<usize, StoreError> {
        let tx = self.db_mut().transaction()?;
        let mut gone = 0;
        gone += tx.execute(
            "DELETE FROM outcome WHERE action_id IN \
             (SELECT id FROM action_use WHERE reported_at < ?1)",
            params![before],
        )?;
        gone += tx.execute(
            "DELETE FROM action_memory WHERE action_id IN \
             (SELECT id FROM action_use WHERE reported_at < ?1)",
            params![before],
        )?;
        gone += tx.execute(
            "DELETE FROM action_use WHERE reported_at < ?1",
            params![before],
        )?;
        gone += tx.execute(
            "DELETE FROM injection_memory WHERE injection_id IN \
             (SELECT id FROM injection WHERE created_at < ?1)",
            params![before],
        )?;
        gone += tx.execute(
            "DELETE FROM injection WHERE created_at < ?1",
            params![before],
        )?;
        gone += tx.execute(
            "DELETE FROM recall_candidate WHERE recall_id IN \
             (SELECT id FROM recall_run WHERE requested_at < ?1)",
            params![before],
        )?;
        gone += tx.execute(
            "DELETE FROM recall_run WHERE requested_at < ?1",
            params![before],
        )?;
        tx.commit()?;
        Ok(gone)
    }
}

/// One recall, and everything that followed from it.
#[derive(Debug, Clone)]
pub struct Trace {
    /// Which search.
    pub recall: String,
    /// Which project.
    pub scope: ScopeId,
    /// A digest of what was asked.
    pub query_hash: String,
    /// When.
    pub requested_at: Timestamp,
    /// How many it was allowed to return.
    pub result_limit: usize,
    /// How long it took.
    pub latency_us: u64,
    /// Every memory considered: id, rank, whether returned, score.
    pub considered: Vec<(MemoryId, usize, bool, f64)>,
    /// Every action that followed.
    pub actions: Vec<TracedAction>,
}

/// One action in a trace.
#[derive(Debug, Clone)]
pub struct TracedAction {
    /// Which action.
    pub id: String,
    /// What did it.
    pub tool: Option<String>,
    /// How sure we are it used what was injected.
    pub attribution: Attribution,
    /// How it went.
    pub outcome: OutcomeKind,
}
