//! Between a row and a record.
//!
//! One place, both directions. A column read in two files drifts the moment either is edited,
//! and the drift shows up as a memory that comes back subtly different from the one that went
//! in — which is the hardest kind of bug to see in a store nobody reads by hand.

use crate::StoreError;
use balthasar_model::{
    Body, Importance, Link, LinkRelation, Memory, MemoryId, Privacy, Provenance, ScopeId,
    SessionId, Strength, Temporal, Through, Tier, Witness, WitnessId, WitnessKind,
};
use rusqlite::Row;

/// Every column of `memory`, in the order the queries below select them.
///
/// Qualified, because recall joins `memory_fts` and both tables have an `id`. An unqualified
/// list works everywhere except the one query that matters and fails there with "ambiguous
/// column name", which says nothing about which of the two was meant.
pub const COLUMNS: &str = "memory.id, memory.tier, memory.scope, memory.session, \
     memory.body, memory.text, memory.content_hash, \
     memory.observed_at, memory.happened_at, memory.valid_from, memory.valid_to, \
     memory.importance, memory.strength, memory.last_accessed, memory.access_count, \
     memory.pinned, memory.confidence, memory.privacy, memory.through, memory.who, \
     memory.archived_at, memory.embedding, memory.embed_model, memory.single_valued";

/// A memory from a row, without its witnesses or links.
///
/// Those are separate tables and separate queries; a caller that needs them asks for them, so
/// listing a thousand memories does not fetch four thousand witnesses nobody looked at.
pub fn memory(row: &Row<'_>) -> Result<Memory, StoreError> {
    let body_json: String = row.get("body")?;
    let body: Body = serde_json::from_str(&body_json)?;

    Ok(Memory {
        id: MemoryId::new(row.get::<_, String>("id")?),
        tier: parse(&row.get::<_, String>("tier")?)?,
        scope: ScopeId::new(row.get::<_, String>("scope")?),
        session: row.get::<_, Option<String>>("session")?.map(SessionId::new),
        content_hash: row.get("content_hash")?,
        temporal: Temporal {
            observed_at: row.get("observed_at")?,
            happened_at: row.get("happened_at")?,
            valid_from: row.get("valid_from")?,
            valid_to: row.get("valid_to")?,
        },
        strength: Strength {
            value: row.get("strength")?,
            importance: parse(&row.get::<_, String>("importance")?)?,
            last_accessed: row.get("last_accessed")?,
            access_count: row.get("access_count")?,
            pinned: row.get::<_, i64>("pinned")? != 0,
        },
        confidence: row.get("confidence")?,
        privacy: parse(&row.get::<_, String>("privacy")?)?,
        provenance: Provenance {
            through: through(&row.get::<_, String>("through")?)?,
            who: row.get("who")?,
        },
        witnesses: Vec::new(),
        links: Vec::new(),
        archived_at: row.get("archived_at")?,
        embedding: row
            .get::<_, Option<Vec<u8>>>("embedding")?
            .as_deref()
            .and_then(floats),
        body,
    })
}

/// A little-endian `f32` blob as a vector, or `None` when it is not one.
///
/// A blob whose length is not a multiple of four was written by something else. Refusing it is
/// right: comparing against a misread vector produces a number that means nothing, and nothing
/// downstream would notice.
#[must_use]
pub fn floats(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.is_empty() || !blob.len().is_multiple_of(4) {
        return None;
    }
    Some(
        blob.as_chunks::<4>()
            .0
            .iter()
            .map(|four| f32::from_le_bytes(*four))
            .collect(),
    )
}

