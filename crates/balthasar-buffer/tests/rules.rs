//! Every rule of the layout, on synthetic rows.

use balthasar_buffer::{Ask, Compacting, Covered, Held, Row, Rules, Slot, budget, lay};

/// 157k of room and 144,440 for the conversation, which is what every rule below is sized
/// against. One request may hold half a window, so that room takes a 336k one: 168k of limit,
/// less 11k of fixed cost. The reply is inside the other half.
fn ask() -> Ask {
    Ask {
        window: 336_000,
        reply: 32_000,
        fixed: 11_000,
        ..Ask::default()
    }
}

fn user(cursor: u64, tokens: u32) -> Row {
    Row {
        cursor,
        unit: cursor,
        tokens,
        user: true,
        ..Row::default()
    }
}

fn said(cursor: u64, tokens: u32) -> Row {
    Row {
        cursor,
        unit: cursor,
        tokens,
        ..Row::default()
    }
}

fn result(cursor: u64, unit: u64, tokens: u32) -> Row {
    Row {
        cursor,
        unit,
        tokens,
        stub_tokens: 20,
        tool: true,
        ..Row::default()
    }
}

/// A first prompt, a tool call per entry of `big`, then a second prompt and three small results.
fn session(big: &[u32]) -> Vec<Row> {
    let mut rows = vec![user(0, 50)];
    let mut cursor = 1;
    for tokens in big {
        rows.push(said(cursor, 100));
        rows.push(result(cursor + 1, cursor, *tokens));
        cursor += 2;
    }
    rows.push(user(cursor, 50));
    cursor += 1;
    for _ in 0..3 {
        rows.push(said(cursor, 100));
        rows.push(result(cursor + 1, cursor, 2_000));
        cursor += 2;
    }
    rows
}

fn stubs(slots: &[Slot]) -> Vec<u64> {
    slots
        .iter()
        .filter_map(|s| match s {
            Slot::Stub(c) => Some(*c),
            _ => None,
        })
        .collect()
}

fn named(slots: &[Slot]) -> Vec<u64> {
    slots
        .iter()
        .filter_map(|s| match s {
            Slot::Item(c) | Slot::Stub(c) => Some(*c),
            _ => None,
        })
        .collect()
}

#[test]
fn under_prune_at_everything_goes_word_for_word() {
    let rows = session(&[5_000, 5_000]);
    let laid = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    assert!(stubs(&laid.slots).is_empty());
    assert_eq!(named(&laid.slots).len(), rows.len());
    assert!(laid.fits);
    assert!(
        laid.why.starts_with("everything word for word"),
        "{}",
        laid.why
    );
}

#[test]
fn over_prune_at_the_largest_old_result_is_stubbed_first() {
    let rows = session(&[40_000, 30_000, 20_000, 30_000]);
    let laid = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    // 40k alone leaves it just above the batch target; of the two 30k results the older goes.
    assert_eq!(stubs(&laid.slots), [2, 4], "{}", laid.why);
    assert!(
        laid.why.contains("2 tool results stubbed (~70k)"),
        "{}",
        laid.why
    );
}

#[test]
fn the_prompt_in_progress_keeps_its_reads_while_older_ones_are_left() {
    // Its largest reads went first, and the model lost track of what it had just read.
    let mut rows = vec![user(0, 50)];
    for cursor in [1, 3] {
        rows.push(said(cursor, 100));
        rows.push(result(cursor + 1, cursor, 30_000));
    }
    rows.push(user(5, 50));
    rows.push(said(6, 100));
    rows.push(result(7, 6, 50_000));
    for cursor in [8, 10, 12] {
        rows.push(said(cursor, 100));
        rows.push(result(cursor + 1, cursor, 2_000));
    }
    let laid = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    let stubbed = stubs(&laid.slots);
    assert!(
        stubbed.contains(&2) && !stubbed.contains(&7),
        "{stubbed:?}: {}",
        laid.why
    );
}

#[test]
fn a_stub_that_frees_too_little_waits_for_a_batch() {
    let mut rows = vec![user(0, 50), said(1, 103_000), result(2, 1, 2_000)];
    rows.extend(session(&[])[1..].iter().cloned().map(|mut r| {
        r.cursor += 10;
        r.unit += 10;
        r
    }));
    let laid = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    assert!(stubs(&laid.slots).is_empty(), "{}", laid.why);
    assert!(laid.why.contains("none yet"), "{}", laid.why);
}

