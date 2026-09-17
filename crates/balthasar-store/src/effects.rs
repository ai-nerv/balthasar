//! Durable distillation intents committed with their layout's transcript effects.

use crate::{StoreError, Transcript};
use balthasar_model::{AgentId, MemoryId, ScopeId, SessionId, Timestamp};
use rusqlite::{OptionalExtension, params};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutTarget {
    Unresolved,
    Missing,
    Bound(MemoryId),
    Finished,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutEffect {
    pub layout: String,
    pub session: SessionId,
    pub run: SessionId,
    pub agent: AgentId,
    pub scope: ScopeId,
    pub cursor: u64,
    pub text: String,
    pub at: Timestamp,
}

impl Transcript {
    /// The first selected target, or the effect's unresolved/completed state.
    pub fn layout_target(&self, effect: &LayoutEffect) -> Result<LayoutTarget, StoreError> {
        let row = self.db().query_row(
            "SELECT done, resolved, target FROM layout_effect \
             WHERE layout = ?1 AND cursor = ?2 AND session = ?3 AND agent = ?4 AND scope = ?5 AND run = ?6",
            params![effect.layout, effect.cursor, effect.session.as_str(), effect.agent.as_str(), effect.scope.as_str(), effect.run.as_str()],
            |r| Ok((r.get::<_, bool>(0)?, r.get::<_, bool>(1)?, r.get::<_, Option<String>>(2)?)),
        ).optional()?;
        Ok(match row {
            None | Some((true, _, _)) => LayoutTarget::Finished,
            Some((false, false, _)) => LayoutTarget::Unresolved,
            Some((false, true, None)) => LayoutTarget::Missing,
            Some((false, true, Some(id))) => LayoutTarget::Bound(MemoryId::new(id)),
        })
    }

    /// Persist the first resolution and return it, including when another replayer won.
    pub fn bind_layout_target(
        &self,
        effect: &LayoutEffect,
        target: Option<&MemoryId>,
    ) -> Result<LayoutTarget, StoreError> {
        self.atomic(|store| {
            store.db().execute(
                "UPDATE layout_effect SET resolved = 1, target = ?7 \
                 WHERE layout = ?1 AND cursor = ?2 AND session = ?3 AND agent = ?4 AND scope = ?5 AND run = ?6 \
                 AND done = 0 AND resolved = 0",
                params![effect.layout, effect.cursor, effect.session.as_str(), effect.agent.as_str(), effect.scope.as_str(), effect.run.as_str(), target.map(MemoryId::as_str)],
            )?;
            store.layout_target(effect)
        })
    }

    /// Queue the captured source in the same transaction as its layout confirmation.
    pub fn queue_layout_effect(&self, effect: &LayoutEffect) -> Result<(), StoreError> {
        let cursor = i64::try_from(effect.cursor)
            .map_err(|_| StoreError::Foreign("effect cursor exceeds the store range".into()))?;
        self.db().execute(
            "INSERT OR IGNORE INTO layout_effect \
             (layout, session, run, agent, scope, cursor, text, at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                effect.layout,
                effect.session.as_str(),
                effect.run.as_str(),
                effect.agent.as_str(),
                effect.scope.as_str(),
                cursor,
                effect.text,
                effect.at
            ],
        )?;
        Ok(())
    }

    /// Pending effects owned by this transcript, agent, and project.
    pub fn layout_effects(
        &self,
        session: &SessionId,
        agent: &AgentId,
        scope: &ScopeId,
    ) -> Result<Vec<LayoutEffect>, StoreError> {
        let mut query = self.db().prepare(
            "SELECT layout, run, cursor, text, at FROM layout_effect \
             WHERE session = ?1 AND agent = ?2 AND scope = ?3 AND done = 0 ORDER BY rowid",
        )?;
        let rows = query.query_map(
            params![session.as_str(), agent.as_str(), scope.as_str()],
            |r| {
                Ok(LayoutEffect {
                    layout: r.get(0)?,
                    session: session.clone(),
                    run: SessionId::new(r.get::<_, String>(1)?),
                    agent: agent.clone(),
                    scope: scope.clone(),
                    cursor: r.get::<_, u64>(2)?,
                    text: r.get(3)?,
                    at: r.get(4)?,
                })
            },
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Acknowledge a resolved effect and discard its captured text.
    pub fn finish_layout_effect(&self, effect: &LayoutEffect) -> Result<(), StoreError> {
        let changed = self.db().execute(
            "UPDATE layout_effect SET done = 1, text = '' \
             WHERE layout = ?1 AND cursor = ?2 AND session = ?3 AND agent = ?4 AND scope = ?5 AND run = ?6 AND resolved = 1",
            params![
                effect.layout,
                effect.cursor,
                effect.session.as_str(),
                effect.agent.as_str(),
                effect.scope.as_str(),
                effect.run.as_str()
            ],
        )?;
        if changed == 0 && self.layout_target(effect)? == LayoutTarget::Unresolved {
            return Err(StoreError::Foreign(
                "layout effect target has not been resolved".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) fn prepare(connection: &rusqlite::Connection) -> Result<(), StoreError> {
    let tx =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch(SCHEMA)?;
    let columns = {
        let mut query = tx.prepare("SELECT name FROM pragma_table_info('layout_effect')")?;
        query
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    for (name, declaration) in [
        (
            "resolved",
            "INTEGER NOT NULL DEFAULT 0 CHECK (resolved IN (0, 1))",
        ),
        ("target", "TEXT"),
    ] {
        if !columns.iter().any(|column| column == name) {
            tx.execute_batch(&format!(
                "ALTER TABLE layout_effect ADD COLUMN {name} {declaration}"
            ))?;
        }
    }
    tx.commit()?;
    Ok(())
}

const SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS layout_effect (
  layout TEXT NOT NULL,
  session TEXT NOT NULL,
  run TEXT NOT NULL,
  agent TEXT NOT NULL,
  scope TEXT NOT NULL,
  cursor INTEGER NOT NULL CHECK (cursor >= 0),
  text TEXT NOT NULL,
  at INTEGER NOT NULL,
  done INTEGER NOT NULL DEFAULT 0 CHECK (done IN (0, 1)),
  resolved INTEGER NOT NULL DEFAULT 0 CHECK (resolved IN (0, 1)),
  target TEXT,
  PRIMARY KEY (layout, cursor)
) STRICT;
CREATE INDEX IF NOT EXISTS pending_layout_effect ON layout_effect(session, agent, scope) WHERE done = 0;
";
