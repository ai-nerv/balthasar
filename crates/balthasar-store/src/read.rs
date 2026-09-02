//! Getting something out.
//!
//! Two stages, because it is a nineteen-fold difference and because it is the only way
//! commitment 3 survives: full-text search prefilters, and scoring ranks what it found.
//! Vectors, when they arrive at M4, become another term in the same sum — never a replacement
//! for this path, and never a full scan of an embedding column.

use crate::score::{Scored, Weights, cosine, coverage, frecency, fts_query, relative, terms_of};
use crate::{Store, StoreError, row};
use balthasar_model::{Link, Memory, MemoryId, Privacy, Tier, Timestamp, Witness};
use rusqlite::{OptionalExtension, params};

/// What a recall was asked for.
#[derive(Debug, Clone)]
pub struct Recall {
    /// What to look for. Empty means "whatever is most worth showing".
    pub query: String,
    /// How many to answer with.
    pub limit: usize,
    /// Only these tiers, or every durable one when empty.
    pub tiers: Vec<Tier>,
    /// Confidence a memory must reach to come back at all.
    ///
    /// The *live* floor, not the injection floor. A memory below assertion is still findable,
    /// and the gap between those two numbers is the whole answer to staleness.
    pub floor: f64,
    /// Whether to look in the archive too. Only worth it when the live results are weak.
    pub include_archived: bool,
    /// Whether the results are bound for a remote model.
    pub remote: bool,
    /// How much of the question a memory must actually answer to come back at all.
    ///
    /// Separate from `floor`, which is about how sure the memory is. This one is about whether
    /// it is an answer to *this* question — a confidently-held fact about the staging box is
    /// not an answer about the production box, however sure anybody is of it.
    ///
    /// Zero by default: a bare `recall` shows what it found and lets a person judge. The
    /// injection path raises it, because a model shown a near-miss will use it.
    pub relevance: f64,
    /// The moment to score against.
    pub now: Timestamp,
    /// Which project's entity index to consult.
    pub scope_name: String,
    /// How much each signal counts.
    pub weights: Weights,
    /// The query's own vector, when there is an embedder.
    pub embedding: Option<Vec<f32>>,
    /// Whether this store is the project's rather than the global one.
    pub near: bool,
    /// Whether finding a memory counts as needing it.
    ///
    /// Off by default, and the default is the point. Reinforcing on every search meant a
    /// person browsing their own store reset the strength of everything they looked at, so
    /// nothing ever faded and `balthasar decay` always reported an empty pass. Inspection is not
    /// use. The injection path turns this on, because a memory the model was actually given
    /// *was* needed.
    pub reinforce: bool,
}

impl Recall {
    /// A search for `query`, with everything else at its default.
    #[must_use]
    pub fn of(query: impl Into<String>, now: Timestamp) -> Self {
        Self {
            query: query.into(),
            limit: 10,
            tiers: Vec::new(),
            floor: balthasar_model::floor::LIVE,
            include_archived: false,
            remote: false,
            relevance: 0.0,
            now,
            scope_name: String::new(),
            weights: Weights::default().without_vectors(),
            embedding: None,
            near: false,
            reinforce: false,
        }
    }
}

/// A group of memories saying the same thing in different runs.
///
/// Built here because the query that finds them is here, and used by the ladder. Putting it the
/// other way round would have the store depending on the crate that reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Cluster {
    /// What they say.
    pub text: String,
    /// The digest they share.
    pub hash: String,
    /// Which runs said it, oldest first.
    pub sessions: Vec<balthasar_model::SessionId>,
    /// When the first of them did.
    pub first_seen: Timestamp,
    /// The memories this cluster was assembled from, when they are in the store being written.
    ///
    /// Empty when they are not. A `link` row has foreign keys into `memory`, so an id belonging
    /// to a run's own file cannot be recorded from the project's — and the honest answer to
    /// "what was this made from" is then the session, which the promoted memory already carries.
    pub sources: Vec<balthasar_model::MemoryId>,
}

/// How many candidates full-text search may hand to scoring.
///
/// A bound, so the cost of a query never scales with the size of the store. Exceeding it
/// returns fewer candidates rather than an error: an error channel would be an oracle for how
/// much the store holds.
const CANDIDATES: usize = 500;

