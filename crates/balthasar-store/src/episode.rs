//! Storing what the segmenter decided.
//!
//! Rows here are derived. The transcript they point at is the authority, and a segmentation can
//! be thrown away and rebuilt without losing anything — which is what makes changing the rules
//! a thing that can be tried rather than a thing that has to be right first time.
//!
//! Versioned rather than overwritten. A new derivation is written beside the old one so the two
//! can be compared on the same session, and only then does anything switch over.

use crate::{Store, StoreError};
use balthasar_model::{SessionId, Timestamp};
use rusqlite::params;

/// One stored span, as it came back.
#[derive(Debug, Clone, PartialEq)]
pub struct Episode {
    /// This span.
    pub id: String,
    /// Which run.
    pub session: SessionId,
    /// First turn.
    pub start_cursor: u64,
    /// Last turn.
    pub end_cursor: u64,
    /// When it began.
    pub started_at: Timestamp,
    /// When it ended.
    pub ended_at: Timestamp,
    /// What opened it, and why.
    pub before: (String, String),
    /// What closed it, and why, when something has.
    pub after: Option<(String, String)>,
    /// Which rules produced it.
    pub method: String,
    /// Which version of them.
    pub derivation: u32,
}

impl Store {
    /// Replace one derivation's view of a session.
    ///
    /// Scoped to `(session, derivation)`, so writing version two leaves version one standing and
    /// the two can be compared. Rewriting the same version is idempotent, which is what lets a
    /// live session re-segment on every append without accumulating duplicates.
    pub fn keep_segments(
        &mut self,
        session: &SessionId,
        derivation: u32,
        spans: &[Episode],
    ) -> Result<usize, StoreError> {
        let tx = self.db_mut().transaction()?;
        for span in spans {
            tx.execute(
                "INSERT INTO episode_segment \
                 (id, session, start_cursor, end_cursor, started_at, ended_at, \
                  boundary_before, reason_before, boundary_after, reason_after, \
                  method, derivation) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
                 ON CONFLICT(session, derivation, start_cursor) DO UPDATE SET \
                   end_cursor = excluded.end_cursor, ended_at = excluded.ended_at, \
                   boundary_after = excluded.boundary_after, \
                   reason_after = excluded.reason_after",
                params![
                    span.id,
                    session.as_str(),
                    span.start_cursor as i64,
                    span.end_cursor as i64,
                    span.started_at,
                    span.ended_at,
                    span.before.0,
                    span.before.1,
                    span.after.as_ref().map(|a| a.0.clone()),
                    span.after.as_ref().map(|a| a.1.clone()),
                    span.method,
                    i64::from(derivation),
                ],
            )?;
        }
        tx.commit()?;
        Ok(spans.len())
    }

    /// One derivation's view of a session, in order.
    pub fn segments(
        &self,
        session: &SessionId,
        derivation: u32,
    ) -> Result<Vec<Episode>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT id, start_cursor, end_cursor, started_at, ended_at, \
                    boundary_before, reason_before, boundary_after, reason_after, method \
             FROM episode_segment WHERE session = ?1 AND derivation = ?2 ORDER BY start_cursor",
        )?;
        let rows = statement
            .query_map(params![session.as_str(), i64::from(derivation)], |r| {
                Ok(Episode {
                    id: r.get(0)?,
                    session: session.clone(),
                    start_cursor: r.get::<_, i64>(1)? as u64,
                    end_cursor: r.get::<_, i64>(2)? as u64,
                    started_at: r.get(3)?,
                    ended_at: r.get(4)?,
                    before: (r.get(5)?, r.get(6)?),
                    after: match (
                        r.get::<_, Option<String>>(7)?,
                        r.get::<_, Option<String>>(8)?,
                    ) {
                        (Some(signal), Some(why)) => Some((signal, why)),
                        _ => None,
                    },
                    method: r.get(9)?,
                    derivation,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Which derivations this store holds for a session, oldest first.
    ///
    /// What a comparison between two versions of the rules starts from.
    pub fn derivations_of(&self, session: &SessionId) -> Result<Vec<u32>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT DISTINCT derivation FROM episode_segment WHERE session = ?1 \
             ORDER BY derivation",
        )?;
        let rows = statement
            .query_map(
                params![session.as_str()],
                |r| Ok(r.get::<_, i64>(0)? as u32),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = 1_756_000_000;

    fn run() -> SessionId {
        SessionId::new("01RUN")
    }

    fn span(id: &str, from: u64, to: u64, closed: bool) -> Episode {
        Episode {
            id: id.to_owned(),
            session: run(),
            start_cursor: from,
            end_cursor: to,
            started_at: NOW,
            ended_at: NOW + 60,
            before: ("session-start".to_owned(), "the run began".to_owned()),
            after: closed.then(|| {
                (
                    "goal-change".to_owned(),
                    "asked for something else".to_owned(),
                )
            }),
            method: "rules".to_owned(),
            derivation: 1,
        }
    }

    #[test]
    fn a_segmentation_comes_back_as_it_went_in() {
        let mut store = Store::ephemeral().expect("store");
        store
            .keep_segments(
                &run(),
                1,
                &[span("e1", 0, 2, true), span("e2", 3, 5, false)],
            )
            .expect("keep");

        let held = store.segments(&run(), 1).expect("read");
        assert_eq!(held.len(), 2);
        assert_eq!(held[0].end_cursor, 2);
        assert!(held[0].after.is_some(), "the first one closed");
        assert!(held[1].after.is_none(), "the last one is still open");
        assert_eq!(held[0].before.1, "the run began");
    }

    #[test]
    fn re_segmenting_the_same_version_does_not_duplicate() {
        // A live session re-segments on every append. Without this that would be one new row
        // per turn, and the derived table would outgrow the transcript it describes.
        let mut store = Store::ephemeral().expect("store");
        for _ in 0..5 {
            store
                .keep_segments(&run(), 1, &[span("e1", 0, 2, false)])
                .expect("keep");
        }
        assert_eq!(store.segments(&run(), 1).expect("read").len(), 1);
    }

    #[test]
    fn the_open_segment_grows_where_it_stands() {
        let mut store = Store::ephemeral().expect("store");
        store
            .keep_segments(&run(), 1, &[span("e1", 0, 2, false)])
            .expect("keep");
        store
            .keep_segments(&run(), 1, &[span("e1", 0, 7, false)])
            .expect("keep");

        let held = store.segments(&run(), 1).expect("read");
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].end_cursor, 7);
    }

    #[test]
    fn a_new_derivation_stands_beside_the_old_one() {
        // What makes changing the rules safe: both answers exist at once and can be compared on
        // the same session before anything switches over.
        let mut store = Store::ephemeral().expect("store");
        store
            .keep_segments(&run(), 1, &[span("e1", 0, 5, false)])
            .expect("v1");
        store
            .keep_segments(
                &run(),
                2,
                &[span("f1", 0, 2, true), span("f2", 3, 5, false)],
            )
            .expect("v2");

        assert_eq!(store.segments(&run(), 1).expect("read").len(), 1);
        assert_eq!(store.segments(&run(), 2).expect("read").len(), 2);
        assert_eq!(store.derivations_of(&run()).expect("versions"), vec![1, 2]);
    }

    #[test]
    fn a_session_nothing_has_segmented_has_no_episodes() {
        let store = Store::ephemeral().expect("store");
        assert!(store.segments(&run(), 1).expect("read").is_empty());
        assert!(store.derivations_of(&run()).expect("versions").is_empty());
    }
}
