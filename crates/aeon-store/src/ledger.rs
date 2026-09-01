//! What is in a session's context window right now.
//!
//! The short-term half. A harness streams its turns here as they settle, and asks what it
//! should send before each request. Because the streaming already happened, the question
//! "what should I send" needs only cursors — the text is already here.
//!
//! Nothing in this file decides anything. It records and it answers; the deciding is
//! `aeon-buffer`'s, which is the arrangement that lets a plan be rehearsed without a store to
//! rehearse it against.

use crate::{Store, StoreError};
use aeon_model::{MemoryId, SessionId, Timestamp};
use rusqlite::params;
use std::fmt;
use std::str::FromStr;

/// What has become of one turn in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Sent as it was written.
    #[default]
    Live,
    /// Replaced by a short description of itself. Reversible: the text is still in scratch.
    Masked,
    /// Not sent. Covered by a summary, or simply too old to matter.
    Dropped,
    /// Part of the span a summary stands in for.
    Summarised,
}

impl State {
    /// The column spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Masked => "masked",
            Self::Dropped => "dropped",
            Self::Summarised => "summarised",
        }
    }

    /// Whether a turn in this state still costs its full length.
    #[must_use]
    pub fn is_live(self) -> bool {
        matches!(self, Self::Live)
    }
}

impl FromStr for State {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "live" => Ok(Self::Live),
            "masked" => Ok(Self::Masked),
            "dropped" => Ok(Self::Dropped),
            "summarised" => Ok(Self::Summarised),
            other => Err(other.to_owned()),
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One turn's place in one window.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    /// The harness's own index. aeon never renumbers, so a plan can be applied without a
    /// translation table and a harness that rewinds simply stops sending the cursors it left.
    pub cursor: u64,
    /// The scratch memory holding what was said, when there is one.
    pub memory: Option<MemoryId>,
    /// Who said it.
    pub role: String,
    /// What kind of turn.
    pub kind: String,
    /// Which tool, for a call or its result.
    pub tool: Option<String>,
    /// What it costs.
    pub tokens: u32,
    /// What has become of it.
    pub state: State,
    /// Whether a plan may not touch it.
    pub pinned: bool,
    /// When it settled.
    pub at: Timestamp,
}

impl Entry {
    /// What this turn actually costs to send, given its state.
    ///
    /// A masked turn costs what its replacement costs rather than nothing: the stub still
    /// occupies room, and a planner that counted it as free would keep masking things that
    /// were already masked.
    #[must_use]
    pub fn cost(&self, masked_cost: u32) -> u32 {
        match self.state {
            State::Live => self.tokens,
            State::Masked => masked_cost.min(self.tokens),
            State::Dropped | State::Summarised => 0,
        }
    }
}

impl Store {
    /// Record one turn, and what it cost.
    ///
    /// Idempotent on `(session, cursor)`: a harness re-sending a turn is correcting what it
    /// said about that turn, not adding a second copy of it to the window.
    pub fn observe(&mut self, session: &SessionId, entry: &Entry) -> Result<(), StoreError> {
        self.db().execute(
            "INSERT INTO ledger (session, cursor, memory, role, kind, tool, tokens, state, pinned, at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
             ON CONFLICT(session, cursor) DO UPDATE SET \
               memory = ?3, role = ?4, kind = ?5, tool = ?6, tokens = ?7, pinned = ?9, at = ?10",
            params![
                session.as_str(),
                entry.cursor as i64,
                entry.memory.as_ref().map(aeon_model::MemoryId::as_str),
                entry.role,
                entry.kind,
                entry.tool,
                entry.tokens,
                entry.state.as_str(),
                i64::from(entry.pinned),
                entry.at,
            ],
        )?;
        Ok(())
    }

