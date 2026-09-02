//! Reading part of a scrollback.
//!
//! memo is the only copy of what was said, so a run's transcript grows without limit while a
//! model's context does not. These are the reads that sit between "one turn" and "all of it",
//! and every one of them is about a bound holding.

use memo_model::SessionId;
use memo_store::{Budget, Transcript, Turn, Want};

const NOW: memo_model::Timestamp = 1_756_000_000;

fn run() -> SessionId {
    SessionId::new("01RUN")
}

fn turn(cursor: u64, text: &str) -> Turn {
    Turn {
        cursor,
        at: NOW + cursor as i64,
        role: if cursor.is_multiple_of(2) {
            "user"
        } else {
            "tool"
        }
        .to_owned(),
        kind: "prose".to_owned(),
        text: text.to_owned(),
        tool: None,
        entry: None,
        raw: None,
        revisions: 0,
        ..Turn::default()
    }
}

/// A run of `n` turns, each about 25 tokens.
fn a_long_run(n: u64) -> Transcript {
    let mut held = Transcript::ephemeral().expect("transcript");
    held.open_run(&run(), "/w/thing", "/w/thing", "suite", NOW)
        .expect("open");
    for cursor in 0..n {
        let text = format!("turn number {cursor} {}", "word ".repeat(20));
        held.write(&run(), &turn(cursor, &text)).expect("write");
    }
    held
}

#[test]
fn the_tail_is_the_end_and_stays_inside_its_budget() {
    // What a resuming run wants: what was just happening, not what happened in March.
    let held = a_long_run(400);
    let read = held
        .read(
            &run(),
            &Want::Tail,
            &Budget {
                tokens: 200,
                turns: 100,
            },
        )
        .expect("read");

    assert!(read.tokens <= 200, "{} tokens", read.tokens);
    assert!(!read.is_complete(), "there was more");
    let last = read.turns.last().expect("something");
    assert_eq!(last.cursor, 399, "it read from the end");
}

#[test]
fn a_bounded_read_says_what_it_left_out() {
    // The honesty bit. A caller cannot otherwise tell "that is all there was" from "that is all
    // you asked for".
    let held = a_long_run(400);
    let read = held
        .read(
            &run(),
            &Want::Tail,
            &Budget {
                tokens: 200,
                turns: 100,
            },
        )
        .expect("read");

    assert!(read.omitted > 0);
    assert!(read.next.is_some(), "and where to continue from");
    let said = read.note().expect("a note");
    assert!(said.contains("more turn"), "{said}");
}

#[test]
fn a_run_that_fits_is_complete_and_says_nothing() {
    let held = a_long_run(3);
    let read = held
        .read(&run(), &Want::Tail, &Budget::default())
        .expect("read");

    assert_eq!(read.turns.len(), 3);
    assert!(read.is_complete());
    assert_eq!(read.note(), None);
    assert_eq!(read.next, None);
}

#[test]
fn a_span_is_what_an_episode_is() {
    // F2 segments a run into episodes with cursor ranges, which makes the episode the natural
    // chunk to hand a model — this is how one is fetched.
    let held = a_long_run(100);
    let read = held
        .read(&run(), &Want::Span { from: 10, to: 19 }, &Budget::default())
        .expect("read");

    assert_eq!(read.turns.len(), 10);
    assert_eq!(read.turns[0].cursor, 10);
    assert_eq!(read.turns[9].cursor, 19);
    assert!(read.is_complete());
}

#[test]
fn a_long_span_comes_back_in_chunks() {
    // The answer to "this could be endless". A span too big for one budget is read in pieces,
    // each one continuing from where the last stopped.
    let held = a_long_run(200);
    let budget = Budget {
        tokens: 150,
        turns: 50,
    };

    let mut at = 0;
    let mut seen = 0;
    let mut rounds = 0;
    loop {
        let read = held
            .read(&run(), &Want::Span { from: at, to: 199 }, &budget)
            .expect("read");
        seen += read.turns.len();
        rounds += 1;
        match read.next {
            Some(next) => at = next,
            None => break,
        }
        assert!(rounds < 100, "it did not converge");
    }
    assert_eq!(seen, 200, "every turn came back across {rounds} chunk(s)");
    assert!(rounds > 1, "and it took more than one");
}

#[test]
fn a_turn_larger_than_the_budget_still_comes_back() {
    // A budget smaller than one turn must give that turn and say the budget was exceeded, not
    // give nothing and read as an empty run.
    let mut held = Transcript::ephemeral().expect("transcript");
    held.open_run(&run(), "/w/thing", "/w/thing", "suite", NOW)
        .expect("open");
    held.write(&run(), &turn(0, &"x".repeat(40_000)))
        .expect("write");

    let read = held
        .read(
            &run(),
            &Want::Tail,
            &Budget {
                tokens: 10,
                turns: 10,
            },
        )
        .expect("read");
    assert_eq!(read.turns.len(), 1);
    assert!(read.tokens > 10, "and it says what it actually cost");
}

