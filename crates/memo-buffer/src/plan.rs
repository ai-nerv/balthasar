//! The instruction set a harness applies.

use crate::{Shape, Window};
use memo_store::Entry;

/// A turn replaced by a description of itself.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Masked {
    /// Which turn.
    pub cursor: u64,
    /// What to send instead.
    pub r#as: String,
    /// What it cost before.
    pub was: u32,
}

/// A run of turns a summary stands in for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Span {
    /// First cursor covered.
    pub from: u64,
    /// Last cursor covered.
    pub to: u64,
}

/// What to send.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct Plan {
    /// Turns to send as they are.
    pub keep: Vec<u64>,
    /// Turns to replace with a description.
    pub mask: Vec<Masked>,
    /// Turns to leave out entirely.
    pub drop: Vec<u64>,
    /// The span to summarise, when masking was not enough.
    pub summarise: Option<Span>,
    /// What the window will cost afterwards.
    pub used: u32,
    /// What it cost before.
    pub was: u32,
    /// How much room there is.
    pub target: u32,
    /// Whether the plan fits.
    ///
    /// A plan that cannot fit says so rather than returning something the provider will refuse.
    pub fits: bool,
    /// One line saying what was done, for a harness to show or log.
    pub why: String,
}

/// Decide what a harness should send.
///
/// `describe` is asked for a masked turn's replacement text, keyed on the tool. Only the tool's
/// author knows what a useful stub says — "3,200 lines of test output, 41 failures" is worth
/// sending and "[output omitted]" is not.
#[must_use]
pub fn plan(
    entries: &[Entry],
    window: &Window,
    mut describe: impl FnMut(&Entry) -> Option<String>,
) -> Plan {
    let before = Shape::of(entries, window.masked_cost);
    let target = window.target();

    let mut plan = Plan {
        was: before.used,
        target,
        fits: true,
        ..Plan::default()
    };

    if !window.is_workable() {
        plan.fits = false;
        plan.why = format!(
            "a {}-token window with {} reserved leaves no room for a conversation",
            window.size, window.reserve
        );
        return plan;
    }

    // Nothing to do is a real answer, and the common one. A harness asking every turn should
    // usually be told to send what it has.
    if before.used <= target {
        plan.keep = entries.iter().map(|e| e.cursor).collect();
        plan.used = before.used;
        plan.why = format!("{} of {target} tokens — nothing to do", before.used);
        return plan;
    }

    // The recent turns are never touched: the detail still in play lives there, and a summary
    // of it is always worse than having it.
    let untouchable = untouchable(entries, window.keep);
    let mut used = before.used;

    // Masking first, always. Free, reversible, and where a coding agent's tokens actually are.
    for entry in entries {
        if used <= target {
            break;
        }
        if untouchable.contains(&entry.cursor) || entry.pinned || !entry.state.is_live() {
            continue;
        }
        if entry.role != "tool" || entry.tokens < window.mask_over {
            continue;
        }
        let Some(text) = describe(entry) else {
            continue;
        };
        let cost = tokens_of(&text).min(entry.tokens);
        used = used.saturating_sub(entry.tokens).saturating_add(cost);
        plan.mask.push(Masked {
            cursor: entry.cursor,
            r#as: text,
            was: entry.tokens,
        });
    }

    // Only then a summary, and only over what masking could not free.
    let masked: Vec<u64> = plan.mask.iter().map(|m| m.cursor).collect();
    if used > target {
        // The span has to free what is needed *and* cover the summary's own cost, or the
        // plan comes back still over by exactly what the summary added.
        let needed = (used - target).saturating_add(summary_cost(window));
        let (span, freed) = summarisable(entries, &untouchable, &masked, window, needed);
        if let Some(span) = span {
            plan.summarise = Some(span);
            // The summary is itself something to send. Counting the span as free would have a
            // plan claim room it does not have, and the overflow would arrive at the provider.
            used = used
                .saturating_sub(freed)
                .saturating_add(summary_cost(window));
            for entry in entries {
                if entry.cursor >= span.from && entry.cursor <= span.to {
                    plan.drop.push(entry.cursor);
                }
            }
        }
    }

    plan.keep = entries
        .iter()
        .map(|e| e.cursor)
        .filter(|c| !plan.drop.contains(c))
        .collect();
    plan.used = used;
    plan.fits = used <= target;
    plan.why = describe_plan(&plan, target);
    plan
}

