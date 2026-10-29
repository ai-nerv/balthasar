//! Rules the person took back: what an undone note said, and the turns it cited.

use crate::{Change, Store, StoreError, Transcript};
use balthasar_model::{MemoryId, SessionId};
use rusqlite::params;
use serde_json::Value;

/// How much of a claim must match an undone note's for the two to be one rule.
const ALIKE: f32 = 0.6;

/// One undone note change, as the person left it.
#[derive(Debug, Clone, PartialEq)]
pub struct TakenBack {
    /// What the note said.
    pub text: String,
    /// The turns its evidence cited.
    pub cited: Vec<(SessionId, u64)>,
}

impl TakenBack {
    /// The rule an undone change took back, when it made a note that cited something.
    #[must_use]
    pub fn of(change: &Change) -> Option<Self> {
        if change.state != "undone" {
            return None;
        }
        let after = change.after.as_ref()?;
        let text = after.get("text").and_then(Value::as_str)?.to_owned();
        let sources = [
            &change.evidence,
            after.get("evidence").unwrap_or(&Value::Null),
        ]
        .into_iter()
        .filter_map(|evidence| evidence.get("sources").and_then(Value::as_array))
        .flatten();
        let cited: Vec<(SessionId, u64)> = sources
            .filter_map(|source| {
                let session = source.get("session").and_then(Value::as_str)?;
                let cursor = source.get("cursor").and_then(Value::as_u64)?;
                Some((SessionId::new(session), cursor))
            })
            .collect();
        (!cited.is_empty()).then_some(Self { text, cited })
    }

    /// Whether `text`, said at `session`'s `cursor`, is this rule.
    #[must_use]
    pub fn covers(&self, session: &SessionId, cursor: u64, text: &str) -> bool {
        self.cited.iter().any(|(s, c)| s == session && *c == cursor)
            && (balthasar_model::same_claim(&self.text, text)
                || balthasar_model::claim_overlap(&self.text, text) >= ALIKE)
    }
}

impl Transcript {
    /// Every note change the person undid that cited a turn.
    pub fn taken_back(&self) -> Result<Vec<TakenBack>, StoreError> {
        let mut out = Vec::new();
        for change in self.changes(usize::MAX)? {
            out.extend(TakenBack::of(&change));
        }
        Ok(out)
    }
}

impl Store {
    /// The open memories a witness at `session`'s `cursor` vouches for.
    pub fn witnessed_at(
        &self,
        session: &SessionId,
        cursor: u64,
    ) -> Result<Vec<MemoryId>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT DISTINCT w.memory FROM witness w JOIN memory m ON m.id = w.memory \
             WHERE w.session = ?1 AND w.cursor = ?2 AND m.valid_to IS NULL ORDER BY w.memory",
        )?;
        let found = statement
            .query_map(
                params![session.as_str(), i64::try_from(cursor).unwrap_or(i64::MAX)],
                |r| Ok(MemoryId::new(r.get::<_, String>(0)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }
}