#[test]
fn the_turn_cap_holds_even_when_the_tokens_would_allow_more() {
    // Ten thousand one-word turns fit a generous budget and are useless to read.
    let mut held = Transcript::ephemeral().expect("transcript");
    held.open_run(&run(), "/w/thing", "/w/thing", "suite", NOW)
        .expect("open");
    for cursor in 0..500 {
        held.write(&run(), &turn(cursor, "ok")).expect("write");
    }

    let read = held
        .read(
            &run(),
            &Want::Tail,
            &Budget {
                tokens: 100_000,
                turns: 20,
            },
        )
        .expect("read");
    assert_eq!(read.turns.len(), 20);
    assert_eq!(read.omitted, 480);
}

#[test]
fn a_citation_comes_back_with_context_on_both_sides() {
    // What `memo why` needs: the quoted turn is useless without what surrounded it.
    let held = a_long_run(100);
    let read = held
        .read(
            &run(),
            &Want::Around { cursor: 50 },
            &Budget {
                tokens: 200,
                turns: 20,
            },
        )
        .expect("read");

    let cursors: Vec<u64> = read.turns.iter().map(|t| t.cursor).collect();
    assert!(cursors.contains(&50), "the turn itself: {cursors:?}");
    assert!(cursors.iter().any(|c| *c < 50), "something before it");
    assert!(cursors.iter().any(|c| *c > 50), "and something after");
    assert!(read.tokens <= 200);
}

#[test]
fn a_citation_at_the_very_start_still_works() {
    // No turns before it. Growing outward has to cope with one side being empty rather than
    // returning nothing.
    let held = a_long_run(20);
    let read = held
        .read(
            &run(),
            &Want::Around { cursor: 0 },
            &Budget {
                tokens: 200,
                turns: 10,
            },
        )
        .expect("read");

    assert_eq!(read.turns.first().expect("something").cursor, 0);
    assert!(read.turns.len() > 1, "and it grew forwards");
}

#[test]
fn a_citation_that_is_not_there_reads_as_empty() {
    let held = a_long_run(10);
    let read = held
        .read(&run(), &Want::Around { cursor: 9999 }, &Budget::default())
        .expect("read");
    assert!(read.turns.is_empty());
    assert!(
        read.is_complete(),
        "nothing was omitted — there was nothing"
    );
}

#[test]
fn matching_finds_the_turns_that_mention_everything_asked() {
    let mut held = Transcript::ephemeral().expect("transcript");
    held.open_run(&run(), "/w/thing", "/w/thing", "suite", NOW)
        .expect("open");
    held.write(&run(), &turn(0, "we deploy with flyctl"))
        .expect("write");
    held.write(&run(), &turn(1, "the tests run with make"))
        .expect("write");
    held.write(&run(), &turn(2, "flyctl deploy needs the token"))
        .expect("write");

    let read = held
        .read(
            &run(),
            &Want::Matching {
                terms: vec!["flyctl".to_owned(), "deploy".to_owned()],
            },
            &Budget::default(),
        )
        .expect("read");

    let cursors: Vec<u64> = read.turns.iter().map(|t| t.cursor).collect();
    assert_eq!(cursors, vec![0, 2], "both terms, not either");
}

#[test]
fn matching_nothing_asks_for_nothing() {
    // An empty term list must not match every turn in the run, which is the shape a caller
    // passing an unparsed query would arrive in.
    let held = a_long_run(50);
    let read = held
        .read(
            &run(),
            &Want::Matching { terms: Vec::new() },
            &Budget::default(),
        )
        .expect("read");
    assert!(read.turns.is_empty());
}

#[test]
fn reading_never_touches_the_scrollback() {
    // The transcript is the only copy. A read that could change it would be the worst bug in
    // the system, so this is checked rather than assumed.
    let held = a_long_run(50);
    let before = held.replay(&run()).expect("replay");

    for want in [
        Want::Tail,
        Want::Span { from: 0, to: 10 },
        Want::Around { cursor: 25 },
        Want::Matching {
            terms: vec!["turn".to_owned()],
        },
    ] {
        held.read(
            &run(),
            &want,
            &Budget {
                tokens: 50,
                turns: 5,
            },
        )
        .expect("read");
    }

    assert_eq!(held.replay(&run()).expect("replay"), before);
}

#[test]
fn a_bounded_read_of_a_huge_run_is_quick() {
    // The point of all of this. `replay` on this would build ten thousand turns in memory; a
    // bounded read must cost what it returns, not what exists.
    let held = a_long_run(10_000);
    let started = std::time::Instant::now();
    let read = held
        .read(
            &run(),
            &Want::Tail,
            &Budget {
                tokens: 500,
                turns: 40,
            },
        )
        .expect("read");
    let took = started.elapsed();

    assert!(read.turns.len() <= 40);
    assert!(
        took.as_millis() < 250,
        "took {took:?} on a ten-thousand-turn run"
    );
}

