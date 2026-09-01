//! Storing and traversing derived relationships.
//!
//! Everything here is an index. The memories these edges connect are the durable thing; the
//! edges themselves can be dropped and recomputed, and the point of keeping them in their own
//! table is that dropping them is safe.
//!
//! Traversal is bounded three ways — by kind, by fan-out, and by hop count — and the first
//! implementation walks exactly one hop. Multi-hop has to prove its value separately, because
//! an unbounded walk over a dense store is the failure mode that turns a memory layer into a
//! graph database nobody asked for.

use crate::{Store, StoreError};
use aeon_model::{Derivation, Family, MemoryId, Relation, Timestamp, View};
use rusqlite::params;

/// How far a traversal may reach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    /// How many edges to follow out of any one memory.
    pub fan_out: usize,
    /// How many memories a whole traversal may add.
    pub budget: usize,
    /// How many hops. One, until multi-hop earns more.
    pub hops: usize,
    /// The weight below which an edge is not worth following.
    pub floor: f64,
}

impl Default for Reach {
    fn default() -> Self {
        Self {
            fan_out: 8,
            budget: 64,
            hops: 1,
            floor: 0.2,
        }
    }
}

impl Store {
    /// Write derived edges, and their inverses where the kind has one.
    ///
    /// Inverses are materialised rather than computed at query time so that traversal is one
    /// indexed read in either direction. It costs rows and saves a union on every recall.
    pub fn relate(&mut self, edges: &[Relation]) -> Result<usize, StoreError> {
        let tx = self.db_mut().transaction()?;
        let mut written = 0;
        for edge in edges {
            written += write_edge(&tx, edge)?;
            // Including the kinds that are their own inverse. `a same-entity b` and
            // `b same-entity a` are different rows, and without the second one a traversal
            // from `b` cannot see `a` — which is the whole point of a symmetric edge.
            if let Some(back) = edge.view.inverse() {
                let mut mirrored = edge.clone();
                mirrored.from = edge.to.clone();
                mirrored.to = edge.from.clone();
                mirrored.view = back;
                written += write_edge(&tx, &mirrored)?;
            }
        }
        tx.commit()?;
        Ok(written)
    }

