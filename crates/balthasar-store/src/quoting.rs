//! Turns that still quote a memory's text, and taking the text out of them.

use crate::{StoreError, Transcript};
use balthasar_model::SessionId;
use rusqlite::params;

/// What a redacted quotation reads as.
pub const REDACTED: &str = "[redacted]";

/// The turns and revisions holding `?1` in their text or their raw record.
const QUOTES: &str = "instr(text, ?1) > 0 OR instr(coalesce(raw, ''), ?1) > 0 \
                      OR instr(coalesce(raw, ''), ?2) > 0";

impl Transcript {
    /// Every turn whose text, raw record or an earlier revision still holds `text`, as
    /// `(session, cursor)` in order.
    pub fn quoting(&self, text: &str) -> Result<Vec<(SessionId, u64)>, StoreError> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut statement = self.db().prepare(&format!(
            "SELECT session, cursor FROM turn WHERE {QUOTES} \
             UNION SELECT session, cursor FROM turn_revision WHERE {QUOTES} ORDER BY 1, 2"
        ))?;
        let found = statement
            .query_map(params![text, escaped(text)], |r| {
                Ok((
                    SessionId::new(r.get::<_, String>(0)?),
                    r.get::<_, i64>(1)?.max(0) as u64,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }

    /// Put [`REDACTED`] in place of `text` in every turn [`Transcript::quoting`] finds, its
    /// revisions and its search entry. Refused while a quoting session has a writer. Answers
    /// how many turns changed.
    pub fn redact(&self, text: &str) -> Result<usize, StoreError> {
        let quoting = self.quoting(text)?;
        if quoting.is_empty() {
            return Ok(0);
        }
        let tx = rusqlite::Transaction::new_unchecked(
            self.db(),
            rusqlite::TransactionBehavior::Immediate,
        )?;
        if let Some(maintenance) = crate::purge::maintenance(self)? {
            for (session, _) in &quoting {
                if !maintenance.available(session.as_str())? {
                    return Err(StoreError::Foreign(
                        "a transcript quoting it has a writer".into(),
                    ));
                }
            }
        }
        let quiet = escaped(REDACTED);
        for table in ["turn", "turn_revision"] {
            tx.execute(
                &format!(
                    "UPDATE {table} SET text = replace(text, ?1, ?3), \
                     raw = replace(replace(raw, ?1, ?3), ?2, ?4) WHERE {QUOTES}"
                ),
                params![text, escaped(text), REDACTED, quiet],
            )?;
        }
        for (session, cursor) in &quoting {
            let cursor = i64::try_from(*cursor).unwrap_or(i64::MAX);
            tx.execute(
                "DELETE FROM turn_fts WHERE session = ?1 AND cursor = ?2",
                params![session.as_str(), cursor],
            )?;
            tx.execute(
                "INSERT INTO turn_fts (text, session, cursor) SELECT text, session, cursor \
                 FROM turn WHERE session = ?1 AND cursor = ?2 AND trim(text) != ''",
                params![session.as_str(), cursor],
            )?;
        }
        tx.commit()?;
        self.db()
            .execute("INSERT INTO turn_fts(turn_fts) VALUES ('optimize')", [])?;
        Ok(quoting.len())
    }
}

/// `text` as it is spelled inside a JSON string.
fn escaped(text: &str) -> String {
    let quoted = serde_json::Value::String(text.to_owned()).to_string();
    quoted
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(&quoted)
        .to_owned()
}