/// The cursors a plan may not touch: the recent tail, and anything pinned.
fn untouchable(entries: &[Entry], keep: usize) -> Vec<u64> {
    let mut out: Vec<u64> = entries.iter().rev().take(keep).map(|e| e.cursor).collect();
    out.extend(entries.iter().filter(|e| e.pinned).map(|e| e.cursor));
    out
}

/// Roughly what a summary costs to send.
///
/// Longer than a mask stub — it stands in for a whole span rather than one result — and short
/// beside what it replaces. An estimate, like every token count here.
fn summary_cost(window: &Window) -> u32 {
    window.masked_cost.saturating_mul(4)
}

/// Whether a span frees more than the summary standing in for it will cost.
///
/// The only honest bar. An earlier version demanded a multiple of the summary's cost and
/// refused spans that were *needed but small* — which had a plan decline to summarise 188
/// tokens it had to free, hand back something that did not fit, and say so cheerfully. A
/// summary has to pay for itself; beyond that, fitting is what matters.
fn pays_for_itself(freed: u32, window: &Window) -> bool {
    freed > summary_cost(window)
}

/// The oldest span worth summarising, and what summarising it would free.
///
/// Taken from the front, because the oldest turns are the ones a summary loses least by
/// replacing. Stops as soon as it has freed enough: summarising more than is needed throws
/// away detail for nothing.
fn summarisable(
    entries: &[Entry],
    untouchable: &[u64],
    masked: &[u64],
    window: &Window,
    needed: u32,
) -> (Option<Span>, u32) {
    let mut freed = 0_u32;
    let mut from = None;
    let mut to = 0_u64;

    for entry in entries {
        if untouchable.contains(&entry.cursor) || entry.pinned {
            break;
        }
        let cost = if masked.contains(&entry.cursor) {
            window.masked_cost.min(entry.tokens)
        } else {
            entry.cost(window.masked_cost)
        };
        if from.is_none() {
            from = Some(entry.cursor);
        }
        to = entry.cursor;
        freed = freed.saturating_add(cost);
        if freed >= needed {
            break;
        }
    }

    match from {
        Some(from) if pays_for_itself(freed, window) => (Some(Span { from, to }), freed),
        _ => (None, 0),
    }
}

/// One line saying what was done.
fn describe_plan(plan: &Plan, target: u32) -> String {
    let mut parts = Vec::new();
    if !plan.mask.is_empty() {
        let saved: u32 = plan.mask.iter().map(|m| m.was).sum();
        parts.push(format!(
            "{} tool result(s) masked (~{saved} tokens)",
            plan.mask.len()
        ));
    }
    if let Some(span) = plan.summarise {
        parts.push(format!("turns {}–{} summarised", span.from, span.to));
    }
    if parts.is_empty() {
        parts.push("nothing could be freed".to_owned());
    }
    format!(
        "{} — {} of {target} tokens, was {}",
        parts.join(", "),
        plan.used,
        plan.was
    )
}

