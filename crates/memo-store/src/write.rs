//! Putting something in, and the one decision that involves.
//!
//! A memory arriving is not simply an insert. The store already holds an answer to the slot, or
//! the same sentence under a different id, or nothing at all, and those are three different
//! outcomes. Deciding between them is R-Mem's add/update/none, done deterministically for the
//! case that does not need a model: an exact slot match, or an exact content hash.
//!
//! Nothing here deletes. A superseded fact has its interval closed and an edge drawn to what
//! replaced it, and stays exactly where it was.

use crate::{Store, StoreError, row};
use memo_model::{
    Contradiction, LinkRelation, Memory, MemoryId, Timestamp, Witness, confidence_of,
};
use rusqlite::{OptionalExtension, params};

/// What happened to something offered to the store.
#[derive(Debug, Clone, PartialEq)]
pub enum Landing {
    /// Nothing like it was here. It is now.
    Added(MemoryId),
    /// The store already said this. The evidence for it grew.
    Reinforced(MemoryId),
    /// The store said something else about the same slot. That answer's interval is closed,
    /// this one stands, and an edge runs between them.
    Superseded {
        /// What was true until now.
        was: MemoryId,
        /// What is true now.
        now: MemoryId,
    },
}

impl Landing {
    /// The memory that is live afterwards, whichever way it went.
    #[must_use]
    pub fn id(&self) -> &MemoryId {
        match self {
            Self::Added(id) | Self::Reinforced(id) => id,
            Self::Superseded { now, .. } => now,
        }
    }
}

impl Store {
    /// Offer a memory to the store, and let it decide what that means.
    ///
    /// `witness` is the evidence for it. There is no way to write a durable memory without
    /// evidence, because a fact that cannot answer "how do you know" is the thing this whole
    /// design exists to prevent.
    pub fn remember(
        &mut self,
        memory: Memory,
        witness: Witness,
        now: Timestamp,
    ) -> Result<Landing, StoreError> {
        if let Some(existing) = self.standing(&memory)? {
            if self.says_the_same(&existing, &memory)? {
                self.attach(&existing, witness, now)?;
                return Ok(Landing::Reinforced(existing));
            }
            let landed = self.supersede(&existing, memory, witness, now)?;
            return Ok(landed);
        }
        let id = self.insert(memory, &witness, now)?;
        Ok(Landing::Added(id))
    }

    /// Write a memory with nothing attached. See [`Store::keep_scratch`].
    fn insert_unwitnessed(
        &mut self,
        memory: Memory,
        now: Timestamp,
    ) -> Result<MemoryId, StoreError> {
        self.write_row(memory, None, now)
    }

    /// Write a memory and its first witness.
    fn insert(
        &mut self,
        mut memory: Memory,
        witness: &Witness,
        now: Timestamp,
    ) -> Result<MemoryId, StoreError> {
        memory.witnesses.push(witness.clone());
        memory.rescore(&[], now);
        self.write_row(memory, Some(witness.clone()), now)
    }

