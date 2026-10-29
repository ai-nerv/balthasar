//! What may be put in front of a model as memory.

use crate::{Recall, Scored, Store, StoreError};
use balthasar_model::{Memory, Presentation, Tier};

/// A recalled memory that is open: built only here, from a search of `live_memory`.
#[derive(Debug, Clone)]
pub struct Current(Scored);

impl Current {
    /// The memory.
    #[must_use]
    pub fn memory(&self) -> &Memory {
        &self.0.memory
    }

    /// The memory with how it scored.
    #[must_use]
    pub fn into_scored(self) -> Scored {
        self.0
    }
}

impl Store {
    /// How a memory may be offered, from the channels its evidence arrived on: the best any
    /// one witness permits. Content that looks like an instruction, on a channel that cannot
    /// carry one, is quarantined.
    pub fn presentation_of(&self, memory: &Memory) -> Result<Presentation, StoreError> {
        Ok(presented(memory, &self.witnesses_of(&memory.id)?))
    }

    /// What is on record in `scope`: open facts and habits at or above `floor`, newest first,
    /// at most `limit` of them. Read from an index, never by sorting the store.
    pub fn catalogued(
        &self,
        scope: &str,
        floor: f64,
        limit: usize,
    ) -> Result<Vec<Current>, StoreError> {
        let mut statement = self.db().prepare(&catalog_sql())?;
        let found = statement
            .query_map(rusqlite::params![scope, floor, limit as i64], |r| {
                Ok(crate::row::memory(r))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        found
            .into_iter()
            .map(|memory| {
                let memory = memory?;
                Ok(Current(Scored::of(memory)))
            })
            .collect()
    }

    /// Ids this memory replaced, and ids of open memories it is still disputed with.
    pub fn standing_of(
        &self,
        memory: &Memory,
    ) -> Result<
        (
            Vec<balthasar_model::MemoryId>,
            Vec<balthasar_model::MemoryId>,
        ),
        StoreError,
    > {
        let mut replaced = Vec::new();
        let mut disputed = Vec::new();
        let links = self.links_of(&memory.id)?;
        let reconciled: Vec<_> = links
            .iter()
            .filter(|l| l.rel == balthasar_model::LinkRelation::Reconciled)
            .map(|l| l.to.clone())
            .collect();
        for link in links {
            match link.rel {
                balthasar_model::LinkRelation::Supersedes => replaced.push(link.to),
                balthasar_model::LinkRelation::Contradicts
                    if !reconciled.contains(&link.to) && self.get_live(&link.to)?.is_some() =>
                {
                    disputed.push(link.to);
                }
                _ => {}
            }
        }
        Ok((replaced, disputed))
    }

    /// A search that finds only open memories, whatever `ask` says about history.
    pub fn recall_current(&self, ask: &Recall) -> Result<Vec<Current>, StoreError> {
        let mut open = ask.clone();
        open.history = false;
        open.include_archived = false;
        Ok(self
            .recall(&open)?
            .into_iter()
            .filter(|hit| is_open(&hit.memory))
            .map(Current)
            .collect())
    }
}

/// [`Store::presentation_of`], from witnesses already read.
#[must_use]
pub fn presented(memory: &Memory, witnesses: &[balthasar_model::Witness]) -> Presentation {
    let domains: std::collections::BTreeSet<String> = witnesses
        .iter()
        .map(balthasar_model::Witness::domain_of)
        .collect();
    let suspicious = balthasar_model::looks_like_injection(&memory.text());
    witnesses
        .iter()
        .map(|w| {
            let flagged = suspicious && !w.channel.may_be_imperative();
            balthasar_model::presentation_for(w.channel, domains.len() > 1, flagged)
        })
        .min()
        .unwrap_or(Presentation::Evidence)
}

/// The catalog's query. The unary `+` keeps tier and confidence off every index, so the scan
/// follows `memory_catalog` in order and stops at the limit.
fn catalog_sql() -> String {
    format!(
        "SELECT {} FROM live_memory AS memory \
         WHERE memory.scope = ?1 AND +memory.tier IN ('fact', 'habit') \
           AND +memory.confidence >= ?2 \
         ORDER BY memory.valid_from DESC, memory.id DESC LIMIT ?3",
        crate::row::LIVE_COLUMNS
    )
}

fn is_open(memory: &Memory) -> bool {
    memory.temporal.valid_to.is_none()
        && memory.archived_at.is_none()
        && memory.tier != Tier::Archive
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_reads_an_index_and_never_sorts_the_store() {
        let store = Store::ephemeral().expect("store");
        let plan: Vec<String> = store
            .db()
            .prepare(&format!("EXPLAIN QUERY PLAN {}", catalog_sql()))
            .expect("prepare")
            .query_map(rusqlite::params!["/w", 0.35, 121], |r| {
                r.get::<_, String>(3)
            })
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("plan");
        let plan = plan.join("\n");
        assert!(plan.contains("memory_catalog"), "{plan}");
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    }
}