/// Below how many lexical hits a query is treated as having missed.
///
/// Full-text search gates the candidates, so a query sharing no words with anything answers
/// nothing however good its vector is — `deployment` would not find `we deploy with fly`. When
/// the lexical stage comes back this thin, a bounded scan tops the set up and lets the vector
/// decide. Still bounded, so the cost of a miss does not scale with the store either.
const THIN: usize = 8;

impl Store {
    /// One memory, with its witnesses and links.
    pub fn get(&self, id: &MemoryId) -> Result<Option<Memory>, StoreError> {
        let found = self
            .db()
            .query_row(
                &format!("SELECT {} FROM memory WHERE id = ?1", row::COLUMNS),
                params![id.as_str()],
                |r| Ok(row::memory(r)),
            )
            .optional()?;
        let Some(memory) = found else {
            return Ok(None);
        };
        let mut memory = memory?;
        memory.witnesses = self.witnesses_of(&memory.id)?;
        memory.links = self.links_of(&memory.id)?;
        Ok(Some(memory))
    }

    /// Every piece of evidence for a memory, newest first.
    ///
    /// Newest first because that is the order `balthasar why` reads best in: what most recently
    /// convinced us, then what convinced us before that.
    pub fn witnesses_of(&self, id: &MemoryId) -> Result<Vec<Witness>, StoreError> {
        let mut statement = self
            .db()
            .prepare("SELECT * FROM witness WHERE memory = ?1 ORDER BY at DESC, id")?;
        let found = statement
            .query_map(params![id.as_str()], |r| Ok(row::witness(r)))?
            .collect::<Result<Vec<_>, _>>()?;
        found.into_iter().collect()
    }

    /// Every edge out of a memory.
    pub fn links_of(&self, id: &MemoryId) -> Result<Vec<Link>, StoreError> {
        let mut statement = self
            .db()
            .prepare("SELECT dst, rel, at FROM link WHERE src = ?1 ORDER BY at")?;
        let found = statement
            .query_map(params![id.as_str()], |r| Ok(row::link(r)))?
            .collect::<Result<Vec<_>, _>>()?;
        found.into_iter().collect()
    }

    /// The live answer to a slot, if the store has one.
    pub fn live_slot(
        &self,
        scope: &str,
        subject: &str,
        predicate: &str,
    ) -> Result<Option<Memory>, StoreError> {
        let found: Option<String> = self
            .db()
            .query_row(
                "SELECT id FROM memory WHERE scope = ?1 AND subject = ?2 AND predicate = ?3 \
                   AND tier = 'fact' AND valid_to IS NULL AND archived_at IS NULL",
                params![scope, subject, predicate],
                |r| r.get(0),
            )
            .optional()?;
        match found {
            Some(id) => self.get(&MemoryId::new(id)),
            None => Ok(None),
        }
    }

    /// What a slot said at a moment in the past.
    ///
    /// The question a validity interval exists to answer, and the reason a contradicted fact is
    /// closed rather than removed. Without it, "what was the deploy target in March" has no
    /// answer at all.
    pub fn slot_at(
        &self,
        scope: &str,
        subject: &str,
        predicate: &str,
        at: Timestamp,
    ) -> Result<Option<Memory>, StoreError> {
        let found: Option<String> = self
            .db()
            .query_row(
                "SELECT id FROM memory WHERE scope = ?1 AND subject = ?2 AND predicate = ?3 \
                   AND tier IN ('fact', 'archive') AND valid_from <= ?4 \
                   AND (valid_to IS NULL OR valid_to > ?4) \
                 ORDER BY valid_from DESC LIMIT 1",
                params![scope, subject, predicate, at],
                |r| r.get(0),
            )
            .optional()?;
        match found {
            Some(id) => self.get(&MemoryId::new(id)),
            None => Ok(None),
        }
    }

