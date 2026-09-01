//! Reading part of a scrollback, when the whole of it will not fit.
//!
//! `replay` hands back every turn a run ever produced. That is right for restoring a session and
//! wrong for everything else: a long run is unbounded by construction — memo is the only copy —
//! and a model's context is not. So the reads here are bounded by a **token budget**, because
//! that is the constraint that actually exists; bounding by turns instead would let one pasted
//! stack trace eat the whole allowance.
//!
//! Four questions, and they are the ones a harness actually asks:
//!
//! - **the tail** — what was just happening, for a run picking up where it left off;
//! - **a span** — a cursor range, which is what an episode is;
//! - **around a cursor** — the context for a citation, so `memo why` can quote in situ;
//! - **matching** — turns that mention something, within one run.
//!
//! Every read says what it left out. A bounded read that silently drops half a session is worse
//! than an unbounded one, because the caller cannot tell the difference between "that is all
//! there was" and "that is all you asked for".

use crate::{StoreError, Transcript, Turn};
use memo_model::SessionId;
use rusqlite::params;

/// Characters per token, for estimating what a turn costs.
///
/// The same rough four as everywhere else in memo. Wrong in the third decimal and right about
/// the order, which is what a budget needs.
const CHARS_PER_TOKEN: usize = 4;

/// How much may come back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    /// The token allowance.
    pub tokens: usize,
    /// A hard cap on turns, whatever the tokens say.
    ///
    /// Ten thousand one-word turns fit a generous token budget and are still useless to read,
    /// and assembling them costs the memory the budget was protecting.
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
    ///
    /// A scan rather than an index. The transcript is already the largest thing memo stores and
    /// a full-text index over it would roughly double that; within one run a scan is bounded by
    /// the run, and *across* runs the right entry point is the memory store — its witnesses
    /// carry cursors, and it already has an index.
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
    /// Where to continue from, when the budget stopped it short.
    ///
    /// `Some` means there is more in the direction being read. A caller that wants the rest asks
    /// again from here, which is what makes a long span readable in chunks rather than not at
    /// all.
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

/// Drop a leading part-message.
///
/// One assistant message is several blocks — prose, a thought, three tool calls — written as
/// separate turns so a span can address one of them. A read that cut into the middle would hand
/// a model an assistant turn without the tool call it made, or a tool result with nothing that
/// asked for it, which is worse than not showing the message at all.
///
/// Only the front is trimmed. A read growing backwards from the end stops where the budget runs
/// out, and that edge is the one that lands mid-message; the other edge is the end of the run.
fn on_a_boundary(turns: &mut Vec<Turn>, all: &[Turn]) {
    let Some(first) = turns.first() else { return };
    let Some(entry) = first.entry.clone() else {
        return;
    };
    // Whether anything earlier belongs to the same message. If not, this already starts on a
    // boundary and nothing needs dropping.
    let started_earlier = all
        .iter()
        .any(|t| t.cursor < first.cursor && t.entry.as_ref() == Some(&entry));
    if started_earlier {
        turns.retain(|t| t.entry.as_ref() != Some(&entry));
    }
}