    /// Everything in one session's window, in the order it was said.
    pub fn ledger(&self, session: &SessionId) -> Result<Vec<Entry>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT cursor, memory, role, kind, tool, tokens, state, pinned, at \
             FROM ledger WHERE session = ?1 ORDER BY cursor",
        )?;
        let found = statement
            .query_map(params![session.as_str()], |r| {
                Ok(Entry {
                    cursor: r.get::<_, i64>(0)?.max(0) as u64,
                    memory: r.get::<_, Option<String>>(1)?.map(MemoryId::new),
                    role: r.get(2)?,
                    kind: r.get(3)?,
                    tool: r.get(4)?,
                    tokens: r.get(5)?,
                    state: r.get::<_, String>(6)?.parse().unwrap_or_default(),
                    pinned: r.get::<_, i64>(7)? != 0,
                    at: r.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(found)
    }

    /// Record what a plan did to a turn.
    pub fn mark(
        &mut self,
        session: &SessionId,
        cursor: u64,
        state: State,
    ) -> Result<(), StoreError> {
        self.db().execute(
            "UPDATE ledger SET state = ?3 WHERE session = ?1 AND cursor = ?2",
            params![session.as_str(), cursor as i64, state.as_str()],
        )?;
        Ok(())
    }

    /// The text of one turn, for a plan that has to summarise it.
    pub fn said(&self, entry: &Entry) -> Result<Option<String>, StoreError> {
        let Some(id) = &entry.memory else {
            return Ok(None);
        };
        Ok(self.get(id)?.map(|memory| memory.text()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(cursor: u64, tokens: u32) -> Entry {
        Entry {
            cursor,
            memory: None,
            role: "tool".into(),
            kind: "tool_result".into(),
            tool: Some("shell".into()),
            tokens,
            state: State::Live,
            pinned: false,
            at: 0,
        }
    }

    #[test]
    fn a_window_comes_back_in_the_order_it_was_said() {
        let mut store = Store::ephemeral().expect("store");
        let session = SessionId::new("s");
        for cursor in [3, 1, 2] {
            store.observe(&session, &turn(cursor, 10)).expect("observe");
        }
        let cursors: Vec<u64> = store
            .ledger(&session)
            .expect("ledger")
            .into_iter()
            .map(|e| e.cursor)
            .collect();
        assert_eq!(cursors, [1, 2, 3]);
    }

    #[test]
    fn re_observing_a_turn_corrects_it_rather_than_duplicating_it() {
        let mut store = Store::ephemeral().expect("store");
        let session = SessionId::new("s");
        store.observe(&session, &turn(1, 10)).expect("first");
        store.observe(&session, &turn(1, 4000)).expect("again");
        let held = store.ledger(&session).expect("ledger");
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].tokens, 4000);
    }

    #[test]
    fn two_sessions_keep_separate_windows() {
        // A project has many sessions and they share durable memory. They do not share a
        // context window, and a planner that mixed them would drop one session's turns to
        // make room in another's.
        let mut store = Store::ephemeral().expect("store");
        store
            .observe(&SessionId::new("a"), &turn(1, 10))
            .expect("a");
        store
            .observe(&SessionId::new("b"), &turn(1, 10))
            .expect("b");
        assert_eq!(store.ledger(&SessionId::new("a")).expect("a").len(), 1);
        assert_eq!(store.ledger(&SessionId::new("b")).expect("b").len(), 1);
    }

    #[test]
    fn a_masked_turn_still_costs_what_its_replacement_costs() {
        // A planner that counted a masked turn as free would keep masking things that were
        // already masked and never reach its target.
        let mut entry = turn(1, 3000);
        entry.state = State::Masked;
        assert_eq!(entry.cost(40), 40);
        entry.state = State::Dropped;
        assert_eq!(entry.cost(40), 0);
        entry.state = State::Live;
        assert_eq!(entry.cost(40), 3000);
    }

    #[test]
    fn masking_never_makes_a_turn_cost_more_than_it_did() {
        let mut entry = turn(1, 12);
        entry.state = State::Masked;
        assert_eq!(
            entry.cost(40),
            12,
            "a stub longer than the thing is not a saving"
        );
    }

    #[test]
    fn a_state_survives_the_round_trip() {
        let mut store = Store::ephemeral().expect("store");
        let session = SessionId::new("s");
        store.observe(&session, &turn(1, 10)).expect("observe");
        store.mark(&session, 1, State::Masked).expect("mark");
        assert_eq!(
            store.ledger(&session).expect("ledger")[0].state,
            State::Masked
        );
    }

    #[test]
    fn observing_does_not_reset_a_state_a_plan_decided() {
        // A harness re-sending a turn is correcting its metadata, not undoing a masking the
        // planner has already applied and already told it about.
        let mut store = Store::ephemeral().expect("store");
        let session = SessionId::new("s");
        store.observe(&session, &turn(1, 10)).expect("observe");
        store.mark(&session, 1, State::Masked).expect("mark");
        store.observe(&session, &turn(1, 10)).expect("again");
        assert_eq!(
            store.ledger(&session).expect("ledger")[0].state,
            State::Masked
        );
    }
}