    /// Search.
    pub fn recall(&self, ask: &Recall) -> Result<Vec<Scored>, StoreError> {
        let mut candidates = if ask.query.trim().is_empty() {
            self.everything(ask)?
        } else {
            self.matching(ask)?
        };

        // What the query is *about*, whatever words it used. Deliberately able to ADD
        // candidates rather than only reorder the ones full-text search found: a boost that
        // can only reorder decides, in advance, what can never be found at all.
        let by_entity = if ask.scope_name.is_empty() {
            Vec::new()
        } else {
            self.by_entity(&ask.scope_name, &ask.query, ask.limit.max(THIN))?
        };
        if !by_entity.is_empty() {
            let held: Vec<String> = candidates
                .iter()
                .map(|(memory, _)| memory.id.to_string())
                .collect();
            for (id, _) in &by_entity {
                if held.contains(&id.to_string()) {
                    continue;
                }
                if let Some(memory) = self.get(id)? {
                    // Neutral on the lexical axis. It did not match the words, and scoring
                    // that as a failure would bury it under anything matching a stopword.
                    candidates.push((memory, 0.0));
                }
            }
        }

        // A query with a vector and no lexical foothold is the case two-stage retrieval is
        // worst at, and it is not a rare one: `deployment` shares no token with `we deploy
        // with fly`. Topping up lets the vector answer where the words could not.
        if ask.embedding.is_some() && candidates.len() < THIN {
            let seen: Vec<String> = candidates
                .iter()
                .map(|(memory, _)| memory.id.to_string())
                .collect();
            for (memory, _) in self.everything(ask)? {
                if !seen.contains(&memory.id.to_string()) {
                    // Neutral on the lexical axis: it did not match, and scoring it as a
                    // failure would bury it under anything that matched a stopword.
                    candidates.push((memory, 0.0));
                }
            }
        }

        let mut scored: Vec<Scored> = candidates
            .into_iter()
            .filter(|(memory, _)| memory.privacy.may_reach(ask.remote))
            .filter(|(memory, _)| memory.privacy != Privacy::Secret || ask.include_archived)
            .filter(|(memory, _)| memory.confidence >= ask.floor)
            .map(|(memory, lexical)| {
                let w = ask.weights;
                let entity = by_entity
                    .iter()
                    .find(|(id, _)| *id == memory.id)
                    .map_or(0.0, |(_, worth)| *worth);
                let strength = memory.strength.at(ask.now);
                let confidence = memory.confidence;
                let frecency = frecency(
                    memory.strength.access_count,
                    memory.strength.last_accessed,
                    ask.now,
                );
                let semantic = ask
                    .embedding
                    .as_deref()
                    .zip(memory.embedding.as_deref())
                    .and_then(|(query, held)| cosine(query, held));

                // An absent semantic term is neutral rather than zero, and its weight goes to
                // the lexical one. Scoring "we could not compare" as "they do not match" would
                // push everything on an unembedded store to the bottom of one axis.
                let (semantic_term, lexical_weight) = match semantic {
                    Some(value) => (w.semantic * value, w.lexical),
                    None => (0.0, w.lexical + w.semantic),
                };

                Scored {
                    score: semantic_term
                        + lexical_weight * lexical
                        + w.entity * entity
                        + w.frecency * frecency
                        + w.confidence * confidence
                        + w.strength * strength
                        + if ask.near { w.scope } else { 0.0 },
                    semantic,
                    lexical,
                    entity,
                    frecency,
                    confidence,
                    strength,
                    near: ask.near,
                    memory,
                }
            })
            .collect();

        // Answering less than the caller asked for is not answering. A bare recall lets
        // everything through and shows the breakdown; the injection path raises the bar.
        scored.retain(|hit| hit.relevance() >= ask.relevance);
        scored.sort_by(|a, b| b.score.total_cmp(&a.score));
        scored.truncate(ask.limit);
        Ok(scored)
    }