/// What a turn costs.
#[must_use]
pub fn tokens_of(turn: &Turn) -> usize {
    turn.text.len().div_ceil(CHARS_PER_TOKEN)
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

    /// Turns from `from` onwards, stopping at `to` or at the budget.
    fn forwards(
        &self,
        session: &SessionId,
        from: u64,
        to: Option<u64>,
        budget: &Budget,
    ) -> Result<Read, StoreError> {
        let ceiling = to.unwrap_or(u64::MAX);
        let all = self.range(session, from, ceiling)?;
        let mut turns = Vec::new();
        let mut tokens = 0;
        for turn in all.iter() {
            let cost = tokens_of(turn);
            // Always take the first, however large. A budget smaller than one turn should give
            // that turn and say the budget was exceeded, not give nothing and look empty.
            if !turns.is_empty() && (tokens + cost > budget.tokens || turns.len() >= budget.turns) {
                break;
            }
            tokens += cost;
            turns.push(turn.clone());
        }
        let omitted = all.len() - turns.len();
        let next = (omitted > 0).then(|| turns.last().map_or(from, |t| t.cursor + 1));
        Ok(Read {
            turns,
            tokens,
            omitted,
            next,
        })
    }

    /// Turns ending at `until`, taken from the end backwards.
    fn backwards(
        &self,
        session: &SessionId,
        until: u64,
        budget: &Budget,
    ) -> Result<Read, StoreError> {
        let all = self.range(session, 0, until)?;
        let mut turns = Vec::new();
        let mut tokens = 0;
        for turn in all.iter().rev() {
            let cost = tokens_of(turn);
            if !turns.is_empty() && (tokens + cost > budget.tokens || turns.len() >= budget.turns) {
                break;
            }
            tokens += cost;
            turns.push(turn.clone());
        }
        turns.reverse();
        on_a_boundary(&mut turns, &all);
        let omitted = all.len() - turns.len();
        // Reading backwards, "more" is what came *before* — so continuing means asking for the
        // span that ends just before the earliest turn returned.
        let next = (omitted > 0).then(|| turns.first().map_or(0, |t| t.cursor.saturating_sub(1)));
        Ok(Read {
            turns,
            tokens,
            omitted,
            next,
        })
    }

    /// The turns either side of one cursor.
    ///
    /// Grown outward a turn at a time rather than by a fixed radius, so a citation surrounded by
    /// long turns gets fewer of them and one surrounded by short turns gets more. What a reader
    /// needs is a paragraph of context, not a turn count.
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
        let before = self.range(session, 0, cursor.saturating_sub(1))?;
        let after = self.range(session, cursor + 1, u64::MAX)?;

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

        let total = before.len() + 1 + after.len();
        Ok(Read {
            omitted: total - taken.len(),
            turns: taken.into_iter().collect(),
            tokens,
            next: None,
        })
    }

    /// Turns mentioning every term.
    fn matching(
        &self,
        session: &SessionId,
        terms: &[String],
        budget: &Budget,
    ) -> Result<Read, StoreError> {
        let all = self.range(session, 0, u64::MAX)?;
        let wanted: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();
        let hits: Vec<&Turn> = all
            .iter()
            .filter(|turn| {
                let text = turn.text.to_lowercase();
                !wanted.is_empty() && wanted.iter().all(|term| text.contains(term))
            })
            .collect();

        let mut turns = Vec::new();
        let mut tokens = 0;
        for turn in &hits {
            let cost = tokens_of(turn);
            if !turns.is_empty() && (tokens + cost > budget.tokens || turns.len() >= budget.turns) {
                break;
            }
            tokens += cost;
            turns.push((*turn).clone());
        }
        let omitted = hits.len() - turns.len();
        let next = (omitted > 0).then(|| turns.last().map_or(0, |t| t.cursor + 1));
        Ok(Read {
            turns,
            tokens,
            omitted,
            next,
        })
    }

    /// Every turn in a cursor range, in order.
    fn range(&self, session: &SessionId, from: u64, to: u64) -> Result<Vec<Turn>, StoreError> {
        let mut statement = self.db().prepare(
            "SELECT cursor, at, role, kind, text, tool, raw, entry, revisions FROM turn \
             WHERE session = ?1 AND cursor >= ?2 AND cursor <= ?3 ORDER BY cursor",
        )?;
        // Clamped, because SQLite has no unsigned integer and `u64::MAX as i64` is -1 — which
        // made `cursor <= ?3` match nothing, so every read that scanned to the end came back
        // empty while a bounded span worked fine.
        let ceiling = i64::try_from(to).unwrap_or(i64::MAX);
        let floor = i64::try_from(from).unwrap_or(i64::MAX);
        let rows = statement
            .query_map(params![session.as_str(), floor, ceiling], |r| {
                Ok(Turn {
                    cursor: r.get::<_, i64>(0)? as u64,
                    at: r.get(1)?,
                    role: r.get(2)?,
                    kind: r.get(3)?,
                    text: r.get(4)?,
                    tool: r.get(5)?,
                    raw: r.get(6)?,
                    entry: r.get(7)?,
                    revisions: r.get::<_, i64>(8)? as u32,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
