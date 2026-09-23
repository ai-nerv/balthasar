//! A layout: what one request holds, slot by slot.
//!
//! Pure. The host hands in the rows, what the other slots cost and whether a summary may be
//! asked for; this decides which rows go word for word, which are stubbed, dropped or due for a
//! summary, and where the slots sit. Nothing here is recorded.

use crate::{Rules, Span};
use std::collections::{BTreeSet, HashMap, HashSet};

/// What a harness asked with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ask {
    /// 0 for the first request of a prompt.
    pub round: u32,
    pub window: u32,
    /// What the reply may take.
    pub reply: u32,
    /// The system prompt and the tool schemas.
    pub fixed: u32,
    /// Seconds since the last request.
    pub idle_s: Option<u64>,
    /// How often this prompt has overflowed; each time tightens the rules.
    pub tight: u32,
}

/// One row, as the rules see it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub cursor: u64,
    /// The group it travels with.
    pub unit: u64,
    /// What it costs word for word, before the factor.
    pub tokens: u32,
    /// What it costs stubbed, before the factor.
    pub stub_tokens: u32,
    pub tool: bool,
    pub user: bool,
    pub keep: bool,
    pub error: bool,
    /// Stubbed in a layout that was applied, so it stays stubbed.
    pub stubbed: bool,
}

/// What a stored summary stands in for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Covered {
    pub from: u64,
    pub to: u64,
    pub tokens: u32,
}

impl Covered {
    fn covers(&self, cursor: u64) -> bool {
        (self.from..=self.to).contains(&cursor)
    }
}

/// Whether another summary may be asked for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Compacting {
    #[default]
    Allowed,
    /// One is already on its way.
    Pending,
    /// This prompt has had its share.
    Spent,
}

/// What the other slots hold, before the factor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Held {
    pub summary: Option<Covered>,
    pub pinned: u32,
    pub memory: u32,
    /// What the warning note costs, should it be needed.
    pub note: u32,
    pub compacting: Compacting,
}

/// How the room is split and what is spent, after the factor.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Budget {
    pub window: u32,
    pub fixed: u32,
    pub reply: u32,
    pub room: u32,
    pub conversation: u32,
    pub memory: u32,
    pub pinned: u32,
    pub summary: u32,
    pub used: u32,
    pub estimated_input: u32,
    pub factor: f64,
}

/// One place in the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Pinned,
    Summary,
    Item(u64),
    Stub(u64),
    Note,
    Memory,
}

/// What one request holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub budget: Budget,
    pub slots: Vec<Slot>,
    /// The span a summary is wanted for, when one is.
    pub compact: Option<Span>,
    pub stubbed: Vec<u64>,
    pub dropped: Vec<u64>,
    pub warned: bool,
    pub fits: bool,
    pub why: String,
}

/// A count after the factor.
#[must_use]
pub fn corrected(tokens: u32, factor: f64) -> u32 {
    let scaled = (f64::from(tokens) * factor).ceil();
    if scaled >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        scaled.max(0.0) as u32
    }
}

/// The room, and what each slot may hold of it.
#[must_use]
pub fn budget(rules: &Rules, ask: &Ask, factor: f64) -> Budget {
    let fixed = corrected(ask.fixed, factor);
    // The whole input is held under `L` — a share of the window, not merely what is left of it
    // after the fixed costs and the reply. Planning to the leftover puts the fixed costs outside
    // the budget, and the boundary that counts the request counts them too.
    let limit = crate::policy::Room::of(rules.policy, Some(ask.window), ask.reply)
        .map_or(0, |room| room.limit);
    let room = limit.saturating_sub(fixed);
    let part = |share: f64| (f64::from(room) * share).round() as u32;
    let (memory, pinned, summary) = (
        part(rules.shares.memory),
        part(rules.shares.pinned),
        part(rules.shares.summary),
    );
    Budget {
        window: ask.window,
        fixed,
        reply: ask.reply,
        room,
        conversation: room.saturating_sub(memory + pinned + summary),
        memory,
        pinned,
        summary,
        used: 0,
        estimated_input: fixed,
        factor,
    }
}