#[test]
fn a_cold_cache_stubs_further() {
    let rows = session(&[18_000; 6]);
    let cold = Ask {
        idle_s: Some(900),
        ..ask()
    };
    let warm = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    let laid = lay(&Rules::default(), &cold, &rows, &Held::default(), 1.0);
    assert_eq!(stubs(&warm.slots).len(), 2, "{}", warm.why);
    assert_eq!(stubs(&laid.slots).len(), 3, "{}", laid.why);
    assert!(laid.why.contains("cache cold"));
}

#[test]
fn what_is_kept_or_failed_and_the_last_results_are_never_stubbed() {
    let mut rows = session(&[60_000, 30_000, 30_000]);
    rows[2].keep = true;
    rows[4].error = true;
    let laid = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    let stubbed = stubs(&laid.slots);
    assert_eq!(stubbed, [6], "{}", laid.why);
    let last_three: Vec<u64> = rows
        .iter()
        .rev()
        .filter(|r| r.tool)
        .take(3)
        .map(|r| r.cursor)
        .collect();
    assert!(last_three.iter().all(|c| !stubbed.contains(c)));
}

#[test]
fn over_compact_at_a_summary_of_the_oldest_whole_groups_is_wanted() {
    let mut rows = vec![user(0, 50)];
    for n in 0..13 {
        rows.push(said(1 + n * 2, 5_000));
        rows.push(Row {
            unit: 1 + n * 2,
            ..said(2 + n * 2, 5_000)
        });
    }
    rows.push(user(40, 50));
    rows.push(said(41, 100));
    let laid = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    let span = laid.compact.expect("a summary wanted");
    assert_eq!(
        span.from, 1,
        "the first prompt is protected and stays ahead of the summary"
    );
    assert_eq!(span.to % 2, 0, "ends on the last row of a group: {span:?}");
    let left: u32 = rows
        .iter()
        .filter(|r| !(span.from..=span.to).contains(&r.cursor))
        .map(|r| r.tokens)
        .sum();
    assert!(left <= 72_220, "back to compact_to: {left}");
    assert!(span.to < 40, "never past a protected user turn");
}

#[test]
fn a_summary_rolls_forward_and_sits_where_its_span_was() {
    let mut rows = vec![user(0, 50)];
    for n in 1..=35 {
        rows.push(said(n, 5_000));
    }
    rows.push(user(40, 50));
    let held = Held {
        summary: Some(Covered {
            from: 1,
            to: 10,
            tokens: 2_000,
        }),
        ..Held::default()
    };
    let laid = lay(&Rules::default(), &ask(), &rows, &held, 1.0);
    assert_eq!(
        &laid.slots[..3],
        [Slot::Item(0), Slot::Summary, Slot::Item(11)]
    );
    let span = laid.compact.expect("still over compact_at");
    assert_eq!(span.from, 1, "the new summary takes in the old one");
    assert!(span.to > 10);
}

#[test]
fn the_thrash_guard_stops_asking_for_summaries() {
    let mut rows = vec![user(0, 50)];
    for n in 1..=26 {
        rows.push(said(n, 5_000));
    }
    rows.push(user(40, 50));
    let held = Held {
        compacting: Compacting::Spent,
        ..Held::default()
    };
    let laid = lay(&Rules::default(), &ask(), &rows, &held, 1.0);
    assert!(laid.compact.is_none());
    assert!(laid.why.contains("no more summaries this prompt"));
}

#[test]
fn without_a_summary_the_oldest_whole_groups_are_dropped_to_fit() {
    let mut rows = vec![user(0, 50)];
    for n in 0..20 {
        rows.push(said(1 + n * 2, 4_000));
        rows.push(Row {
            stub_tokens: 3_999,
            ..result(2 + n * 2, 1 + n * 2, 4_000)
        });
    }
    rows.push(user(50, 50));
    let held = Held {
        compacting: Compacting::Pending,
        ..Held::default()
    };
    let laid = lay(&Rules::default(), &ask(), &rows, &held, 1.0);
    assert!(laid.fits, "{}", laid.why);
    assert!(!laid.dropped.is_empty());
    for cursor in &laid.dropped {
        let unit = rows
            .iter()
            .find(|r| r.cursor == *cursor)
            .expect("a row")
            .unit;
        assert!(
            rows.iter()
                .filter(|r| r.unit == unit)
                .all(|r| laid.dropped.contains(&r.cursor)),
            "group {unit} was split"
        );
    }
    assert!(
        laid.why.contains("dropped until a summary exists"),
        "{}",
        laid.why
    );
    assert!(named(&laid.slots).contains(&50), "the latest prompt stays");
}