    /// Write one row, and its first witness when there is one.
    fn write_row(
        &mut self,
        memory: Memory,
        witness: Option<Witness>,
        _now: Timestamp,
    ) -> Result<MemoryId, StoreError> {
        let (subject, predicate, object) = row::slot(&memory.body);
        // Whether the slot holds one current answer or an accumulating set. Written as a
        // column because the partial unique index keys on it, and an index cannot call a
        // function.
        let single = predicate
            .as_deref()
            .is_none_or(memo_model::is_single_valued);
        // A claim with no slot still has an opening. It is the bucket a later revision of the
        // same claim is looked up in.
        let lead = (memory.tier == memo_model::Tier::Fact && predicate.is_none())
            .then(|| memo_model::lead(&memory.text()))
            .filter(|l| !l.is_empty());
        let body = serde_json::to_string(&memory.body)?;
        let text = memory.text();
        let tx = self.db_mut().transaction()?;
        tx.execute(
            "INSERT INTO memory \
             (id, tier, scope, session, subject, predicate, object, body, text, content_hash, \
              observed_at, happened_at, valid_from, valid_to, \
              importance, strength, last_accessed, access_count, pinned, \
              confidence, privacy, through, who, archived_at, single_valued, lead) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
                     ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
            params![
                memory.id.as_str(),
                row::tier_str(memory.tier),
                memory.scope.as_str(),
                memory.session.as_ref().map(|s| s.as_str()),
                subject,
                predicate,
                object,
                body,
                text,
                memory.content_hash,
                memory.temporal.observed_at,
                memory.temporal.happened_at,
                memory.temporal.valid_from,
                memory.temporal.valid_to,
                row::importance_str(memory.strength.importance),
                memory.strength.value,
                memory.strength.last_accessed,
                memory.strength.access_count,
                i64::from(memory.strength.pinned),
                memory.confidence,
                row::privacy_str(memory.privacy),
                row::through_str(memory.provenance.through),
                memory.provenance.who,
                memory.archived_at,
                i64::from(single),
                lead,
            ],
        )?;
        tx.execute(
            "INSERT INTO memory_fts (id, text, subject, predicate, object) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![memory.id.as_str(), text, subject, predicate, object],
        )?;
        if let Some(witness) = &witness {
            write_witness(&tx, &memory.id, witness)?;
        }
        for link in &memory.links {
            write_link(&tx, &memory.id, link.to.as_str(), link.rel, link.at)?;
        }
        tx.commit()?;

        // What the memory is about. After the row, because the entity table points at it.
        let scope = memory.scope.to_string();
        self.index_entities(&memory.id, &scope, &text)?;
        Ok(memory.id)
    }

    /// Add evidence to something already held, and recompute what it is worth.
    pub fn attach(
        &mut self,
        id: &MemoryId,
        witness: Witness,
        now: Timestamp,
    ) -> Result<f64, StoreError> {
        {
            let tx = self.db_mut().transaction()?;
            write_witness(&tx, id, &witness)?;
            tx.commit()?;
        }
        self.rescore(id, now)
    }

    /// Recompute a memory's confidence from what the store currently holds about it.
    ///
    /// Called after every change to the evidence, because a record whose number and whose
    /// argument disagree is the one state `memo why` must never be able to show.
    pub fn rescore(&mut self, id: &MemoryId, now: Timestamp) -> Result<f64, StoreError> {
        let witnesses = self.witnesses_of(id)?;
        let against = self.contradictions_of(id)?;
        let (superseded, pinned): (bool, bool) = self.db().query_row(
            "SELECT valid_to IS NOT NULL, pinned FROM memory WHERE id = ?1",
            params![id.as_str()],
            |r| Ok((r.get::<_, i64>(0)? != 0, r.get::<_, i64>(1)? != 0)),
        )?;
        let score = confidence_of(&witnesses, &against, superseded, pinned, now);
        self.db().execute(
            "UPDATE memory SET confidence = ?2 WHERE id = ?1",
            params![id.as_str(), score],
        )?;
        Ok(score)
    }

    /// Close one answer's interval and stand another in its place.
    ///
    /// The new memory inherits the old one's confidence as a floor. Being a correction of
    /// something established is itself evidence about the correction, and starting a
    /// replacement from zero is why systems oscillate between a stale fact and its fix.
    fn supersede(
        &mut self,
        old: &MemoryId,
        new: Memory,
        witness: Witness,
        now: Timestamp,
    ) -> Result<Landing, StoreError> {
        let was: f64 = self.db().query_row(
            "SELECT confidence FROM memory WHERE id = ?1",
            params![old.as_str()],
            |r| r.get(0),
        )?;
        // The interval closes first: the partial unique index will refuse the replacement
        // while the old answer is still live, which is exactly the protection it exists for.
        self.db().execute(
            "UPDATE memory SET valid_to = ?2 WHERE id = ?1 AND valid_to IS NULL",
            params![old.as_str(), now],
        )?;
        let new_id = self.insert(new, &witness, now)?;

        {
            let tx = self.db_mut().transaction()?;
            write_link(&tx, &new_id, old.as_str(), LinkRelation::Supersedes, now)?;
            write_link(&tx, old, new_id.as_str(), LinkRelation::Contradicts, now)?;
            tx.commit()?;
        }

        let earned = self.rescore(&new_id, now)?;
        if was > earned {
            self.db().execute(
                "UPDATE memory SET confidence = ?2 WHERE id = ?1",
                params![new_id.as_str(), was],
            )?;
        }
        self.rescore(old, now)?;

        Ok(Landing::Superseded {
            was: old.clone(),
            now: new_id,
        })
    }

