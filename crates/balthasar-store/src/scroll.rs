//! Reading part of a scrollback, when the whole of it will not fit.
//!
//! `replay` hands back every turn a run ever produced. The reads here are bounded by a token
//! budget instead: bounding by turns would let one pasted stack trace eat the whole allowance.
//!
//! Four questions: the tail, a cursor span, the turns around a cursor, and turns matching words
//! within one run. Every read says what it left out, so a caller can tell "that is all there
//! was" from "that is all you asked for".

use crate::{StoreError, Transcript, Turn};
use balthasar_model::SessionId;
use rusqlite::params;

/// Characters per token, for estimating what a turn costs. The same rough four as elsewhere.
const CHARS_PER_TOKEN: usize = 4;

/// How much may come back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    pub tokens: usize,
    /// A hard cap on turns, whatever the tokens say. Ten thousand one-word turns fit a generous
    /// budget and are still useless to read.
    pub turns: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            tokens: 4_000,
            turns: 200,
        }
    }
}

/// What part of a run to read.
#[derive(Debug, Clone, PartialEq)]
pub enum Want {
    /// The most recent turns. What a resuming run wants.
    Tail,
    /// A cursor range, inclusive. What an episode is.
    Span {
        /// First cursor.
        from: u64,
        /// Last cursor.
        to: u64,
    },
    /// The turns on either side of one cursor, for quoting it in context.
    Around {
        /// The turn being cited.
        cursor: u64,
    },
    /// Turns mentioning all of these words, within this run.
    Matching {
        /// The words, lowercased on the way in.
        terms: Vec<String>,
    },
}

/// What came back, and what did not.
#[derive(Debug, Clone, PartialEq)]
pub struct Read {
    /// The turns, in cursor order.
    pub turns: Vec<Turn>,
    /// What they are estimated to cost.
    pub tokens: usize,
    /// How many turns matched but did not fit.
    pub omitted: usize,
    /// Where to continue from, when the budget stopped it short. `Some` means there is more in
    /// the direction being read; a caller that wants the rest asks again from here.
    pub next: Option<u64>,
}

impl Read {
    /// Whether everything asked for fitted.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.omitted == 0
    }

    /// The sentence a caller shows when it did not.
    #[must_use]
    pub fn note(&self) -> Option<String> {
        (!self.is_complete()).then(|| {
            format!(
                "{} more turn(s) in this run; continue from cursor {}",
                self.omitted,
                self.next.unwrap_or_default()
            )
        })
    }
}

use crate::transcript::read as turn_of;

/// What a turn costs: the harness's own count when it has one, and an estimate otherwise.
#[must_use]
pub fn tokens_of(turn: &Turn) -> usize {
    match turn.tokens {
        Some(counted) => counted as usize,
        None => turn.text.len().div_ceil(CHARS_PER_TOKEN),
    }
}

impl Transcript {
    /// Read part of a run, within a budget.
    pub fn read(
        &self,
        session: &SessionId,
        want: &Want,
        budget: &Budget,
    ) -> Result<Read, StoreError> {
        match want {
            Want::Tail => self.backwards(session, u64::MAX, budget),
            Want::Span { from, to } => self.forwards(session, *from, Some(*to), budget),
            Want::Around { cursor } => self.centred(session, *cursor, budget),
            Want::Matching { terms } => self.matching(session, terms, budget),
        }
    }

    /// Turns from `from` onwards, stopping at `to` or at the budget. Stepped, so what is not
    /// returned is not read; the rest is counted.
    fn forwards(
        &self,
        session: &SessionId,
        from: u64,
        to: Option<u64>,
        budget: &Budget,
    ) -> Result<Read, StoreError> {
        let ceiling = to.unwrap_or(u64::MAX);
        let turns = self.stepped(
            &format!(
                "SELECT {} FROM turn WHERE session = ?1 AND cursor >= ?2 AND cursor <= ?3 \
                 ORDER BY cursor",
                crate::transcript::COLUMNS
            ),
            params![session.as_str(), clamp(from), clamp(ceiling)],
            budget,
        )?;
        let tokens = turns.iter().map(tokens_of).sum();
        let total: i64 = self.db().query_row(
            "SELECT count(*) FROM turn WHERE session = ?1 AND cursor >= ?2 AND cursor <= ?3",
            params![session.as_str(), clamp(from), clamp(ceiling)],
            |r| r.get(0),
        )?;
        let omitted = (total.max(0) as usize).saturating_sub(turns.len());
        let next = (omitted > 0).then(|| turns.last().map_or(from, |t| t.cursor + 1));
        Ok(Read {
            turns,
            tokens,
            omitted,
            next,
        })
    }