#[test]
fn slots_go_pinned_summary_conversation_note_memory_then_the_latest_prompt() {
    // Rows 0 and 1 are worth summarising: 900 tokens standing in for 300. A summary larger than
    // its own span is not used at all, which `a_summary_larger_than_its_span_is_not_used` pins.
    let mut rows = vec![user(0, 400), said(1, 500), user(2, 50), said(3, 100)];
    rows.push(said(4, 120_000));
    rows.push(user(5, 50));
    let held = Held {
        summary: Some(Covered {
            from: 0,
            to: 1,
            tokens: 300,
        }),
        pinned: 400,
        memory: 700,
        note: 20,
        ..Held::default()
    };
    let laid = lay(&Rules::default(), &ask(), &rows, &held, 1.0);
    assert_eq!(
        laid.slots,
        [
            Slot::Pinned,
            Slot::Summary,
            Slot::Item(2),
            Slot::Item(3),
            Slot::Item(4),
            Slot::Note,
            Slot::Memory,
            Slot::Item(5),
        ],
        "{}",
        laid.why
    );
    assert!(laid.why.starts_with("summary 0–1"), "{}", laid.why);
}

#[test]
fn shares_scale_from_32k_to_a_million() {
    for window in [32_000, 200_000, 1_048_576] {
        let asked = Ask {
            window,
            reply: window / 8,
            fixed: 3_000,
            ..Ask::default()
        };
        let split = budget(&Rules::default(), &asked, 1.0);
        // What is planned for is the limit less the fixed costs, not what the window has left
        // over: the whole input is held under a share of the window, and the fixed costs are
        // part of that input.
        let room = window / 2 - 3_000;
        assert_eq!(split.room, room, "window {window}");
        assert_eq!(split.memory, (f64::from(room) * 0.05).round() as u32);
        assert_eq!(
            split.conversation + split.memory + split.pinned + split.summary,
            room
        );
    }
}

/// What the whole request may be, as against what is left of the window after the reply.
mod mem_budget_denominator {
    use super::*;

    fn asked(window: u32, reply: u32, fixed: u32) -> Ask {
        Ask {
            window,
            reply,
            fixed,
            ..Ask::default()
        }
    }

    #[test]
    fn everything_planned_for_fits_under_half_the_window() {
        for window in [32_000, 128_000, 200_000, 1_048_576] {
            let split = budget(&Rules::default(), &asked(window, window / 8, 3_000), 1.0);
            assert!(
                split.fixed + split.room <= window / 2,
                "window {window}: {} fixed and {} of room",
                split.fixed,
                split.room
            );
        }
    }

    #[test]
    fn the_reply_still_fits_beside_the_whole_input() {
        for window in [32_000, 128_000, 200_000, 1_048_576] {
            let reply = window / 8;
            let split = budget(&Rules::default(), &asked(window, reply, 3_000), 1.0);
            assert!(
                split.fixed + split.room + reply <= window,
                "window {window}"
            );
        }
    }

    #[test]
    fn a_reply_larger_than_the_share_takes_the_room_rather_than_the_window() {
        // 0.9 of a window reserved for the reply leaves less than the share, and the limit is
        // the smaller of the two.
        let split = budget(&Rules::default(), &asked(100_000, 90_000, 1_000), 1.0);
        assert_eq!(split.room, 9_000);
        assert!(split.fixed + split.room + 90_000 <= 100_000);
    }

    #[test]
    fn fixed_costs_larger_than_the_limit_leave_no_room_rather_than_wrapping() {
        let split = budget(&Rules::default(), &asked(32_000, 4_000, 30_000), 1.0);
        assert_eq!(split.room, 0);
        assert_eq!(split.conversation, 0);
    }

    #[test]
    fn a_window_of_nothing_plans_for_nothing() {
        let split = budget(&Rules::default(), &asked(0, 0, 0), 1.0);
        assert_eq!(split.room, 0);
    }
}