    /// Move a memory out of the live set, keeping everything about it.
    pub fn archive(&mut self, id: &MemoryId, now: Timestamp) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE memory SET archived_at = ?2, tier = 'archive' \
             WHERE id = ?1 AND archived_at IS NULL",
            params![id.as_str(), now],
        )?;
        Ok(())
    }

    /// Note that a memory was recalled: reinforce it, so what is needed stops fading.
    pub fn touch(&mut self, id: &MemoryId, now: Timestamp) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE memory SET strength = 1.0, last_accessed = ?2, \
             access_count = access_count + 1 WHERE id = ?1",
            params![id.as_str(), now],
        )?;
        Ok(())
    }

    /// Attach a vector to a memory.
    ///
    /// `model` is stored beside it because a model change invalidates every vector: comparing a
    /// `bge-small` embedding against a `bge-base` one produces a number, and the number is
    /// meaningless. Keeping the name is what lets `doctor` notice and `reindex` fix it.
    pub fn embed(&mut self, id: &MemoryId, vector: &[f32], model: &str) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE memory SET embedding = ?2, embed_model = ?3 WHERE id = ?1",
            params![id.as_str(), row::blob(vector), model],
        )?;
        Ok(())
    }

    /// Memories with no vector, or one from a different model, oldest first.
    ///
    /// What `reindex` walks. Bounded, because embedding is done in batches and a caller that
    /// asked for everything would hold the whole store in memory to do it.
    pub fn unembedded(
        &self,
        model: &str,
        limit: usize,
    ) -> Result<Vec<(MemoryId, String)>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT id, text FROM memory              WHERE archived_at IS NULL AND (embedding IS NULL OR embed_model IS NOT ?1)              ORDER BY id LIMIT ?2",
        )?;
        let found = statement
            .query_map(params![model, limit as i64], |r| {
                Ok((
                    MemoryId::new(r.get::<_, String>(0)?),
                    r.get::<_, String>(1)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }

    /// Write a scratch memory with no evidence attached.
    ///
    /// The one place a memory is written without a witness, and it is allowed because scratch
    /// is not asserted to anybody. An observation is a record of something said, not a claim
    /// that it is true; the ladder is what turns one into the other, and that is where the
    /// evidence arrives.
    ///
    /// Answers **the id that actually holds the text**, which is not always the one that came
    /// in: a session that says the same thing twice gets one row and two references to it.
    /// Returning the caller's id regardless left the ledger pointing at a memory that was
    /// never written, and the foreign key caught it one call later.
    pub fn keep_scratch(&mut self, memory: Memory) -> Result<MemoryId, StoreError> {
        if memory.tier.must_be_witnessed() {
            return Err(StoreError::Foreign(format!(
                "a {} may not be written without evidence",
                memory.tier
            )));
        }
        // Scoped to the session, not just the store. Scratch is one run's own, and two
        // sessions that happen to say the same sentence must not end up sharing a row whose
        // `session` names only the first of them.
        if let Some(held) = self.scratch_saying(&memory)? {
            return Ok(held);
        }
        let now = memory.temporal.observed_at;
        self.insert_unwitnessed(memory, now)
    }

    /// Draw an asserted edge between two memories that are already here.
    ///
    /// Links are normally set on a memory before it is written, which is right when the edge is
    /// known at write time. This is for the edges that are not — a derivation noticed later, or
    /// a relationship a person asserts about two things already believed.
    pub fn link(
        &mut self,
        src: &MemoryId,
        dst: &MemoryId,
        rel: memo_model::LinkRelation,
        at: Timestamp,
    ) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT OR IGNORE INTO link (src, rel, dst, at) VALUES (?1, ?2, ?3, ?4)",
            params![src.as_str(), row::relation_str(rel), dst.as_str(), at],
        )?;
        Ok(())
    }

    /// The scratch memory a session holds for this text, if any.
    ///
    /// How a turn finds its own memory. The transcript holds no memory id — it is a separate
    /// file and a reference across the two could dangle — so the text is the join, which is
    /// also what `keep_scratch` deduplicates on.
    pub fn scratch_for(
        &self,
        scope: &memo_model::ScopeId,
        session: &memo_model::SessionId,
        text: &str,
    ) -> Result<Option<MemoryId>, StoreError> {
        let probe = Memory::new(
            MemoryId::new("probe"),
            memo_model::Tier::Scratch,
            scope.clone(),
            memo_model::Body::note(text, memo_model::NoteKind::Observation),
            0,
        );
        let found: Option<String> = self
            .db()
            .query_row(
                "SELECT id FROM memory \
                 WHERE scope = ?1 AND session = ?2 AND content_hash = ?3 AND tier = 'scratch' \
                 LIMIT 1",
                params![scope.as_str(), session.as_str(), probe.content_hash],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.map(MemoryId::new))
    }

    /// The scratch memory this session already has for this text, if any.
    fn scratch_saying(&self, memory: &Memory) -> Result<Option<MemoryId>, StoreError> {
        let Some(session) = &memory.session else {
            return Ok(None);
        };
        let found: Option<String> = self
            .db()
            .query_row(
                "SELECT id FROM memory \
                 WHERE scope = ?1 AND session = ?2 AND content_hash = ?3 AND tier = ?4 \
                 LIMIT 1",
                params![
                    memory.scope.as_str(),
                    session.as_str(),
                    memory.content_hash,
                    row::tier_str(memory.tier)
                ],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.map(MemoryId::new))
    }

    /// Set how far a memory may travel.
    ///
    /// A decision rather than a derivation, like pinning: nothing about a memory's content says
    /// whether its owner wants it leaving the machine.
    pub fn db_privacy(
        &mut self,
        id: &MemoryId,
        privacy: memo_model::Privacy,
    ) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE memory SET privacy = ?2 WHERE id = ?1",
            params![id.as_str(), privacy.as_str()],
        )?;
        Ok(())
    }

    /// Pin or unpin, which is a decision rather than a derivation.
    pub fn pin(&mut self, id: &MemoryId, pinned: bool, now: Timestamp) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE memory SET pinned = ?2 WHERE id = ?1",
            params![id.as_str(), i64::from(pinned)],
        )?;
        self.rescore(id, now)?;
        Ok(())
    }

    /// What is pulling against this memory, and how sure each of them is.
    fn contradictions_of(&self, id: &MemoryId) -> Result<Vec<Contradiction>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT m.confidence FROM link l JOIN memory m ON m.id = l.src \
             WHERE l.dst = ?1 AND l.rel = 'contradicts' AND m.archived_at IS NULL \
               AND m.valid_to IS NULL",
        )?;
        let found = statement
            .query_map(params![id.as_str()], |r| {
                Ok(Contradiction {
                    confidence: r.get(0)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }
}

/// One witness row. Idempotent by id, so re-ingesting the same transcript adds nothing.
fn write_witness(
    tx: &rusqlite::Transaction<'_>,
    memory: &MemoryId,
    witness: &Witness,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT OR IGNORE INTO witness \
         (id, memory, kind, session, scope, at, cursor, weight, note, channel, domain) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            witness.id.as_str(),
            memory.as_str(),
            witness.kind.as_str(),
            witness.session.as_str(),
            witness.scope.as_str(),
            witness.at,
            witness.cursor.map(|c| c as i64),
            witness.weight,
            witness.note,
            witness.channel.as_str(),
            witness.domain.as_ref().map(memo_model::Domain::as_str),
        ],
    )?;
    Ok(())
}