    /// Every live edge out of a memory, strongest first.
    pub fn relations_of(
        &self,
        memory: &MemoryId,
        families: &[Family],
        reach: &Reach,
    ) -> Result<Vec<Relation>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT to_memory, kind, weight, source, derivation_version, evidence_cursor, \
                    created_at \
             FROM relation_view \
             WHERE from_memory = ?1 AND stale_at IS NULL AND weight >= ?2 \
             ORDER BY weight DESC, to_memory \
             LIMIT ?3",
        )?;
        let rows = statement
            .query_map(
                params![memory.as_str(), reach.floor, reach.fan_out as i64],
                |r| {
                    Ok((
                        MemoryId::new(r.get::<_, String>(0)?),
                        r.get::<_, String>(1)?,
                        r.get::<_, f64>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, Option<i64>>(5)?,
                        r.get::<_, Timestamp>(6)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows
            .into_iter()
            .filter_map(|(to, kind, weight, source, version, cursor, at)| {
                let view: View = kind.parse().ok()?;
                // Filtering here rather than in SQL keeps the family definition in one place —
                // the model — instead of duplicated as a list of strings in a query.
                if !families.is_empty() && !families.contains(&view.family()) {
                    return None;
                }
                Some(Relation {
                    from: memory.clone(),
                    to,
                    view,
                    weight,
                    source: source.parse().unwrap_or(Derivation::Rule),
                    derivation_version: version.max(0) as u32,
                    evidence_cursor: cursor.map(|c| c as u64),
                    created_at: at,
                })
            })
            .collect())
    }

    /// Walk outward from a set of memories, gathering what the edges reach.
    ///
    /// Returns each memory found with the edge that reached it, so a result can say *why* it is
    /// a candidate rather than only that it is one. Bounded by `reach` at every step, and the
    /// seeds themselves are never returned — a traversal that rediscovered its own starting
    /// point would inflate every candidate set with what search already found.
    pub fn traverse(
        &self,
        seeds: &[MemoryId],
        families: &[Family],
        reach: &Reach,
    ) -> Result<Vec<(MemoryId, Relation)>, StoreError> {
        let mut seen: Vec<MemoryId> = seeds.to_vec();
        let mut out: Vec<(MemoryId, Relation)> = Vec::new();
        let mut frontier: Vec<MemoryId> = seeds.to_vec();

        for _ in 0..reach.hops.max(1) {
            let mut next = Vec::new();
            for from in &frontier {
                if out.len() >= reach.budget {
                    return Ok(out);
                }
                for edge in self.relations_of(from, families, reach)? {
                    if seen.contains(&edge.to) {
                        continue;
                    }
                    seen.push(edge.to.clone());
                    next.push(edge.to.clone());
                    out.push((edge.to.clone(), edge));
                    if out.len() >= reach.budget {
                        return Ok(out);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        Ok(out)
    }

    /// Retire every rebuildable edge from one derivation, keeping it readable.
    ///
    /// Marked stale rather than removed, so a new derivation can be compared against the one it
    /// replaces on the same store. What a person asserted by hand is left alone: rebuilding an
    /// index must not discard something somebody said.
    pub fn retire_relations(
        &mut self,
        source: Derivation,
        version: u32,
        now: Timestamp,
    ) -> Result<usize, StoreError> {
        if !source.is_rebuildable() {
            return Ok(0);
        }
        Ok(self.db().execute(
            "UPDATE relation_view SET stale_at = ?3 \
             WHERE source = ?1 AND derivation_version = ?2 AND stale_at IS NULL",
            params![source.as_str(), i64::from(version), now],
        )?)
    }

    /// How many live edges of each kind this store holds.
    pub fn relation_census(&self) -> Result<Vec<(View, usize)>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT kind, count(*) FROM relation_view WHERE stale_at IS NULL \
             GROUP BY kind ORDER BY count(*) DESC",
        )?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(kind, n)| Some((kind.parse().ok()?, n.max(0) as usize)))
            .collect())
    }
}

/// One edge, replacing whatever was there for the same derivation.
fn write_edge(tx: &rusqlite::Transaction<'_>, edge: &Relation) -> Result<usize, StoreError> {
    Ok(tx.execute(
        "INSERT INTO relation_view \
         (from_memory, to_memory, kind, weight, source, derivation_version, \
          evidence_cursor, created_at, stale_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL) \
         ON CONFLICT(from_memory, to_memory, kind, derivation_version) DO UPDATE SET \
           weight = excluded.weight, source = excluded.source, \
           evidence_cursor = excluded.evidence_cursor, created_at = excluded.created_at, \
           stale_at = NULL",
        params![
            edge.from.as_str(),
            edge.to.as_str(),
            edge.view.as_str(),
            edge.weight,
            edge.source.as_str(),
            i64::from(edge.derivation_version),
            edge.evidence_cursor.map(|c| c as i64),
            edge.created_at,
        ],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = 1_756_000_000;

    fn edge(from: &str, to: &str, view: View, weight: f64) -> Relation {
        Relation {
            from: MemoryId::new(from),
            to: MemoryId::new(to),
            view,
            weight,
            source: Derivation::Rule,
            derivation_version: 1,
            evidence_cursor: None,
            created_at: NOW,
        }
    }

    #[test]
    fn an_order_edge_is_written_both_ways() {
        // So that "what came before this" and "what came after that" are each one indexed read
        // rather than a union over two directions.
        let mut store = Store::ephemeral().expect("store");
        store
            .relate(&[edge("a", "b", View::Before, 1.0)])
            .expect("relate");

        let out = store
            .relations_of(&MemoryId::new("b"), &[], &Reach::default())
            .expect("read");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].view, View::After);
        assert_eq!(out[0].to, MemoryId::new("a"));
    }

    #[test]
    fn a_symmetric_edge_is_reachable_from_either_end() {
        // Found by the census: `same-entity` is its own inverse, and an earlier guard against
        // writing a duplicate stopped the mirror row from existing at all. Two memories were
        // related in one direction only, so half the traversals silently missed.
        let mut store = Store::ephemeral().expect("store");
        store
            .relate(&[edge("a", "b", View::SameEntity, 1.0)])
            .expect("relate");

        for end in ["a", "b"] {
            let out = store
                .relations_of(&MemoryId::new(end), &[], &Reach::default())
                .expect("read");
            assert_eq!(out.len(), 1, "nothing reachable from {end}");
            assert_eq!(out[0].view, View::SameEntity);
        }
    }

    #[test]
    fn a_causal_edge_is_written_one_way_only() {
        // A fix resolves a failure. Materialising an inverse would let a traversal walk from
        // the repair back to the thing it repaired and call that a cause.
        let mut store = Store::ephemeral().expect("store");
        store
            .relate(&[edge("fix", "fail", View::Resolved, 1.0)])
            .expect("relate");

        assert_eq!(
            store
                .relations_of(&MemoryId::new("fail"), &[], &Reach::default())
                .expect("read")
                .len(),
            0
        );
        assert_eq!(
            store
                .relations_of(&MemoryId::new("fix"), &[], &Reach::default())
                .expect("read")
                .len(),
            1
        );
    }

    #[test]
    fn a_family_filter_narrows_what_is_traversed() {
        let mut store = Store::ephemeral().expect("store");
        store
            .relate(&[
                edge("a", "b", View::Before, 1.0),
                edge("a", "c", View::SameEntity, 1.0),
                edge("a", "d", View::Resolved, 1.0),
            ])
            .expect("relate");

        let seed = MemoryId::new("a");
        let reach = Reach::default();
        assert_eq!(
            store
                .relations_of(&seed, &[Family::Temporal], &reach)
                .expect("read")
                .len(),
            1
        );
        assert_eq!(
            store
                .relations_of(&seed, &[Family::Causal], &reach)
                .expect("read")
                .len(),
            1
        );
        assert_eq!(
            store.relations_of(&seed, &[], &reach).expect("read").len(),
            3
        );
    }

    #[test]
    fn a_weak_edge_is_not_worth_following() {
        let mut store = Store::ephemeral().expect("store");
        store
            .relate(&[
                edge("a", "b", View::SimilarTo, 0.9),
                edge("a", "c", View::SimilarTo, 0.05),
            ])
            .expect("relate");

        let held = store
            .relations_of(&MemoryId::new("a"), &[], &Reach::default())
            .expect("read");
        assert_eq!(held.len(), 1, "the weak one was left alone");
        assert_eq!(held[0].to, MemoryId::new("b"));
    }

    #[test]
    fn traversal_never_returns_what_it_started_from() {
        // Otherwise every candidate set would be inflated with what search had already found,
        // and a relationship view would look better than it is.
        let mut store = Store::ephemeral().expect("store");
        store
            .relate(&[edge("a", "b", View::Before, 1.0)])
            .expect("relate");

        let found = store
            .traverse(&[MemoryId::new("a")], &[], &Reach::default())
            .expect("walk");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, MemoryId::new("b"));
    }

    #[test]
    fn traversal_stops_at_one_hop_by_default() {
        // Multi-hop has to prove its value separately. An unbounded walk over a dense store is
        // how a memory layer turns into a graph database nobody asked for.
        let mut store = Store::ephemeral().expect("store");
        store
            .relate(&[
                edge("a", "b", View::SameEntity, 1.0),
                edge("b", "c", View::SameEntity, 1.0),
            ])
            .expect("relate");

        let one = store
            .traverse(&[MemoryId::new("a")], &[], &Reach::default())
            .expect("walk");
        assert_eq!(one.len(), 1, "b only");

        let two = store
            .traverse(
                &[MemoryId::new("a")],
                &[],
                &Reach {
                    hops: 2,
                    ..Reach::default()
                },
            )
            .expect("walk");
        assert_eq!(two.len(), 2, "b and c");
    }

    #[test]
    fn a_dense_store_cannot_blow_the_budget() {
        let mut store = Store::ephemeral().expect("store");
        let edges: Vec<Relation> = (0..500)
            .map(|n| edge("hub", &format!("m{n}"), View::SameEntity, 1.0))
            .collect();
        store.relate(&edges).expect("relate");

        let reach = Reach {
            budget: 10,
            fan_out: 100,
            ..Reach::default()
        };
        let found = store
            .traverse(&[MemoryId::new("hub")], &[], &reach)
            .expect("walk");
        assert!(found.len() <= 10, "{} came back", found.len());
    }

    #[test]
    fn rebuilding_retires_what_it_replaces_and_keeps_it_readable() {
        let mut store = Store::ephemeral().expect("store");
        store
            .relate(&[edge("a", "b", View::SameEntity, 1.0)])
            .expect("relate");
        let retired = store
            .retire_relations(Derivation::Rule, 1, NOW + 10)
            .expect("retire");

        assert!(retired > 0);
        assert!(
            store
                .relations_of(&MemoryId::new("a"), &[], &Reach::default())
                .expect("read")
                .is_empty(),
            "a stale edge is not traversed"
        );
        let still: i64 = store
            .db()
            .query_row("SELECT count(*) FROM relation_view", [], |r| r.get(0))
            .expect("count");
        assert!(still > 0, "and it is still there to compare against");
    }

    #[test]
    fn what_a_person_asserted_survives_a_rebuild() {
        let mut store = Store::ephemeral().expect("store");
        let mut said = edge("a", "b", View::Caused, 1.0);
        said.source = Derivation::Manual;
        store.relate(&[said]).expect("relate");

        assert_eq!(
            store
                .retire_relations(Derivation::Manual, 1, NOW)
                .expect("retire"),
            0
        );
        assert_eq!(
            store
                .relations_of(&MemoryId::new("a"), &[], &Reach::default())
                .expect("read")
                .len(),
            1
        );
    }

    #[test]
    fn a_new_derivation_does_not_disturb_the_old_one() {
        let mut store = Store::ephemeral().expect("store");
        store
            .relate(&[edge("a", "b", View::SameEntity, 1.0)])
            .expect("v1");
        let mut second = edge("a", "b", View::SameEntity, 0.4);
        second.derivation_version = 2;
        store.relate(&[second]).expect("v2");

        let held = store
            .relations_of(&MemoryId::new("a"), &[], &Reach::default())
            .expect("read");
        assert_eq!(held.len(), 2, "both versions are live and comparable");
    }

    #[test]
    fn deriving_the_same_edge_twice_updates_rather_than_duplicates() {
        let mut store = Store::ephemeral().expect("store");
        store
            .relate(&[edge("a", "b", View::SameEntity, 0.4)])
            .expect("once");
        store
            .relate(&[edge("a", "b", View::SameEntity, 0.9)])
            .expect("again");

        let held = store
            .relations_of(&MemoryId::new("a"), &[], &Reach::default())
            .expect("read");
        assert_eq!(held.len(), 1);
        assert!((held[0].weight - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn an_edge_never_touches_the_memories_it_connects() {
        // The load-bearing separation: relating two memories is not evidence about either.
        let mut store = Store::ephemeral().expect("store");
        let held = aeon_model::Memory::new(
            crate::mint(NOW),
            aeon_model::Tier::Fact,
            aeon_model::ScopeId::new("/w/p"),
            aeon_model::Body::note("a claim", aeon_model::NoteKind::Claim),
            NOW,
        );
        let id = held.id.clone();
        store
            .remember(
                held,
                aeon_model::Witness::new(
                    aeon_model::WitnessId::new("w1"),
                    aeon_model::WitnessKind::Imperative,
                    aeon_model::SessionId::new("01RUN"),
                    aeon_model::ScopeId::new("/w/p"),
                    NOW,
                ),
                NOW,
            )
            .expect("remember");
        let before = store.get(&id).expect("get").expect("there").confidence;

        for n in 0..50 {
            store
                .relate(&[edge(
                    id.as_str(),
                    &format!("other{n}"),
                    View::SimilarTo,
                    1.0,
                )])
                .expect("relate");
        }
        let after = store.get(&id).expect("get").expect("there").confidence;
        assert!(
            (before - after).abs() < f64::EPSILON,
            "fifty edges moved confidence: {before} -> {after}"
        );
    }
}