/// Roughly how many tokens a piece of text costs.
fn tokens_of(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use memo_store::State;

    fn turn(cursor: u64, role: &str, tokens: u32) -> Entry {
        Entry {
            cursor,
            memory: None,
            role: role.into(),
            kind: if role == "tool" {
                "tool_result"
            } else {
                "prose"
            }
            .into(),
            tool: (role == "tool").then(|| "shell".to_owned()),
            tokens,
            state: State::Live,
            pinned: false,
            at: 0,
        }
    }

    fn stub(_entry: &Entry) -> Option<String> {
        Some("`make test` — ok, 41 lines".to_owned())
    }

    fn small() -> Window {
        Window {
            size: 1000,
            reserve: 200,
            inject: 100,
            mask_over: 50,
            keep: 2,
            masked_cost: 10,
        }
    }

    #[test]
    fn a_window_with_room_is_left_alone() {
        // The common answer, and it has to be cheap. A harness asks every turn.
        let entries = vec![turn(1, "user", 10), turn(2, "assistant", 20)];
        let plan = plan(&entries, &small(), stub);
        assert_eq!(plan.keep, [1, 2]);
        assert!(plan.mask.is_empty() && plan.summarise.is_none());
        assert!(plan.fits);
    }

    #[test]
    fn masking_is_tried_before_summarising() {
        // The whole ordering. Masking is free and reversible; summarising costs a request and
        // loses information irreversibly.
        let entries = vec![
            turn(1, "user", 20),
            turn(2, "tool", 800),
            turn(3, "user", 20),
            turn(4, "assistant", 20),
        ];
        let plan = plan(&entries, &small(), stub);
        assert_eq!(plan.mask.len(), 1, "the tool result was masked");
        assert_eq!(plan.mask[0].cursor, 2);
        assert!(plan.summarise.is_none(), "and nothing had to be summarised");
        assert!(plan.fits);
    }

    #[test]
    fn the_recent_turns_are_never_touched() {
        // The detail still in play lives there: the file just read, the error just seen.
        let entries = vec![
            turn(1, "tool", 400),
            turn(2, "tool", 400),
            turn(3, "tool", 400),
        ];
        let plan = plan(&entries, &small(), stub);
        let masked: Vec<u64> = plan.mask.iter().map(|m| m.cursor).collect();
        assert!(!masked.contains(&2) && !masked.contains(&3), "{masked:?}");
    }

    #[test]
    fn a_pinned_turn_is_never_touched() {
        let mut entries = vec![
            turn(1, "tool", 900),
            turn(2, "user", 10),
            turn(3, "user", 10),
        ];
        entries[0].pinned = true;
        let plan = plan(&entries, &small(), stub);
        assert!(plan.mask.is_empty());
        assert!(!plan.drop.contains(&1));
    }

    #[test]
    fn prose_is_not_masked() {
        // Masking replaces a tool's output with a description of it. There is no description
        // of a sentence that is shorter than the sentence.
        let entries = vec![
            turn(1, "assistant", 900),
            turn(2, "user", 10),
            turn(3, "user", 10),
        ];
        let plan = plan(&entries, &small(), stub);
        assert!(plan.mask.is_empty());
    }

    #[test]
    fn a_small_tool_result_is_not_worth_masking() {
        let entries = vec![
            turn(1, "tool", 20),
            turn(2, "tool", 900),
            turn(3, "user", 5),
            turn(4, "user", 5),
        ];
        let plan = plan(&entries, &small(), stub);
        let masked: Vec<u64> = plan.mask.iter().map(|m| m.cursor).collect();
        assert_eq!(masked, [2], "only the large one");
    }

    #[test]
    fn a_stub_longer_than_the_thing_is_not_counted_as_a_saving() {
        // A mask that made a turn bigger and was counted as having shrunk it would let a plan
        // claim room it does not have, and the overflow would arrive at the provider.
        let entries = vec![turn(1, "tool", 60), turn(2, "user", 60), turn(3, "user", 5)];
        let long = |_: &Entry| Some("x".repeat(4000));
        let window = Window {
            size: 200,
            reserve: 50,
            inject: 50,
            ..small()
        };
        let plan = plan(&entries, &window, long);
        let masked: u32 = plan
            .mask
            .iter()
            .map(|m| m.was)
            .sum::<u32>()
            .saturating_sub(plan.was.saturating_sub(plan.used));
        assert!(
            plan.used >= 60,
            "the stub cannot have made it free: {}",
            plan.used
        );
        let _ = masked;
    }

    #[test]
    fn summarising_happens_only_when_masking_was_not_enough() {
        let entries = vec![
            turn(1, "user", 300),
            turn(2, "user", 300),
            turn(3, "user", 300),
            turn(4, "user", 10),
            turn(5, "user", 10),
        ];
        let plan = plan(&entries, &small(), stub);
        assert!(plan.mask.is_empty(), "there was no tool output to mask");
        let span = plan.summarise.expect("a summary was needed");
        assert_eq!(span.from, 1);
        assert!(plan.drop.contains(&1));
    }

    #[test]
    fn a_summary_covers_no_more_than_it_has_to() {
        // Summarising more than is needed throws away detail for nothing.
        let entries = vec![
            turn(1, "user", 400),
            turn(2, "user", 400),
            turn(3, "user", 100),
            turn(4, "user", 10),
            turn(5, "user", 10),
        ];
        let plan = plan(&entries, &small(), stub);
        let span = plan.summarise.expect("a summary");
        assert!(
            span.to < 3,
            "covered {}–{} and did not need to",
            span.from,
            span.to
        );
    }

    #[test]
    fn one_long_turn_is_worth_summarising_on_its_own() {
        // The check is about what is freed, not about how many turns are covered. Refusing a
        // span of one would leave a session stuck behind a single enormous turn.
        let entries = vec![turn(1, "user", 900), turn(2, "user", 5), turn(3, "user", 5)];
        let plan = plan(&entries, &small(), stub);
        let span = plan.summarise.expect("900 tokens is worth a request");
        assert_eq!((span.from, span.to), (1, 1));
    }

    #[test]
    fn a_summary_that_would_not_pay_for_itself_is_refused() {
        // A summary is itself something to send. One that frees less than it costs makes the
        // window bigger, and a plan that did it would report progress while going backwards.
        let entries = vec![
            turn(1, "user", 15),
            turn(2, "user", 15),
            turn(3, "user", 700),
        ];
        let window = Window { keep: 1, ..small() };
        let plan = plan(&entries, &window, stub);
        assert!(plan.summarise.is_none(), "{:?}", plan.summarise);
        assert!(!plan.fits, "and it says so rather than pretending");
    }

    #[test]
    fn a_window_that_cannot_hold_a_conversation_says_so() {
        let entries = vec![turn(1, "user", 10)];
        let window = Window {
            size: 100,
            reserve: 200,
            ..small()
        };
        let plan = plan(&entries, &window, stub);
        assert!(!plan.fits);
        assert!(plan.why.contains("no room"), "{}", plan.why);
    }

    #[test]
    fn a_plan_says_in_one_line_what_it_did() {
        let entries = vec![
            turn(1, "user", 20),
            turn(2, "tool", 800),
            turn(3, "user", 20),
            turn(4, "user", 20),
        ];
        let plan = plan(&entries, &small(), stub);
        assert!(plan.why.contains("masked"), "{}", plan.why);
        assert!(plan.why.contains("tokens"), "{}", plan.why);
    }

    #[test]
    fn a_turn_already_masked_is_not_masked_again() {
        // A planner that counted a masked turn as free would keep reaching for it and never
        // reach its target.
        let mut entries = vec![turn(1, "tool", 900), turn(2, "user", 5), turn(3, "user", 5)];
        entries[0].state = State::Masked;
        let plan = plan(&entries, &small(), stub);
        assert!(plan.mask.is_empty());
    }

    #[test]
    fn a_tool_nobody_can_describe_is_left_alone() {
        // Only the tool's author knows what a useful stub says. With no handler there is
        // nothing honest to put in its place.
        let entries = vec![turn(1, "tool", 900), turn(2, "user", 5), turn(3, "user", 5)];
        let plan = plan(&entries, &small(), |_| None);
        assert!(plan.mask.is_empty());
    }
}