/// Lay out one request by the rules.
#[must_use]
pub fn lay(rules: &Rules, ask: &Ask, rows: &[Row], held: &Held, factor: f64) -> Layout {
    let mut budget = budget(rules, ask, factor);
    let fix = |tokens: u32| corrected(tokens, factor);
    // A summary stands in for its span only while it is smaller than the span. Summarising a few
    // short turns costs more than keeping them, and a compaction that grows the request is a
    // compaction that should not have happened.
    let held = &worth_it(held, rows, factor);
    let shown: Vec<&Row> = rows
        .iter()
        .filter(|r| !held.summary.is_some_and(|s| s.covers(r.cursor)))
        .collect();

    // What the memory and pinned slots leave unspent is the conversation's.
    let summary = held.summary.map_or(0, |s| fix(s.tokens));
    let room = budget
        .room
        .saturating_sub(fix(held.memory).min(budget.memory))
        .saturating_sub(fix(held.pinned).min(budget.pinned))
        .saturating_sub(budget.summary.max(summary));
    budget.conversation = room;
    let of = |share: f64| (f64::from(room) * share.max(0.0)) as u32;

    let tight = ask.tight;
    let scale = 0.85_f64.powi(i32::try_from(tight).unwrap_or(i32::MAX));
    let protected = protected(
        &shown,
        rules.keep_turns.saturating_sub(tight as usize).max(1),
        rules.keep_results.saturating_sub(tight as usize).max(1),
    );
    let cold = ask.idle_s.is_some_and(|idle| idle > rules.cache_ttl_s);
    let mut why = Vec::new();

    let mut stubbed: BTreeSet<u64> = shown
        .iter()
        .filter(|r| r.stubbed && r.tool)
        .map(|r| r.cursor)
        .collect();
    let cost = |row: &Row, stubbed: &BTreeSet<u64>| {
        if stubbed.contains(&row.cursor) {
            fix(row.stub_tokens)
        } else {
            fix(row.tokens)
        }
    };
    let mut conversation: u32 = shown.iter().map(|r| cost(r, &stubbed)).sum();

    // Over prune_at: the largest old tool results, in one batch worth breaking the cache for.
    if conversation > of(rules.prune_at * scale) {
        let target = if cold {
            of(rules.compact_to * scale)
        } else {
            of(rules.prune_at * scale - rules.prune_min)
        };
        let mut candidates: Vec<&Row> = shown
            .iter()
            .copied()
            .filter(|r| {
                r.tool
                    && !r.keep
                    && !r.error
                    && !protected.contains(&r.cursor)
                    && !stubbed.contains(&r.cursor)
                    && r.tokens > rules.stub_over
                    && r.stub_tokens < r.tokens
            })
            .collect();
        // The prompt in progress reads last: what it just read is what it is working from.
        let asked = shown.iter().rev().find(|r| r.user).map_or(0, |r| r.cursor);
        candidates.sort_by(|a, b| {
            (a.cursor >= asked)
                .cmp(&(b.cursor >= asked))
                .then(b.tokens.cmp(&a.tokens))
                .then(a.cursor.cmp(&b.cursor))
        });
        let before = conversation;
        let mut fresh = Vec::new();
        for row in candidates {
            if conversation <= target {
                break;
            }
            conversation = conversation - fix(row.tokens) + fix(row.stub_tokens);
            fresh.push(row.cursor);
        }
        let freed = before - conversation;
        if cold || tight > 0 || before > room || freed >= of(rules.prune_min) {
            stubbed.extend(fresh);
        } else {
            conversation = before;
            if freed > 0 {
                why.push(format!(
                    "stubbing would free only ~{}, so none yet",
                    thousands(freed)
                ));
            }
        }
    }

    // Over compact_at: a summary of the oldest whole groups, enough to reach compact_to. It rolls
    // forward from the stored one; protected groups ahead of it stay word for word before it.
    let units = units(&shown);
    let mut compact = None;
    if conversation > of(rules.compact_at * scale) {
        match held.compacting {
            Compacting::Allowed => {
                let goal = of(rules.compact_to * scale);
                let (mut freed, mut first, mut last) = (0, None, None);
                for unit in &units {
                    let head = unit[0].cursor;
                    let guarded = unit.iter().any(|r| protected.contains(&r.cursor));
                    if first.is_none() {
                        match held.summary {
                            Some(s) if head < s.from => continue,
                            None if guarded => continue,
                            _ => {}
                        }
                    }
                    if guarded || conversation.saturating_sub(freed) <= goal {
                        break;
                    }
                    first.get_or_insert(head);
                    freed += unit.iter().map(|r| cost(r, &stubbed)).sum::<u32>();
                    last = unit.iter().map(|r| r.cursor).max();
                }
                if let (Some(start), Some(to)) = (first, last) {
                    let from = held.summary.map_or(start, |s| s.from);
                    compact = Some(Span { from, to });
                    why.push(format!("summary of {from}–{to} wanted"));
                }
            }
            Compacting::Pending => why.push("a summary is on its way".to_owned()),
            Compacting::Spent => why.push("no more summaries this prompt".to_owned()),
        }
    }

    // Still over the room with no summary for it: the oldest whole groups go, for this request.
    let mut dropped = BTreeSet::new();
    if conversation > room {
        let mut groups = 0;
        for unit in &units {
            if conversation <= room {
                break;
            }
            if unit
                .iter()
                .any(|r| protected.contains(&r.cursor) || r.keep || r.error)
            {
                continue;
            }
            conversation -= unit.iter().map(|r| cost(r, &stubbed)).sum::<u32>();
            dropped.extend(unit.iter().map(|r| r.cursor));
            groups += 1;
        }
        stubbed.retain(|cursor| !dropped.contains(cursor));
        if groups > 0 {
            why.push(format!(
                "{groups} oldest groups dropped until a summary exists"
            ));
        }
    }

    let warned = conversation > of(rules.warn_at);
    let slots = arrange(&shown, held, &stubbed, &dropped, warned, budget);
    let stubs: u32 = shown
        .iter()
        .filter(|r| stubbed.contains(&r.cursor))
        .map(|r| fix(r.tokens) - fix(r.stub_tokens).min(fix(r.tokens)))
        .sum();
    if !stubbed.is_empty() {
        why.insert(
            0,
            format!(
                "{} tool results stubbed (~{})",
                stubbed.len(),
                thousands(stubs)
            ),
        );
    }
    if let Some(s) = held.summary {
        why.insert(0, format!("summary {}–{}", s.from, s.to));
    }
    if cold && !stubbed.is_empty() {
        why.push("cache cold".to_owned());
    }
    if tight > 0 {
        why.push(format!("tightened after {tight} overflow(s)"));
    }
    if warned {
        why.push("warned".to_owned());
    }

    let mut layout = Layout {
        budget,
        slots,
        compact,
        stubbed: stubbed.into_iter().collect(),
        dropped: dropped.into_iter().collect(),
        warned,
        fits: true,
        why: String::new(),
    };
    account(&mut layout, rows, held, factor);
    let share = u64::from(layout.budget.estimated_input) * 100 / u64::from(ask.window.max(1));
    if why.is_empty() {
        why.push("everything word for word".to_owned());
    }
    why.push(format!("{share}% of the window"));
    layout.why = why.join("; ");
    layout
}