/// One assistant message written as four blocks: prose, then three tool calls.
fn a_message_of_blocks(held: &mut Transcript, from: u64, entry: &str) {
    for (n, (role, kind, text)) in [
        ("assistant", "prose", "I will check three things"),
        ("assistant", "tool_call", "make test"),
        ("assistant", "tool_call", "make lint"),
        ("assistant", "tool_call", "make build"),
    ]
    .iter()
    .enumerate()
    {
        held.write(
            &run(),
            &Turn {
                cursor: from + n as u64,
                at: NOW,
                role: (*role).to_owned(),
                kind: (*kind).to_owned(),
                text: format!("{text} {}", "padding ".repeat(12)),
                tool: None,
                entry: Some(entry.to_owned()),
                raw: (n == 0).then(|| format!("{{\"entry\":\"{entry}\"}}")),
                revisions: 0,
                ..Turn::default()
            },
        )
        .expect("write");
    }
}

#[test]
fn one_message_is_several_blocks_and_each_is_addressable() {
    // The point of the split: a span can name one tool call inside a message rather than only
    // the whole message.
    let mut held = Transcript::ephemeral().expect("transcript");
    held.open_run(&run(), "/w/thing", "/w/thing", "suite", NOW)
        .expect("open");
    a_message_of_blocks(&mut held, 0, "e1");

    let read = held
        .read(&run(), &Want::Span { from: 2, to: 2 }, &Budget::default())
        .expect("read");
    assert_eq!(read.turns.len(), 1);
    assert_eq!(read.turns[0].kind, "tool_call");
    assert!(read.turns[0].text.starts_with("make lint"));
}

#[test]
fn blocks_of_one_message_share_an_entry() {
    let mut held = Transcript::ephemeral().expect("transcript");
    held.open_run(&run(), "/w/thing", "/w/thing", "suite", NOW)
        .expect("open");
    a_message_of_blocks(&mut held, 0, "e1");

    let all = held.replay(&run()).expect("replay");
    assert_eq!(all.len(), 4);
    assert!(all.iter().all(|t| t.entry.as_deref() == Some("e1")));
}

#[test]
fn a_tail_never_starts_in_the_middle_of_a_message() {
    // An assistant turn without the tool call it made, or a tool result with nothing that asked
    // for it, is worse than not showing the message at all.
    let mut held = Transcript::ephemeral().expect("transcript");
    held.open_run(&run(), "/w/thing", "/w/thing", "suite", NOW)
        .expect("open");
    a_message_of_blocks(&mut held, 0, "e1");
    a_message_of_blocks(&mut held, 4, "e2");

    // A budget that lands part-way into the first message.
    let read = held
        .read(
            &run(),
            &Want::Tail,
            &Budget {
                tokens: 30,
                turns: 6,
            },
        )
        .expect("read");

    let entries: std::collections::BTreeSet<Option<String>> =
        read.turns.iter().map(|t| t.entry.clone()).collect();
    for entry in entries.iter().flatten() {
        let shown = read
            .turns
            .iter()
            .filter(|t| t.entry.as_ref() == Some(entry))
            .count();
        let total = held
            .replay(&run())
            .expect("replay")
            .iter()
            .filter(|t| t.entry.as_ref() == Some(entry))
            .count();
        assert_eq!(shown, total, "message {entry} came back in pieces");
    }
}

#[test]
fn a_turn_that_is_its_own_message_needs_no_entry() {
    // A plain user turn is one block, and should not have to invent a grouping to be read.
    let held = a_long_run(10);
    let read = held
        .read(&run(), &Want::Tail, &Budget::default())
        .expect("read");
    assert!(read.turns.iter().all(|t| t.entry.is_none()));
    assert_eq!(read.turns.len(), 10);
}

#[test]
fn only_the_first_block_carries_the_record_to_restore_from() {
    // Restoring reassembles messages, not blocks: the harness's own record belongs to the
    // message, so it is written once and `replay --raw` emits it once. The other blocks exist
    // for reading, not for restoring.
    let mut held = Transcript::ephemeral().expect("transcript");
    held.open_run(&run(), "/w/thing", "/w/thing", "suite", NOW)
        .expect("open");
    a_message_of_blocks(&mut held, 0, "e1");

    let raws: Vec<&String> = held
        .replay(&run())
        .expect("replay")
        .iter()
        .filter_map(|t| t.raw.as_ref())
        .cloned()
        .collect::<Vec<_>>()
        .leak()
        .iter()
        .collect();
    assert_eq!(raws.len(), 1, "one record per message, not per block");
}
