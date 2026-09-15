//! Every rule of the layout, on synthetic rows.

use balthasar_buffer::{Ask, Compacting, Covered, Held, Row, Rules, Slot, budget, lay};

/// A 200k window with 11k fixed and a 32k reply: 157k of room, 144,440 for the conversation.
fn ask() -> Ask {
    Ask {
        window: 200_000,
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
    let mut rows = vec![user(0, 50), said(1, 100), user(2, 50), said(3, 100)];
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
        let room = window - window / 8 - 3_000;
        assert_eq!(split.room, room);
        assert_eq!(split.memory, (f64::from(room) * 0.05).round() as u32);
        assert_eq!(
            split.conversation + split.memory + split.pinned + split.summary,
            room
        );
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