    /// Candidates from full-text search, with their lexical scores.
    ///
    /// `bm25` answers a negative number where more negative is better, which is the opposite of
    /// every other signal here, so it is mapped into `0..1` before it meets them.
    fn matching(&self, ask: &Recall) -> Result<Vec<(Memory, f64)>, StoreError> {
        let sql = format!(
            "SELECT {}, bm25(memory_fts) AS rank FROM memory_fts \
             JOIN memory ON memory.id = memory_fts.id \
             WHERE memory_fts MATCH ?1 {} {} \
             ORDER BY rank LIMIT ?2",
            row::COLUMNS,
            tier_clause(&ask.tiers),
            archive_clause(ask.include_archived),
        );
        let mut statement = self.db().prepare(&sql)?;
        let terms = fts_query(&ask.query);
        let found = statement
            .query_map(params![terms, CANDIDATES as i64], |r| {
                let rank: f64 = r.get("rank")?;
                Ok((row::memory(r), rank))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        // bm25 answers a negative number whose *magnitude* depends on how big the store is —
        // a term in every document scores near zero however well it matched. So the scale is
        // taken from this result set rather than assumed: the best match here is 1.0 and the
        // rest are a fraction of it. A lexical score is a ranking, not a measurement.
        let best = found.iter().map(|(_, rank)| *rank).fold(0.0_f64, f64::min);
        // Relative ranking alone cannot tell a weak match from a strong one: the only result
        // in a set is always the best in it, so "production box" scored exactly what "staging
        // box" did against a memory about the staging box. Coverage is the absolute half —
        // how much of what was asked the memory actually contains.
        let wanted = terms_of(&ask.query);
        found
            .into_iter()
            .map(|(memory, rank)| {
                let memory = memory?;
                let covered = coverage(&wanted, &memory.text());
                Ok((memory, relative(rank, best) * covered))
            })
            .collect()
    }

    /// Everything worth showing, when nobody asked for anything in particular.
    fn everything(&self, ask: &Recall) -> Result<Vec<(Memory, f64)>, StoreError> {
        let sql = format!(
            "SELECT {} FROM memory WHERE 1 = 1 {} {} ORDER BY confidence DESC LIMIT ?1",
            row::COLUMNS,
            tier_clause(&ask.tiers),
            archive_clause(ask.include_archived),
        );
        let mut statement = self.db().prepare(&sql)?;
        let found = statement
            .query_map(params![CANDIDATES as i64], |r| Ok(row::memory(r)))?
            .collect::<Result<Vec<_>, _>>()?;
        // No query means no lexical signal, so the term is neutral rather than zero: scoring
        // everything at zero on one axis would let strength alone decide the order.
        found.into_iter().map(|m| Ok((m?, 0.5))).collect()
    }

    /// Every memory, oldest first. What `balthasar export` walks.
    pub fn all(&self) -> Result<Vec<Memory>, StoreError> {
        let mut statement = self
            .db()
            .prepare(&format!("SELECT {} FROM memory ORDER BY id", row::COLUMNS))?;
        let found = statement
            .query_map([], |r| Ok(row::memory(r)))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(found.len());
        for memory in found {
            let mut memory = memory?;
            memory.witnesses = self.witnesses_of(&memory.id)?;
            memory.links = self.links_of(&memory.id)?;
            out.push(memory);
        }
        Ok(out)
    }

    /// Scratch saying the same thing in `at_least` different runs.
    ///
    /// The CALLUS query, and the reason it counts **sessions** rather than rows: one run
    /// repeating itself would otherwise look exactly like several runs agreeing, and the whole
    /// point of the path is that those are different things.
    ///
    /// Grouped by content hash, which is exact and free. A distiller clusters better; its
    /// absence does not stop this.
    /// The scratch memories in this store saying exactly this, by digest.
    fn saying(
        &self,
        scope: &str,
        hash: &str,
    ) -> Result<Vec<balthasar_model::MemoryId>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT id FROM memory \
             WHERE scope = ?1 AND content_hash = ?2 AND tier = 'scratch' AND archived_at IS NULL",
        )?;
        let found = statement
            .query_map(params![scope, hash], |r| {
                Ok(balthasar_model::MemoryId::new(r.get::<_, String>(0)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }

    pub fn scratch_clusters(
        &self,
        scope: &str,
        at_least: usize,
    ) -> Result<Vec<Cluster>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT content_hash, min(text), min(observed_at), count(DISTINCT session)              FROM memory              WHERE scope = ?1 AND tier = 'scratch' AND archived_at IS NULL                AND session IS NOT NULL AND text != ''              GROUP BY content_hash              HAVING count(DISTINCT session) >= ?2              ORDER BY count(DISTINCT session) DESC, min(observed_at)",
        )?;
        let groups = statement
            .query_map(params![scope, at_least as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Timestamp>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut out = Vec::with_capacity(groups.len());
        for (hash, text, first_seen) in groups {
            out.push(Cluster {
                sessions: self.sessions_saying(scope, &hash)?,
                sources: self.saying(scope, &hash)?,
                text,
                hash,
                first_seen,
            });
        }
        Ok(out)
    }

    /// Which runs said one thing, oldest first.
    fn sessions_saying(
        &self,
        scope: &str,
        hash: &str,
    ) -> Result<Vec<balthasar_model::SessionId>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT DISTINCT session FROM memory              WHERE scope = ?1 AND content_hash = ?2 AND tier = 'scratch' AND session IS NOT NULL              ORDER BY observed_at",
        )?;
        let found = statement
            .query_map(params![scope, hash], |r| {
                Ok(balthasar_model::SessionId::new(r.get::<_, String>(0)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }

    /// How many memories are in each tier.
    pub fn census(&self) -> Result<Vec<(String, i64)>, StoreError> {
        let mut statement = self
            .db()
            .prepare("SELECT tier, count(*) FROM memory GROUP BY tier ORDER BY tier")?;
        let found = statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }
}

/// `AND tier IN (…)`, or nothing when no tier was asked for.
fn tier_clause(tiers: &[Tier]) -> String {
    if tiers.is_empty() {
        return String::new();
    }
    let names: Vec<String> = tiers.iter().map(|t| format!("'{}'", t.as_str())).collect();
    format!("AND memory.tier IN ({})", names.join(", "))
}

/// Whether the archive is in scope.
fn archive_clause(include: bool) -> &'static str {
    if include {
        ""
    } else {
        "AND memory.archived_at IS NULL"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_with_no_tiers_constrains_nothing() {
        assert_eq!(tier_clause(&[]), "");
    }

    #[test]
    fn a_search_for_facts_says_so() {
        assert_eq!(tier_clause(&[Tier::Fact]), "AND memory.tier IN ('fact')");
    }

    #[test]
    fn a_persons_words_are_not_a_query_syntax() {
        // A bare `-` is an operator to FTS5 and a typo to everyone else.
        assert_eq!(fts_query("make -j8"), "\"make\" OR \"-j8\"");
    }

    #[test]
    fn a_word_that_matches_everything_is_dropped() {
        // FTS5 has no stopword list, so "the" was a term like any other — and a question
        // containing it matched every memory containing it, which is most of them.
        assert_eq!(
            fts_query("what is the deploy target"),
            "\"deploy\" OR \"target\""
        );
    }

    #[test]
    fn a_question_made_of_nothing_matches_nothing() {
        // The correct answer to "what is it" is nothing, not everything — and not an error.
        //
        // This test used to assert the sentinel contained a NUL, which encoded the defect
        // rather than the requirement: FTS5 reads a NUL inside a quoted string as the end of
        // the string, so `balthasar recall "what is it"` failed with "unterminated string" instead
        // of answering nothing. What matters is that the sentinel is a term the parser accepts
        // and the tokenizer can never produce.
        for empty in ["  ", "what is it", "how do you do that"] {
            let q = fts_query(empty);
            assert!(
                !q.contains('\u{0}'),
                "the sentinel must be parseable: {empty}"
            );
            assert_eq!(q.matches('"').count() % 2, 0, "and balanced: {empty}");
            assert!(!q.contains(" OR "), "one term, matching nothing: {empty}");
        }
    }

    #[test]
    fn a_quote_in_the_query_cannot_break_out_of_one() {
        assert_eq!(fts_query("say \"hi\""), "\"say\" OR \"hi\"");
    }

    #[test]
    fn matching_half_the_question_is_worth_half() {
        // The defect this exists for: relative ranking made the only result in a set the best
        // in it, so a weak match and a perfect one scored the same.
        let wanted = terms_of("production box");
        assert_eq!(wanted, ["production", "box"]);
        assert!((coverage(&wanted, "the staging box is at 10.0.0.7") - 0.5).abs() < 1e-9);
        assert!(
            (coverage(&terms_of("staging box"), "the staging box is at 10.0.0.7") - 1.0).abs()
                < 1e-9
        );
    }

    #[test]
    fn the_scoring_stage_agrees_with_the_stage_that_found_it() {
        // FTS5 stems, so `tests` finds a memory about `make test`. Scoring it as having
        // matched nothing meant the retrieval stage and the scoring stage disagreed, and a
        // whole category of question came back empty.
        let wanted = terms_of("tests");
        assert!(coverage(&wanted, "`make test` is what works here") > 0.9);
        assert!(coverage(&terms_of("boxes"), "the box is over there") > 0.9);
    }

    #[test]
    fn stemming_does_not_collapse_different_words() {
        assert!(coverage(&terms_of("production"), "the staging box") < 0.1);
    }

    #[test]
    fn a_question_with_nothing_to_ask_is_neutral() {
        // It did not fail to match; there was nothing to match.
        assert_eq!(coverage(&terms_of("what is it"), "anything at all"), 1.0);
    }

    #[test]
    fn the_best_match_in_a_set_scores_full_marks() {
        assert_eq!(relative(-8.0, -8.0), 1.0);
        assert!(relative(-1.0, -8.0) < 1.0);
    }

    #[test]
    fn a_tiny_bm25_magnitude_still_ranks() {
        // With two documents sharing a term, bm25 answers around -1.7e-6. Scoring that as
        // "no lexical match" threw the whole signal away on exactly the small stores a
        // person starts with.
        assert_eq!(relative(-1.69e-6, -1.69e-6), 1.0);
        assert!(relative(-1.3e-6, -1.69e-6) > 0.5);
    }

    #[test]
    fn a_set_that_matched_nothing_is_neutral_rather_than_zero() {
        assert_eq!(relative(0.0, 0.0), 0.5);
    }
}

#[cfg(test)]
mod topping_up {
    use super::*;
    use balthasar_model::{
        Body, Memory, ScopeId, SessionId, Tier, Witness, WitnessId, WitnessKind,
    };

    const NOW: Timestamp = 1_756_000_000;

    fn stored(text: &str) -> Store {
        let mut store = Store::ephemeral().expect("store");
        let memory = Memory::new(
            crate::mint(NOW),
            Tier::Fact,
            ScopeId::global(),
            Body::note(text, balthasar_model::NoteKind::Claim),
            NOW,
        );
        let witness = Witness::new(
            WitnessId::new("w"),
            WitnessKind::Imperative,
            SessionId::new("s"),
            ScopeId::global(),
            NOW,
        );
        store.remember(memory, witness, NOW).expect("remember");
        store
    }

    #[test]
    fn a_query_sharing_no_words_finds_nothing_without_a_vector() {
        // The limitation, asserted so a change to it is deliberate. Two-stage retrieval gates
        // on the lexical stage, and this query does not reach it.
        let store = stored("we deploy with fly");
        let found = store
            .recall(&Recall::of("kubernetes orchestration", NOW))
            .expect("recall");
        assert!(found.is_empty());
    }

    #[test]
    fn a_vector_can_answer_where_the_words_could_not() {
        let store = stored("we deploy with fly");
        let mut ask = Recall::of("kubernetes orchestration", NOW);
        ask.embedding = Some(vec![0.0; 8]);
        let found = store.recall(&ask).expect("recall");
        assert_eq!(found.len(), 1, "the scan topped the candidate set up");
        assert_eq!(found[0].lexical, 0.0, "and did not pretend it matched");
    }

    #[test]
    fn a_query_that_did_match_is_not_topped_up_past_its_own_answer() {
        let store = stored("we deploy with fly");
        let mut ask = Recall::of("deploy", NOW);
        ask.embedding = Some(vec![0.0; 8]);
        let found = store.recall(&ask).expect("recall");
        assert_eq!(found.len(), 1, "no duplicate from the scan");
        assert!(found[0].lexical > 0.0);
    }
}