/// Fill in what a layout spends, and whether it fits.
pub fn account(layout: &mut Layout, rows: &[Row], held: &Held, factor: f64) {
    let fix = |tokens: u32| corrected(tokens, factor);
    let by: HashMap<u64, &Row> = rows.iter().map(|r| (r.cursor, r)).collect();
    let used: u32 = layout
        .slots
        .iter()
        .map(|slot| match slot {
            Slot::Pinned => fix(held.pinned),
            Slot::Summary => held.summary.map_or(0, |s| fix(s.tokens)),
            Slot::Item(cursor) => by.get(cursor).map_or(0, |r| fix(r.tokens)),
            Slot::Stub(cursor) => by.get(cursor).map_or(0, |r| fix(r.stub_tokens)),
            Slot::Note => fix(held.note),
            Slot::Memory => fix(held.memory),
        })
        .sum();
    let budget = &mut layout.budget;
    budget.used = used;
    budget.estimated_input = used.saturating_add(budget.fixed);
    layout.fits =
        u64::from(budget.estimated_input) + u64::from(budget.reply) <= u64::from(budget.window);
}

/// Slots in the order the prompt cache wants: pinned, the conversation with the summary where
/// its span was, and the note and memory just before the latest user message.
fn arrange(
    shown: &[&Row],
    held: &Held,
    stubbed: &BTreeSet<u64>,
    dropped: &BTreeSet<u64>,
    warned: bool,
    budget: Budget,
) -> Vec<Slot> {
    let latest = shown
        .iter()
        .rev()
        .find(|r| r.user && !dropped.contains(&r.cursor))
        .map(|r| r.cursor);
    let tail = |slots: &mut Vec<Slot>| {
        if warned {
            slots.push(Slot::Note);
        }
        if held.memory > 0 && budget.memory > 0 {
            slots.push(Slot::Memory);
        }
    };
    let mut slots = Vec::new();
    if held.pinned > 0 && budget.pinned > 0 {
        slots.push(Slot::Pinned);
    }
    let mut summarised = held.summary.is_none();
    let mut placed = false;
    for row in shown {
        if dropped.contains(&row.cursor) {
            continue;
        }
        if !summarised && held.summary.is_some_and(|s| row.cursor > s.to) {
            slots.push(Slot::Summary);
            summarised = true;
        }
        if Some(row.cursor) == latest {
            tail(&mut slots);
            placed = true;
        }
        slots.push(if stubbed.contains(&row.cursor) {
            Slot::Stub(row.cursor)
        } else {
            Slot::Item(row.cursor)
        });
    }
    if !summarised {
        slots.push(Slot::Summary);
    }
    if !placed {
        tail(&mut slots);
    }
    slots
}