    /// Turns ending at `until`, read newest-first and stopped at the budget.
    fn backwards(
        &self,
        session: &SessionId,
        until: u64,
        budget: &Budget,
    ) -> Result<Read, StoreError> {
        let mut turns = self.newest_first(session, until, budget)?;
        let tokens = turns.iter().map(tokens_of).sum();
        turns.reverse();
        self.drop_a_leading_part_message(session, &mut turns)?;
        let omitted = self.how_many_up_to(session, until)? - turns.len();
        // Reading backwards, "more" is what came *before*, so continuing asks for the span that
        // ends just before the earliest turn returned.
        let next = (omitted > 0).then(|| turns.first().map_or(0, |t| t.cursor.saturating_sub(1)));
        Ok(Read {
            turns,
            tokens,
            omitted,
            next,
        })
    }

    /// The turns either side of one cursor. Grown outward a turn at a time rather than by a fixed
    /// radius, so a citation surrounded by long turns gets fewer of them. No more than the turn
    /// budget is read from either side.
    fn centred(
        &self,
        session: &SessionId,
        cursor: u64,
        budget: &Budget,
    ) -> Result<Read, StoreError> {
        let Some(middle) = self.at(session, cursor)? else {
            return Ok(Read {
                turns: Vec::new(),
                tokens: 0,
                omitted: 0,
                next: None,
            });
        };
        let side = |order: &str, op: &str| -> Result<Vec<Turn>, StoreError> {
            let mut statement = self.db().prepare(&format!(
                "SELECT {} FROM turn WHERE session = ?1 AND cursor {op} ?2 \
                 ORDER BY cursor {order} LIMIT ?3",
                crate::transcript::COLUMNS
            ))?;
            let found = statement
                .query_map(
                    params![session.as_str(), clamp(cursor), budget.turns as i64],
                    turn_of,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(found)
        };
        let mut before = side("DESC", "<")?;
        before.reverse();
        let after = side("ASC", ">")?;

        let mut tokens = tokens_of(&middle);
        let mut taken: std::collections::VecDeque<Turn> = std::collections::VecDeque::new();
        taken.push_back(middle);

        let (mut back, mut fore) = (before.len(), 0usize);
        loop {
            let earlier = back.checked_sub(1).and_then(|at| before.get(at));
            let later = after.get(fore);
            // Alternate, preferring what comes after: a reader following a citation forward is
            // usually looking for what it led to.
            let next = match (later, earlier) {
                (Some(l), _) if fore <= (before.len() - back) => Some((false, l)),
                (_, Some(e)) => Some((true, e)),
                (Some(l), None) => Some((false, l)),
                (None, None) => None,
            };
            let Some((is_before, turn)) = next else { break };
            let cost = tokens_of(turn);
            if tokens + cost > budget.tokens || taken.len() >= budget.turns {
                break;
            }
            tokens += cost;
            if is_before {
                taken.push_front(turn.clone());
                back -= 1;
            } else {
                taken.push_back(turn.clone());
                fore += 1;
            }
        }

        let total = usize::try_from(self.count(session)?).unwrap_or(usize::MAX);
        Ok(Read {
            omitted: total.saturating_sub(taken.len()),
            turns: taken.into_iter().collect(),
            tokens,
            next: None,
        })
    }

    /// Turns mentioning every term, found through the search index: each term a word or the
    /// start of one. At most [`MOST_TERMS`] terms are used.
    fn matching(
        &self,
        session: &SessionId,
        terms: &[String],
        budget: &Budget,
    ) -> Result<Read, StoreError> {
        let wanted: Vec<String> = terms
            .iter()
            .map(|t| t.to_lowercase().replace('"', " ").trim().to_owned())
            .filter(|t| !t.is_empty())
            .take(MOST_TERMS)
            .map(|t| format!("\"{t}\"*"))
            .collect();
        if wanted.is_empty() {
            return Ok(Read {
                turns: Vec::new(),
                tokens: 0,
                omitted: 0,
                next: None,
            });
        }
        let query = wanted.join(" AND ");
        let columns: Vec<String> = crate::transcript::COLUMNS
            .split(',')
            .map(|c| format!("turn.{}", c.trim()))
            .collect();
        let turns = self.stepped(
            &format!(
                "SELECT {} FROM turn_fts JOIN turn \
                   ON turn.session = turn_fts.session AND turn.cursor = turn_fts.cursor \
                 WHERE turn_fts MATCH ?1 AND turn_fts.session = ?2 ORDER BY turn.cursor",
                columns.join(", ")
            ),
            params![query, session.as_str()],
            budget,
        )?;
        let tokens = turns.iter().map(tokens_of).sum();
        let total: i64 = self.db().query_row(
            "SELECT count(*) FROM turn_fts WHERE turn_fts MATCH ?1 AND session = ?2",
            params![query, session.as_str()],
            |r| r.get(0),
        )?;
        let omitted = (total.max(0) as usize).saturating_sub(turns.len());
        let next = (omitted > 0).then(|| turns.last().map_or(0, |t| t.cursor + 1));
        Ok(Read {
            turns,
            tokens,
            omitted,
            next,
        })
    }

    /// The turns a statement steps through, stopping at the budget. The first is always taken,
    /// however large: a budget smaller than one turn still gives it.
    fn stepped(
        &self,
        sql: &str,
        bound: impl rusqlite::Params,
        budget: &Budget,
    ) -> Result<Vec<Turn>, StoreError> {
        let mut statement = self.db().prepare(sql)?;
        let mut rows = statement.query(bound)?;
        let mut taken: Vec<Turn> = Vec::new();
        let mut tokens = 0;
        while let Some(row) = rows.next()? {
            let turn = turn_of(row)?;
            let cost = tokens_of(&turn);
            let over = tokens + cost > budget.tokens || taken.len() >= budget.turns;
            if over && !taken.is_empty() {
                break;
            }
            tokens += cost;
            taken.push(turn);
        }
        Ok(taken)
    }

    /// Turns at or before `until`, newest first, stopping as soon as the budget is spent.
    ///
    /// The statement is stepped rather than collected, so what is not returned is not read.
    fn newest_first(
        &self,
        session: &SessionId,
        until: u64,
        budget: &Budget,
    ) -> Result<Vec<Turn>, StoreError> {
        self.stepped(
            &format!(
                "SELECT {} FROM turn WHERE session = ?1 AND cursor <= ?2 ORDER BY cursor DESC",
                crate::transcript::COLUMNS
            ),
            params![session.as_str(), clamp(until)],
            budget,
        )
    }

    /// How many turns this run has at or before `until`, counted over the cursor index.
    fn how_many_up_to(&self, session: &SessionId, until: u64) -> Result<usize, StoreError> {
        let ceiling = i64::try_from(until).unwrap_or(i64::MAX);
        let held: i64 = self.db().query_row(
            "SELECT count(*) FROM turn WHERE session = ?1 AND cursor <= ?2",
            params![session.as_str(), ceiling],
            |r| r.get(0),
        )?;
        Ok(held.max(0) as usize)
    }

    /// Drop a leading part-message, so a read never starts in the middle of one.
    fn drop_a_leading_part_message(
        &self,
        session: &SessionId,
        turns: &mut Vec<Turn>,
    ) -> Result<(), StoreError> {
        let Some(first) = turns.first() else {
            return Ok(());
        };
        let Some(entry) = first.entry.clone() else {
            return Ok(());
        };
        let started_earlier: bool = self.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM turn \
             WHERE session = ?1 AND entry = ?2 AND cursor < ?3)",
            params![
                session.as_str(),
                entry,
                i64::try_from(first.cursor).unwrap_or(i64::MAX)
            ],
            |r| r.get::<_, i64>(0).map(|held| held != 0),
        )?;
        if started_earlier {
            turns.retain(|t| t.entry.as_ref() != Some(&entry));
        }
        Ok(())
    }
}

/// A cursor as SQLite holds it. Clamped, because SQLite has no unsigned integer and
/// `u64::MAX as i64` is -1, which makes `cursor <= ?` match nothing.
fn clamp(cursor: u64) -> i64 {
    i64::try_from(cursor).unwrap_or(i64::MAX)
}

/// The most terms a `matching` read searches for.
pub const MOST_TERMS: usize = 32;
