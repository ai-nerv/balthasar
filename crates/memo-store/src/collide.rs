//! What a claim collides with, before anything is written.
//!
//! Three questions, asked in order, and the order is the design. Does the store already hold
//! this claim word for word? Does it hold the same claim in other words? Does it hold the same
//! claim with a *different value*, which is a revision rather than agreement?
//!
//! Getting the second and third the wrong way round would be the expensive mistake: a
//! restatement that superseded would throw away the evidence for what it agreed with, and a
//! revision that reinforced would leave a project asserting a value nobody still holds.

use crate::{Store, StoreError, row};
use memo_model::{Memory, MemoryId};
use rusqlite::{OptionalExtension, params};

impl Store {
    /// The live memory this one would collide with, if any.
    ///
    /// A fact collides on its slot; anything else collides on saying the same thing. Both are
    /// exact, both are free, and both are checked before a model is ever consulted.
    pub(crate) fn standing(&self, memory: &Memory) -> Result<Option<MemoryId>, StoreError> {
        let (subject, predicate, _) = row::slot(&memory.body);
        let found: Option<String> = match (subject, predicate) {
            // A single-valued slot collides on the slot: a new answer supersedes the old one.
            // A many-valued one does not — `likes sushi` and `likes pizza` are both true, so
            // it collides only on saying the same thing twice.
            (Some(subject), Some(predicate)) if memo_model::is_single_valued(&predicate) => self
                .db()
                .query_row(
                    "SELECT id FROM memory \
                     WHERE scope = ?1 AND subject = ?2 AND predicate = ?3 \
                       AND tier = 'fact' AND valid_to IS NULL AND archived_at IS NULL",
                    params![memory.scope.as_str(), subject, predicate],
                    |r| r.get(0),
                )
                .optional()?,
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
                } else if let Some(restated) = self.restated_by(memory)? {
                    // The same claim in other words is the same claim. Without this a project
                    // told "we use make test" in one run and "run make test instead" in the
                    // next held two beliefs with one witness each, and neither ever reached the
                    // confidence two witnesses would have given the one.
                    Some(restated)
                } else {
                    // Saying the same thing with a different value is a revision of it. Only
                    // durable claims, and only against other claims that share an opening —
                    // the index makes that a lookup rather than a scan.
                    self.revised_by(memory)?
                }
            }
        };
        Ok(found.map(MemoryId::new))
    }

    /// The live claim this one restates, if it restates anything.
    ///
    /// Candidates come from the full-text index, so the cost is a bounded lookup rather than a
    /// scan, and [`memo_model::same_claim`] settles it — which means a claim beside its own
    /// *replacement* is refused here and left to [`Store::revised_by`], where it belongs.
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
            .find(|(_, held)| memo_model::same_claim(held, &text))
            .map(|(id, _)| id))
    }

    /// The live claim this one revises, if it revises anything.
    ///
    /// Bucketed on the opening two words, so the cost does not scale with the store, and
    /// settled by the word-prefix rule rather than by the bucket.
    fn revised_by(&self, memory: &Memory) -> Result<Option<String>, StoreError> {
        if memory.tier != memo_model::Tier::Fact {
            return Ok(None);
        }
        let text = memory.text();
        let lead = memo_model::lead(&text);
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
            .find(|(_, held)| memo_model::same_claim_different_value(held, &text))
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
        // Word for word, or the same claim in other words. The second is what makes a
        // restatement reinforce what is already held instead of superseding it — and getting
        // this wrong would be worse than not matching at all, because it would replace a belief
        // with an identical one and throw away the evidence for the original.
        Ok(hash == arriving.content_hash || memo_model::same_claim(&text, &arriving.text()))
    }
}