#[test]
fn unused_memory_room_flows_to_the_conversation() {
    let rows = session(&[1_000]);
    let empty = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    let full = Held {
        memory: 7_850,
        ..Held::default()
    };
    let spent = lay(&Rules::default(), &ask(), &rows, &full, 1.0);
    assert_eq!(empty.budget.conversation - spent.budget.conversation, 7_850);
}

#[test]
fn an_overflow_tightens_the_next_layout() {
    let rows = session(&[18_000; 6]);
    let once = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    let tighter = Ask { tight: 1, ..ask() };
    let again = lay(&Rules::default(), &tighter, &rows, &Held::default(), 1.0);
    assert!(stubs(&again.slots).len() > stubs(&once.slots).len());
    assert!(again.budget.used < once.budget.used);
}

#[test]
fn the_factor_corrects_every_estimate() {
    let rows = session(&[1_000]);
    let plain = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    let heavy = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.5);
    assert!(heavy.budget.used > plain.budget.used * 14 / 10);
    assert_eq!(heavy.budget.fixed, 16_500);
}

#[test]
fn a_stub_applied_before_stays_a_stub() {
    let mut rows = session(&[5_000]);
    rows[2].stubbed = true;
    let laid = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    assert_eq!(stubs(&laid.slots), [2]);
}

#[test]
fn old_tool_results_do_not_hold_back_a_summary() {
    // Two results from the first prompt stayed "the last results" through a conversation of prose
    // since, and every summary stopped at the turn before them.
    let mut rows = vec![
        user(0, 50),
        said(1, 100),
        result(2, 1, 500),
        said(3, 100),
        result(4, 3, 500),
    ];
    let mut cursor = 5;
    for _ in 0..14 {
        rows.push(user(cursor, 50));
        rows.push(said(cursor + 1, 10_000));
        cursor += 2;
    }
    let laid = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    let span = laid.compact.expect("a summary wanted");
    assert!(
        span.to > 4,
        "held back by results from a finished prompt: {span:?}"
    );
}

/// What is planned for stays inside what a harness's own boundary will admit.
///
/// The harness computes `L` from the family contract, not from anything shared with this: the
/// formula is restated here on purpose, so a change on either side shows up as a disagreement.
mod mem_dispatch_never_exceeds_policy {
    use super::*;

    /// `L = min(floor(C * share), C - R) - M`, as §4.1 writes it.
    fn limit(window: u32, reply: u32, share: f64, margin: u32) -> u32 {
        let ceiling = (f64::from(window) * share).floor() as u32;
        ceiling
            .min(window.saturating_sub(reply))
            .saturating_sub(margin)
    }

    #[test]
    fn a_whole_planned_request_is_within_the_limit() {
        for window in [32_000_u32, 128_000, 200_000, 1_048_576] {
            let reply = window / 8;
            let asked = Ask {
                window,
                reply,
                fixed: 3_000,
                ..Ask::default()
            };
            let split = budget(&Rules::default(), &asked, 1.0);
            assert!(
                split.fixed + split.room <= limit(window, reply, 0.5, 0),
                "window {window}: {} fixed and {} of room against a limit of {}",
                split.fixed,
                split.room,
                limit(window, reply, 0.5, 0)
            );
        }
    }

    #[test]
    fn a_laid_out_request_is_too() {
        // Not the budget but what `lay` actually chose: slots can be given room the budget did
        // not name, and a limit the budget respects and the layout does not is no limit.
        let rows = session(&[40_000, 30_000, 20_000, 30_000]);
        let asked = ask();
        let laid = lay(&Rules::default(), &asked, &rows, &Held::default(), 1.0);
        let whole = laid.budget.fixed + laid.budget.used;
        assert!(
            whole <= limit(asked.window, asked.reply, 0.5, 0),
            "{whole} against {}",
            limit(asked.window, asked.reply, 0.5, 0)
        );
    }

    #[test]
    fn a_share_and_a_margin_the_harness_might_use_still_agree() {
        for (share, margin) in [(0.5, 0), (0.5, 2_000), (0.4, 0)] {
            let rules = Rules {
                policy: balthasar_buffer::Policy { share, margin },
                ..Rules::default()
            };
            let asked = Ask {
                window: 200_000,
                reply: 32_000,
                fixed: 11_000,
                ..Ask::default()
            };
            let split = budget(&rules, &asked, 1.0);
            assert!(
                split.fixed + split.room <= limit(200_000, 32_000, share, margin),
                "share {share}, margin {margin}"
            );
        }
    }
}