/// What is never stubbed, summarised or dropped: the last user turns, the last tool results, and
/// the newest row.
fn protected(shown: &[&Row], keep_turns: usize, keep_results: usize) -> HashSet<u64> {
    let users: Vec<u64> = shown.iter().filter(|r| r.user).map(|r| r.cursor).collect();
    let recent = &users[users.len().saturating_sub(keep_turns)..];
    // Results the recent prompts work with: one from a prompt long finished is history, and holding
    // it kept every summary at the turn before it.
    let since = recent.first().copied().unwrap_or(0);
    let results: Vec<u64> = shown
        .iter()
        .filter(|r| r.tool && r.cursor >= since)
        .map(|r| r.cursor)
        .collect();
    let mut held: HashSet<u64> = recent.iter().copied().collect();
    held.extend(&results[results.len().saturating_sub(keep_results)..]);
    held.extend(shown.last().map(|r| r.cursor));
    held
}

/// The rows grouped as they travel, oldest group first.
fn units<'a>(shown: &[&'a Row]) -> Vec<Vec<&'a Row>> {
    let mut order: Vec<Vec<&Row>> = Vec::new();
    let mut at: HashMap<u64, usize> = HashMap::new();
    for row in shown {
        if let Some(&index) = at.get(&row.unit) {
            order[index].push(row);
        } else {
            at.insert(row.unit, order.len());
            order.push(vec![row]);
        }
    }
    order
}

/// `41k` rather than `41327`.
fn thousands(tokens: u32) -> String {
    if tokens >= 1_000 {
        format!("{}k", (tokens + 500) / 1_000)
    } else {
        tokens.to_string()
    }
}

/// `held` with its summary dropped when the span it covers is still whole and no larger than the
/// summary itself.
///
/// Whole matters: once the oldest rows have been dropped, the summary is all that is left of them
/// and the live remainder is cheap for that reason, not because summarising was pointless. Both
/// ends of the span still being live is what says nothing has gone.
fn worth_it(held: &Held, rows: &[Row], factor: f64) -> Held {
    let Some(covered) = held.summary else {
        return held.clone();
    };
    let live = |cursor: u64| rows.iter().any(|row| row.cursor == cursor);
    if !live(covered.from) || !live(covered.to) {
        return held.clone();
    }
    let span: u32 = rows
        .iter()
        .filter(|row| covered.covers(row.cursor))
        .map(|row| corrected(row.tokens, factor))
        .sum();
    if span > corrected(covered.tokens, factor) {
        return held.clone();
    }
    Held {
        summary: None,
        ..held.clone()
    }
}