/// A vector as a little-endian blob.
#[must_use]
pub fn blob(floats: &[f32]) -> Vec<u8> {
    floats.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// One witness from a row.
pub fn witness(row: &Row<'_>) -> Result<Witness, StoreError> {
    Ok(Witness {
        id: WitnessId::new(row.get::<_, String>("id")?),
        kind: parse::<WitnessKind>(&row.get::<_, String>("kind")?)?,
        session: SessionId::new(row.get::<_, String>("session")?),
        scope: ScopeId::new(row.get::<_, String>("scope")?),
        at: row.get("at")?,
        cursor: row.get::<_, Option<i64>>("cursor")?.map(|c| c as u64),
        weight: row.get("weight")?,
        note: row.get("note")?,
        // An older row has neither. The defaults are what it always meant: a peer assertion
        // whose source is its own session.
        channel: row
            .get::<_, Option<String>>("channel")?
            .and_then(|t| t.parse().ok())
            .unwrap_or_default(),
        domain: row
            .get::<_, Option<String>>("domain")?
            .map(balthasar_model::Domain::new),
    })
}

/// One link from a row.
pub fn link(row: &Row<'_>) -> Result<Link, StoreError> {
    Ok(Link {
        to: MemoryId::new(row.get::<_, String>("dst")?),
        rel: relation(&row.get::<_, String>("rel")?)?,
        at: row.get("at")?,
    })
}

/// Anything with a `FromStr` whose failure means the file was written by a different balthasar.
fn parse<T>(text: &str) -> Result<T, StoreError>
where
    T: std::str::FromStr,
{
    text.parse()
        .map_err(|_| StoreError::Foreign(text.to_owned()))
}

/// `Through` has no `FromStr` in the model crate, because nothing outside the store spells it.
fn through(text: &str) -> Result<Through, StoreError> {
    match text {
        "local" => Ok(Through::Local),
        "peer" => Ok(Through::Peer),
        "ingest" => Ok(Through::Ingest),
        "sleep" => Ok(Through::Sleep),
        other => Err(StoreError::Foreign(other.to_owned())),
    }
}

/// The column spelling of a door.
#[must_use]
pub fn through_str(through: Through) -> &'static str {
    match through {
        Through::Local => "local",
        Through::Peer => "peer",
        Through::Ingest => "ingest",
        Through::Sleep => "sleep",
    }
}

/// `LinkRelation` likewise.
fn relation(text: &str) -> Result<LinkRelation, StoreError> {
    match text {
        "supersedes" => Ok(LinkRelation::Supersedes),
        "contradicts" => Ok(LinkRelation::Contradicts),
        "supports" => Ok(LinkRelation::Supports),
        "derived_from" => Ok(LinkRelation::DerivedFrom),
        "about" => Ok(LinkRelation::About),
        other => Err(StoreError::Foreign(other.to_owned())),
    }
}

/// The subject, predicate and object a body occupies, when it occupies one.
///
/// Lifted out of the JSON so the partial unique index can key on them. Everything else about a
/// body stays in the JSON, where a new variant costs no migration.
#[must_use]
pub fn slot(body: &Body) -> (Option<String>, Option<String>, Option<String>) {
    match body.slot() {
        Some((subject, predicate)) => (
            Some(subject.to_owned()),
            Some(predicate.to_owned()),
            body.object().map(str::to_owned),
        ),
        None => (None, None, None),
    }
}

/// Unused importance spelling, kept beside its sibling so both are found together.
#[must_use]
pub fn importance_str(importance: Importance) -> &'static str {
    importance.as_str()
}

/// Likewise for privacy and tier, so every column spelling is written in one file.
#[must_use]
pub fn privacy_str(privacy: Privacy) -> &'static str {
    privacy.as_str()
}

/// And for tiers.
#[must_use]
pub fn tier_str(tier: Tier) -> &'static str {
    tier.as_str()
}

/// And for relations, in the writing direction.
#[must_use]
pub fn relation_str(rel: LinkRelation) -> &'static str {
    rel.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_written_by_a_different_balthasar_says_so() {
        // "this store holds a 'semantic' that this build does not know" is a useful sentence.
        // "invalid value" is not.
        let failed = parse::<Tier>("semantic");
        assert!(matches!(failed, Err(StoreError::Foreign(t)) if t == "semantic"));
    }

    #[test]
    fn every_door_round_trips() {
        for door in [
            Through::Local,
            Through::Peer,
            Through::Ingest,
            Through::Sleep,
        ] {
            assert_eq!(through(through_str(door)).expect("known"), door);
        }
    }

    #[test]
    fn every_relation_round_trips() {
        for rel in [
            LinkRelation::Supersedes,
            LinkRelation::Contradicts,
            LinkRelation::Supports,
            LinkRelation::DerivedFrom,
            LinkRelation::About,
        ] {
            assert_eq!(relation(relation_str(rel)).expect("known"), rel);
        }
    }

    #[test]
    fn a_vector_survives_the_round_trip() {
        let vector = vec![0.5_f32, -0.25, 0.0, 1.0];
        assert_eq!(floats(&blob(&vector)), Some(vector));
    }

    #[test]
    fn a_blob_that_is_not_a_vector_is_refused() {
        // Comparing against a misread vector produces a number that means nothing, and
        // nothing downstream would notice.
        assert_eq!(floats(&[1, 2, 3]), None);
        assert_eq!(floats(&[]), None);
    }

    #[test]
    fn only_a_fact_fills_the_indexed_columns() {
        let (s, p, o) = slot(&Body::fact("project", "test_command", "make test"));
        assert_eq!(
            (s.as_deref(), p.as_deref(), o.as_deref()),
            (Some("project"), Some("test_command"), Some("make test"))
        );
        let (s, _, _) = slot(&Body::note("hi", balthasar_model::NoteKind::Scratch));
        assert_eq!(s, None);
    }
}
