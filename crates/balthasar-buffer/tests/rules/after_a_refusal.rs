//! The request after one the provider refused as too long is smaller than it, and never by way of a
//! summary that outweighs what it takes the place of.

use super::*;
use balthasar_buffer::{Refused, against, under};

/// A prompt, a call that failed, and three big results; the last of them the newest row.
fn rows() -> Vec<Row> {
    let mut rows = vec![
        user(0, 50),
        said(1, 10),
        Row {
            error: true,
            ..result(2, 1, 20)
        },
    ];
    for cursor in [3, 5, 7] {
        rows.push(said(cursor, 10));
        rows.push(result(cursor + 1, cursor, 4_000));
    }
    rows
}

/// What the provider refused: the big results but the newest were left out already.
fn floor() -> Refused {
    Refused {
        slots: vec![
            Slot::Item(0),
            Slot::Item(1),
            Slot::Item(2),
            Slot::Item(7),
            Slot::Item(8),
        ],
        summary: None,
    }
}

fn summary(tokens: u32) -> Held {
    Held {
        summary: Some(Covered {
            from: 1,
            to: 6,
            tokens,
        }),
        ..Held::default()
    }
}

#[test]
fn the_newest_result_is_stubbed_when_nothing_else_is_left() {
    let rows = rows();
    let held = Held::default();
    let refused = floor();
    let mut laid = lay(&Rules::default(), &ask(), &rows, &held, 1.0);
    laid.slots.clone_from(&refused.slots);
    let stubbed = under(&mut laid, &rows, &held, &refused, 1.0);
    assert_eq!(stubbed, 1);
    assert_eq!(stubs(&laid.slots), [8], "the newest, which no rule stubs");
    assert!(laid.budget.used < refused.cost(&rows, &held, 1.0));
    assert!(
        laid.slots.contains(&Slot::Item(2)),
        "what failed stays whole"
    );
}

#[test]
fn the_largest_goes_first_and_only_until_it_is_under() {
    let rows = rows();
    let held = Held::default();
    let refused = Refused {
        slots: named(&lay(&Rules::default(), &ask(), &rows, &held, 1.0).slots)
            .into_iter()
            .map(Slot::Item)
            .collect(),
        summary: None,
    };
    let mut laid = lay(&Rules::default(), &ask(), &rows, &held, 1.0);
    let stubbed = under(&mut laid, &rows, &held, &refused, 1.0);
    assert_eq!(stubbed, 1, "one is enough to come in under");
    assert_eq!(
        stubs(&laid.slots),
        [4],
        "of three the same size, the oldest"
    );
    let mut rows = rows;
    rows[8].tokens = 5_000;
    let mut laid = lay(&Rules::default(), &ask(), &rows, &held, 1.0);
    under(&mut laid, &rows, &held, &refused, 1.0);
    assert_eq!(
        stubs(&laid.slots),
        [8],
        "and the largest before any, newest or not"
    );
}

#[test]
fn with_nothing_left_to_stub_it_is_said_and_nothing_changes() {
    let rows = vec![
        user(0, 50),
        said(1, 10),
        Row {
            error: true,
            ..result(2, 1, 4_000)
        },
        said(3, 10),
        Row {
            keep: true,
            ..result(4, 3, 4_000)
        },
    ];
    let held = Held::default();
    let mut laid = lay(&Rules::default(), &ask(), &rows, &held, 1.0);
    let refused = Refused {
        slots: laid.slots.clone(),
        summary: None,
    };
    assert_eq!(under(&mut laid, &rows, &held, &refused, 1.0), 0);
    assert!(
        stubs(&laid.slots).is_empty(),
        "a failure and a kept result stay"
    );
    assert_eq!(laid.budget.used, refused.cost(&rows, &held, 1.0));
    assert!(laid.why.contains("nothing left to stub"), "{}", laid.why);
}

#[test]
fn a_summary_bigger_than_what_it_replaces_there_is_held_back() {
    // Over the call that failed and two results the refused request had already left out: in
    // that request it would stand only where the failed call stood, and it is the larger.
    let rows = rows();
    let held = against(&summary(65), &rows, &floor(), 1.0);
    assert_eq!(held.summary, None);
    assert_eq!(held.compacting, Compacting::Withheld);
    let laid = lay(&Rules::default(), &ask(), &rows, &held, 1.0);
    assert!(!laid.slots.contains(&Slot::Summary), "{:?}", laid.slots);
    assert!(
        laid.slots.contains(&Slot::Item(2)),
        "the failed call goes as it was"
    );
}

#[test]
fn a_summary_held_back_is_not_asked_for_again() {
    let mut rows = vec![user(0, 50)];
    rows.extend((1..=26).map(|n| said(n, 5_000)));
    rows.push(user(40, 50));
    let free = lay(&Rules::default(), &ask(), &rows, &Held::default(), 1.0);
    assert!(free.compact.is_some(), "{}", free.why);
    let held = Held {
        compacting: Compacting::Withheld,
        ..Held::default()
    };
    let laid = lay(&Rules::default(), &ask(), &rows, &held, 1.0);
    assert!(laid.compact.is_none());
    assert!(laid.why.contains("held back"), "{}", laid.why);
}

#[test]
fn a_summary_smaller_than_what_it_replaces_there_is_used() {
    let rows = rows();
    let mut refused = floor();
    refused.slots.splice(3..3, [Slot::Item(3), Slot::Item(4)]);
    let held = against(&summary(65), &rows, &refused, 1.0);
    assert_eq!(
        held,
        summary(65),
        "it stands for a whole result the refused request held"
    );
}

#[test]
fn the_summary_the_refused_request_carried_is_what_it_replaces() {
    let rows = rows();
    let refused = Refused {
        slots: vec![Slot::Item(0), Slot::Summary, Slot::Item(7), Slot::Item(8)],
        summary: summary(65).summary,
    };
    assert_eq!(against(&summary(65), &rows, &refused, 1.0), summary(65));
    assert_eq!(
        against(&summary(80), &rows, &refused, 1.0).summary,
        None,
        "a longer one in its place is held back"
    );
}