/// One edge. Idempotent, because a relation drawn twice is one relation.
fn write_link(
    tx: &rusqlite::Transaction<'_>,
    src: &MemoryId,
    dst: &str,
    rel: LinkRelation,
    at: Timestamp,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT OR IGNORE INTO link (src, rel, dst, at) VALUES (?1, ?2, ?3, ?4)",
        params![src.as_str(), row::relation_str(rel), dst, at],
    )?;
    Ok(())
}

impl Store {
    /// Whether this file has already been read by this extractor at this version.
    ///
    /// The reason a re-run is cheap and a better extractor is not: bumping the version makes
    /// every stamp stale, so old material is read again without anybody remembering to say so.
    pub fn already_read(
        &self,
        source: &str,
        reference: &str,
        version: i64,
    ) -> Result<bool, StoreError> {
        let found: Option<i64> = self
            .db()
            .query_row(
                "SELECT version FROM stamp WHERE source = ?1 AND ref = ?2 AND extractor = 'rules'",
                params![source, reference],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.is_some_and(|held| held >= version))
    }

    /// Record that this file has been read.
    pub fn stamp(
        &mut self,
        source: &str,
        reference: &str,
        version: i64,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT INTO stamp (source, ref, extractor, version, at) \
             VALUES (?1, ?2, 'rules', ?3, ?4) \
             ON CONFLICT(source, ref, extractor) DO UPDATE SET version = ?3, at = ?4",
            params![source, reference, version, now],
        )?;
        Ok(())
    }
}