/// What a configuration may say about the budget, and what it may not.
mod mem_budget_checked_arithmetic {
    use super::*;
    use serde_json::json;

    fn read(said: serde_json::Value) -> Rules {
        Rules::read(Some(&said))
    }

    #[test]
    fn a_share_and_a_margin_are_taken_as_given() {
        let rules = read(json!({ "budget": { "share": 0.4, "margin": 500 } }));
        assert!((rules.policy.share - 0.4).abs() < f64::EPSILON);
        assert_eq!(rules.policy.margin, 500);
    }

    #[test]
    fn a_share_outside_the_range_is_refused_rather_than_clamped() {
        // Clamping would plan against a number nobody asked for and say nothing about it.
        for said in [json!(0.0), json!(-0.5), json!(1.5), json!("half")] {
            let rules = read(json!({ "budget": { "share": said } }));
            assert!(
                (rules.policy.share - Rules::default().policy.share).abs() < f64::EPSILON,
                "{said} was taken"
            );
        }
    }

    #[test]
    fn a_share_that_is_not_a_number_at_all_is_refused() {
        for said in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let rules = read(json!({ "budget": { "share": said } }));
            assert!((rules.policy.share - 0.5).abs() < f64::EPSILON, "{said}");
        }
    }

    #[test]
    fn a_margin_too_large_for_the_type_is_refused_rather_than_wrapping() {
        let rules = read(json!({ "budget": { "margin": u64::from(u32::MAX) + 1 } }));
        assert_eq!(rules.policy.margin, 0);
    }

    #[test]
    fn saying_nothing_leaves_the_shipped_policy() {
        assert_eq!(read(json!({})).policy, Rules::default().policy);
        assert_eq!(Rules::read(None).policy, Rules::default().policy);
    }

    #[test]
    fn the_whole_thing_is_still_bounded_by_whatever_was_accepted() {
        let rules = read(json!({ "budget": { "share": 0.25 } }));
        let asked = Ask {
            window: 200_000,
            reply: 32_000,
            fixed: 11_000,
            ..Ask::default()
        };
        let split = budget(&rules, &asked, 1.0);
        assert!(split.fixed + split.room <= 50_000, "a quarter of 200k");
    }
}

/// A compaction that grows the request is a compaction that should not have happened.
mod a_summary_larger_than_its_span_is_not_used {
    use super::*;

    fn held(tokens: u32) -> Held {
        Held {
            summary: Some(Covered {
                from: 1,
                to: 2,
                tokens,
            }),
            ..Held::default()
        }
    }

    fn rows() -> Vec<Row> {
        vec![user(0, 50), said(1, 100), said(2, 100), user(3, 50)]
    }

    #[test]
    fn a_summary_bigger_than_the_turns_it_stands_for_is_left_out() {
        // Two short turns at 200 tokens, summarised into 300: keeping them is cheaper, and the
        // request that used the summary was larger than the one that did not.
        let laid = lay(&Rules::default(), &ask(), &rows(), &held(300), 1.0);
        assert!(!laid.slots.contains(&Slot::Summary), "{:?}", laid.slots);
        assert_eq!(named(&laid.slots), [0, 1, 2, 3], "the turns went instead");
    }

    #[test]
    fn one_the_same_size_is_not_worth_it_either() {
        let laid = lay(&Rules::default(), &ask(), &rows(), &held(200), 1.0);
        assert!(!laid.slots.contains(&Slot::Summary));
    }

    #[test]
    fn one_that_saves_anything_at_all_is_used() {
        let laid = lay(&Rules::default(), &ask(), &rows(), &held(199), 1.0);
        assert!(laid.slots.contains(&Slot::Summary), "{:?}", laid.slots);
        assert_eq!(named(&laid.slots), [0, 3], "and its span went with it");
    }

    #[test]
    fn a_span_with_nothing_left_in_it_keeps_its_summary() {
        // The ordinary case once rows have been dropped: the summary is all that is left of them,
        // and dropping it too would lose the span entirely.
        let held = Held {
            summary: Some(Covered {
                from: 1,
                to: 2,
                tokens: 300,
            }),
            ..Held::default()
        };
        let laid = lay(
            &Rules::default(),
            &ask(),
            &[user(0, 50), user(3, 50)],
            &held,
            1.0,
        );
        assert!(laid.slots.contains(&Slot::Summary), "{:?}", laid.slots);
    }
}
