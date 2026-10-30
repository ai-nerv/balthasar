//! What a claim collides with, before anything is written.
//!
//! Three questions in order: does the store hold this claim word for word, the same claim in
//! other words, or the same claim with a *different value*. A restatement that superseded would
//! throw away the evidence it agreed with; a revision that reinforced would leave a project
//! asserting a value nobody still holds.

use crate::{Landing, Store, StoreError, row};
use balthasar_model::{LinkRelation, Memory, MemoryId, Timestamp, Witness};
use rusqlite::{OptionalExtension, params};

/// The most memories one dispute group is settled across, and the most passes it gets.
const DISPUTED: usize = 64;
const PASSES: usize = 16;

impl Store {
    /// The live memory this one would collide with, if any. A fact collides on its slot;
    /// anything else collides on saying the same thing.
    pub(crate) fn standing(&self, memory: &Memory) -> Result<Option<MemoryId>, StoreError> {
        let (subject, predicate, _) = row::slot(&memory.body);
        let found: Option<String> = match (subject, predicate) {
            // A single-valued slot collides on the slot. A many-valued one does not — `likes
            // sushi` and `likes pizza` are both true.
            (Some(subject), Some(predicate)) if balthasar_model::is_single_valued(&predicate) => {
                self.db()
                    .query_row(
                        "SELECT id FROM memory \
                     WHERE scope = ?1 AND subject = ?2 AND predicate = ?3 \
                       AND tier = 'fact' AND valid_to IS NULL AND archived_at IS NULL",
                        params![memory.scope.as_str(), subject, predicate],
                        |r| r.get(0),
                    )
                    .optional()?
            }
            _ => {
                // Saying exactly the same thing again is corroboration.
                let same: Option<String> = self
                    .db()
                    .query_row(
                        "SELECT id FROM memory \
                         WHERE scope = ?1 AND content_hash = ?2 AND tier = ?3 \
                           AND valid_to IS NULL AND archived_at IS NULL LIMIT 1",
                        params![
                            memory.scope.as_str(),
                            memory.content_hash,
                            row::tier_str(memory.tier)
                        ],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(same) = same {
                    Some(same)
                } else if memory.tier == balthasar_model::Tier::Habit {
                    // The habit whose step just failed is the one this revises.
                    self.undone_by(memory)?
                } else if let Some(restated) = self.restated_by(memory)? {
                    // The same claim in other words is the same claim.
                    Some(restated)
                } else {
                    // Saying the same thing with a different value is a revision. Only durable
                    // claims, and only against claims that share an opening, which is indexed.
                    self.revised_by(memory)?
                }
            }
        };
        Ok(found.map(MemoryId::new))
    }

    /// What a sentence would revise, if it were written down. The read-only half of the revision
    /// check, for a caller holding words rather than a memory.
    pub fn what_this_revises(
        &self,
        scope: &balthasar_model::ScopeId,
        text: &str,
    ) -> Result<Option<(MemoryId, String)>, StoreError> {
        let lead = balthasar_model::lead(text);
        if lead.is_empty() {
            return Ok(None);
        }
        let mut statement = self.db().prepare(
            "SELECT id, text FROM memory \
             WHERE scope = ?1 AND tier = 'fact' AND lead = ?2 \
               AND valid_to IS NULL AND archived_at IS NULL \
             ORDER BY observed_at DESC LIMIT 8",
        )?;
        let held = statement
            .query_map(params![scope.as_str(), lead], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(held
            .into_iter()
            .find(|(_, held)| balthasar_model::same_claim_different_value(held, text))
            .map(|(id, held)| (MemoryId::new(id), held)))
    }

    /// The live claim this one restates, if it restates anything.
    ///
    /// Candidates come from the full-text index, so the cost is a bounded lookup. A claim beside
    /// its own *replacement* is refused here and left to [`Store::revised_by`].
    fn restated_by(&self, memory: &Memory) -> Result<Option<String>, StoreError> {
        let text = memory.text();
        let mut statement = self.db().prepare(
            "SELECT memory.id, memory.text FROM memory_fts \
             JOIN memory ON memory.id = memory_fts.id \
             WHERE memory_fts MATCH ?1 AND memory.scope = ?2 AND memory.tier = ?3 \
               AND memory.valid_to IS NULL AND memory.archived_at IS NULL \
             LIMIT 12",
        )?;
        let held = statement
            .query_map(
                params![
                    crate::score::fts_query(&text),
                    memory.scope.as_str(),
                    row::tier_str(memory.tier)
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(held
            .into_iter()
            .find(|(_, held)| balthasar_model::same_claim(held, &text))
            .map(|(id, _)| id))
    }

    /// Score every memory in `id`'s dispute group again until no number moves, and answer `id`'s
    /// own score, when it has a group.
    pub(crate) fn settle_disputes(
        &mut self,
        id: &MemoryId,
        now: Timestamp,
    ) -> Result<Option<f64>, StoreError> {
        let mut group = vec![id.clone()];
        let mut next = 0;
        while let Some(here) = group.get(next).cloned() {
            next += 1;
            let mut statement = self.db().prepare(
                "SELECT dst FROM link WHERE src = ?1 AND rel = 'contradicts' \
                 UNION SELECT src FROM link WHERE dst = ?1 AND rel = 'contradicts' ORDER BY 1",
            )?;
            let near = statement
                .query_map(params![here.as_str()], |r| {
                    Ok(MemoryId::new(r.get::<_, String>(0)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for other in near {
                if group.len() < DISPUTED && !group.contains(&other) {
                    group.push(other);
                }
            }
        }
        if group.len() == 1 {
            return Ok(None);
        }
        let mut score = None;
        for _ in 0..PASSES {
            let mut moved = false;
            for member in &group {
                let before: f64 = self.db().query_row(
                    "SELECT confidence FROM memory WHERE id = ?1",
                    params![member.as_str()],
                    |r| r.get(0),
                )?;
                let after = self.rescore_one(member, now)?;
                moved |= (before - after).abs() > 1e-12;
                if member == id {
                    score = Some(after);
                }
            }
            if !moved {
                break;
            }
        }
        Ok(score)
    }

    /// The open habit whose step names the command this habit's trigger says failed.
    fn undone_by(&self, memory: &Memory) -> Result<Option<String>, StoreError> {
        let balthasar_model::Body::Habit { trigger, .. } = &memory.body else {
            return Ok(None);
        };
        let Some(failed) = trigger.split('`').nth(1).filter(|c| !c.is_empty()) else {
            return Ok(None);
        };
        let mut statement = self.db().prepare(
            "SELECT id, body FROM memory WHERE scope = ?1 AND tier = 'habit' \
               AND valid_to IS NULL AND archived_at IS NULL \
             ORDER BY observed_at DESC, id LIMIT 64",
        )?;
        let held = statement
            .query_map(params![memory.scope.as_str()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(held
            .into_iter()
            .find(|(_, body)| {
                serde_json::from_str::<balthasar_model::Body>(body).is_ok_and(|body| {
                    matches!(&body, balthasar_model::Body::Habit { steps, .. }
                        if steps.iter().any(|step| step == failed))
                })
            })
            .map(|(id, _)| id))
    }

    /// The live claim this one revises, if it revises anything. Bucketed on the opening two
    /// words, and settled by the word-prefix rule rather than by the bucket.
    fn revised_by(&self, memory: &Memory) -> Result<Option<String>, StoreError> {
        if memory.tier != balthasar_model::Tier::Fact {
            return Ok(None);
        }
        let text = memory.text();
        let lead = balthasar_model::lead(&text);
        if lead.is_empty() {
            return Ok(None);
        }

        let mut statement = self.db().prepare(
            "SELECT id, text FROM memory \
             WHERE scope = ?1 AND tier = 'fact' AND lead = ?2 \
               AND valid_to IS NULL AND archived_at IS NULL \
             ORDER BY observed_at DESC LIMIT 8",
        )?;
        let held = statement
            .query_map(params![memory.scope.as_str(), lead], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(held
            .into_iter()
            .find(|(_, held)| balthasar_model::same_claim_different_value(held, &text))
            .map(|(id, _)| id))
    }

    /// Whether what is already there says the same thing as what arrived.
    pub(crate) fn says_the_same(
        &self,
        existing: &MemoryId,
        arriving: &Memory,
    ) -> Result<bool, StoreError> {
        let (hash, text): (String, String) = self.db().query_row(
            "SELECT content_hash, text FROM memory WHERE id = ?1",
            params![existing.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        // Word for word, or the same claim in other words. The second is what makes a restatement
        // reinforce what is already held instead of superseding it; a habit is its trigger, so
        // only its own words restate it.
        Ok(hash == arriving.content_hash
            || arriving.tier != balthasar_model::Tier::Habit
                && balthasar_model::same_claim(&text, &arriving.text()))
    }

    /// Write a claim that disputes `held` instead of replacing it: both stay open, each counts
    /// against the other until a person settles them. A restatement of something held
    /// reinforces it as [`Store::remember`] would.
    pub fn dispute(
        &mut self,
        held: &MemoryId,
        memory: Memory,
        witness: Witness,
        now: Timestamp,
    ) -> Result<Landing, StoreError> {
        if let Some(existing) = self.standing(&memory)?
            && existing != *held
            && self.says_the_same(&existing, &memory)?
        {
            self.attach(&existing, witness, now)?;
            return Ok(Landing::Reinforced(existing));
        }
        let id = self.insert(memory, &witness, now)?;
        self.link(&id, held, LinkRelation::Contradicts, now)?;
        self.link(held, &id, LinkRelation::Contradicts, now)?;
        self.rescore(&id, now)?;
        self.rescore(held, now)?;
        Ok(Landing::Added(id))
    }

    /// Write a claim that replaces `held` by name: its interval closes and this one stands in.
    pub fn replace(
        &mut self,
        held: &MemoryId,
        memory: Memory,
        witness: Witness,
        now: Timestamp,
    ) -> Result<Landing, StoreError> {
        self.supersede(held, memory, witness, now)
    }
}