impl Store {
    /// Carry a memory out of the session that made it and into the project's own.
    ///
    /// The manual rung on the ladder. Everything else that crosses does so because something
    /// was observed — a thing recurred, a command was repaired, a person typed an instruction.
    /// This is the case none of those cover: a person looking at what a session holds and
    /// saying *keep that one*.
    ///
    /// Idempotent. Promoting something already promoted attaches the witness and leaves the
    /// tier alone, so saying it twice is agreeing with yourself rather than an error.
    pub fn promote(
        &mut self,
        id: &MemoryId,
        into: memo_model::Tier,
        witness: Witness,
        now: Timestamp,
    ) -> Result<f64, StoreError> {
        if !into.is_durable() {
            return Err(StoreError::Foreign(format!(
                "{into} does not outlive the session that made it"
            )));
        }
        // The session stays on the row. A durable memory belongs to the project, and which run
        // learned it is part of what `memo why` has to be able to say.
        self.db().execute(
            "UPDATE memory SET tier = ?2, archived_at = NULL WHERE id = ?1",
            params![id.as_str(), row::tier_str(into)],
        )?;
        self.attach(id, witness, now)
    }

    /// Everything a session holds that has not crossed into the project's memory.
    ///
    /// What `memo promote` is choosing from, and what dies with the session if nobody does.
    pub fn uncrossed(&self, session: &memo_model::SessionId) -> Result<Vec<Memory>, StoreError> {
        let mut statement = self.db().prepare(&format!(
            "SELECT {} FROM memory \
             WHERE session = ?1 AND tier = 'scratch' AND archived_at IS NULL \
             ORDER BY observed_at, id",
            row::COLUMNS
        ))?;
        let found = statement
            .query_map(params![session.as_str()], |r| Ok(row::memory(r)))?
            .collect::<Result<Vec<_>, _>>()?;
        found.into_iter().collect()
    }
}

#[cfg(test)]
mod promoting {
    use super::*;
    use crate::Store;
    use crate::mint::mint;
    use memo_model::{Body, NoteKind, ScopeId, SessionId, Tier, WitnessId, WitnessKind};

    const NOW: Timestamp = 1_756_000_000;

    fn scratch(store: &mut Store, text: &str) -> MemoryId {
        let mut memory = Memory::new(
            mint(NOW),
            Tier::Scratch,
            ScopeId::new("/w/thing"),
            Body::note(text, NoteKind::Observation),
            NOW,
        );
        memory.session = Some(SessionId::new("0831-yt8z"));
        store.keep_scratch(memory).expect("scratch")
    }

    fn asked_for() -> Witness {
        Witness::new(
            WitnessId::new("said-so"),
            WitnessKind::Imperative,
            SessionId::new("cli"),
            ScopeId::new("/w/thing"),
            NOW,
        )
    }

    #[test]
    fn what_a_person_keeps_becomes_the_projects() {
        let mut store = Store::ephemeral().expect("store");
        let id = scratch(&mut store, "the deploy target is fly.io");

        let confidence = store
            .promote(&id, Tier::Fact, asked_for(), NOW)
            .expect("promote");
        let held = store.get(&id).expect("get").expect("there");

        assert_eq!(held.tier, Tier::Fact);
        assert!(confidence > memo_model::floor::INJECT, "{confidence}");
        assert!(held.is_assertable(memo_model::floor::INJECT, NOW, false));
    }

    #[test]
    fn it_still_says_which_run_learned_it() {
        // A durable memory belongs to the project; which session learned it is part of what
        // `memo why` has to be able to answer.
        let mut store = Store::ephemeral().expect("store");
        let id = scratch(&mut store, "the deploy target is fly.io");
        store
            .promote(&id, Tier::Fact, asked_for(), NOW)
            .expect("promote");

        let held = store.get(&id).expect("get").expect("there");
        assert_eq!(
            held.session.as_ref().map(ToString::to_string).as_deref(),
            Some("0831-yt8z")
        );
    }

    #[test]
    fn saying_it_twice_is_agreeing_with_yourself() {
        let mut store = Store::ephemeral().expect("store");
        let id = scratch(&mut store, "the deploy target is fly.io");
        store
            .promote(&id, Tier::Fact, asked_for(), NOW)
            .expect("once");
        store
            .promote(
                &id,
                Tier::Fact,
                Witness::new(
                    WitnessId::new("said-so-again"),
                    WitnessKind::Imperative,
                    SessionId::new("cli"),
                    ScopeId::new("/w/thing"),
                    NOW,
                ),
                NOW,
            )
            .expect("twice");

        let held = store.get(&id).expect("get").expect("there");
        assert_eq!(held.tier, Tier::Fact);
        assert_eq!(held.witnesses.len(), 2);
    }

    #[test]
    fn a_tier_that_dies_with_the_session_is_not_a_promotion() {
        let mut store = Store::ephemeral().expect("store");
        let id = scratch(&mut store, "a thing");
        assert!(store.promote(&id, Tier::Scratch, asked_for(), NOW).is_err());
        assert!(store.promote(&id, Tier::Archive, asked_for(), NOW).is_err());
    }

    #[test]
    fn what_a_session_holds_is_what_there_is_to_choose_from() {
        let mut store = Store::ephemeral().expect("store");
        let kept = scratch(&mut store, "worth keeping");
        scratch(&mut store, "passing chatter");

        assert_eq!(
            store
                .uncrossed(&SessionId::new("0831-yt8z"))
                .expect("list")
                .len(),
            2
        );
        store
            .promote(&kept, Tier::Fact, asked_for(), NOW)
            .expect("promote");
        let left = store.uncrossed(&SessionId::new("0831-yt8z")).expect("list");
        assert_eq!(left.len(), 1, "what crossed is no longer waiting");
        assert_eq!(left[0].text(), "passing chatter");
    }
}
