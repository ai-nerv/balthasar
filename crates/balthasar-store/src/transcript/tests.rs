use super::*;

#[test]
fn a_turn_occupies_one_place_in_one_run() {
    // Keyed on (session, cursor): a re-sent turn corrects what was said rather than adding one.
    let mut held = Transcript::ephemeral().expect("a transcript");
    let session = SessionId::new("s");
    held.write(&session, &turn(7, "first")).expect("first");
    held.write(&session, &turn(7, "corrected")).expect("again");
    let back = held.replay(&session).expect("replay");
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].text, "corrected");
}

fn turn(cursor: u64, text: &str) -> Turn {
    Turn {
        cursor,
        at: 1_756_000_000,
        role: "user".into(),
        kind: "prose".into(),
        text: text.to_owned(),
        raw: Some(format!(r#"{{"type":"user","text":{text:?}}}"#)),
        ..Turn::default()
    }
}

fn held() -> Transcript {
    Transcript::ephemeral().expect("a scrollback")
}

#[test]
fn a_run_comes_back_in_the_order_it_was_said() {
    let mut t = held();
    let s = SessionId::new("s1");
    for cursor in [2, 0, 1] {
        t.write(&s, &turn(cursor, &format!("turn {cursor}")))
            .expect("write");
    }
    let cursors: Vec<u64> = t
        .replay(&s)
        .expect("replay")
        .iter()
        .map(|t| t.cursor)
        .collect();
    assert_eq!(cursors, [0, 1, 2]);
}

#[test]
fn the_same_thing_said_twice_is_two_turns() {
    // The memory store deduplicates within a session; two identical turns here are two turns.
    let mut t = held();
    let s = SessionId::new("s1");
    t.write(&s, &turn(0, "carry on")).expect("write");
    t.write(&s, &turn(1, "carry on")).expect("write");
    assert_eq!(t.replay(&s).expect("replay").len(), 2);
}

#[test]
fn a_turn_can_be_revised_where_it_stands() {
    let mut t = held();
    let s = SessionId::new("s1");
    t.write(&s, &turn(0, "running")).expect("write");
    t.write(&s, &turn(0, "done")).expect("revise");

    let back = t.replay(&s).expect("replay");
    assert_eq!(back.len(), 1, "a revision is not a second turn");
    assert_eq!(back[0].text, "done");
    assert_eq!(back[0].revisions, 1, "and it is visible that it happened");
}

#[test]
fn what_a_harness_wrote_comes_back_untouched() {
    let mut t = held();
    let s = SessionId::new("s1");
    let raw = r#"{"type":"tool","id":"t1","name":"shell","result":{"output":"ok"}}"#;
    t.write(
        &s,
        &Turn {
            raw: Some(raw.to_owned()),
            ..turn(0, "")
        },
    )
    .expect("write");
    assert_eq!(t.replay(&s).expect("replay")[0].raw.as_deref(), Some(raw));
}

#[test]
fn a_resuming_harness_is_told_where_it_was() {
    let mut t = held();
    let s = SessionId::new("s1");
    assert_eq!(t.next_cursor(&s).expect("next"), 0, "nothing yet");
    t.write(&s, &turn(0, "a")).expect("write");
    t.write(&s, &turn(7, "b")).expect("write");
    assert_eq!(t.next_cursor(&s).expect("next"), 8);
}

#[test]
fn one_turn_can_be_fetched_for_quoting() {
    let mut t = held();
    let s = SessionId::new("s1");
    t.write(&s, &turn(3, "the deploy target is fly.io"))
        .expect("write");
    let one = t.at(&s, 3).expect("at").expect("there");
    assert_eq!(one.text, "the deploy target is fly.io");
    assert!(t.at(&s, 99).expect("at").is_none());
}

#[test]
fn two_runs_keep_separate_scrollbacks() {
    let mut t = held();
    t.write(&SessionId::new("a"), &turn(0, "one"))
        .expect("write");
    t.write(&SessionId::new("b"), &turn(0, "two"))
        .expect("write");
    assert_eq!(t.replay(&SessionId::new("a")).expect("replay").len(), 1);
    assert_eq!(
        t.replay(&SessionId::new("b")).expect("replay")[0].text,
        "two"
    );
}

#[test]
fn a_run_is_opened_once_however_often_it_reconnects() {
    let mut t = held();
    let s = SessionId::new("s1");
    for _ in 0..3 {
        t.open_run(&s, "/w/thing", "/w/thing", "harness", 100)
            .expect("open");
    }
    assert_eq!(t.runs(10).expect("runs").len(), 1);
}

#[test]
fn a_run_says_how_much_it_holds() {
    let mut t = held();
    let s = SessionId::new("s1");
    t.open_run(&s, "/w/thing", "/w/thing", "harness", 100)
        .expect("open");
    for cursor in 0..4 {
        t.write(&s, &turn(cursor, "x")).expect("write");
    }
    assert_eq!(t.runs(10).expect("runs")[0].turns, 4);
    assert_eq!(t.census().expect("census"), (1, 4));
}

#[test]
fn a_model_told_before_its_run_opens_is_kept() {
    let mut t = held();
    let s = SessionId::new("s1");
    t.note_model(&s, "deepseek/v3.2", 163_840).expect("told");
    t.open_run(&s, "/w/thing", "/w/thing", "harness", 100)
        .expect("open");
    assert_eq!(
        t.model_of(&s).expect("read"),
        Some(("deepseek/v3.2".to_owned(), 163_840))
    );
}

#[test]
fn the_scrollback_lives_beside_the_memory_but_not_in_it() {
    let scope = balthasar_model::ScopeId::new("/w/thing");
    let memory = crate::scope_path(&scope, &crate::Tool::default());
    let scrollback = transcript_path(&scope, &crate::Tool::default());
    assert_ne!(memory, scrollback);
    assert_eq!(memory.parent(), scrollback.parent());
    assert!(
        scrollback.to_string_lossy().ends_with("-transcript.db"),
        "{}",
        scrollback.display()
    );
}

#[test]
fn an_insert_never_overwrites() {
    let t = held();
    let s = SessionId::new("s1");
    assert_eq!(
        t.insert(&s, &turn(0, "a")).expect("first"),
        crate::Inserted::Fresh
    );
    assert_eq!(
        t.insert(&s, &turn(0, "a")).expect("retry"),
        crate::Inserted::Same
    );
    assert_eq!(
        t.insert(&s, &turn(0, "b")).expect("other"),
        crate::Inserted::Different
    );
    assert_eq!(t.at(&s, 0).expect("at").expect("there").text, "a");
}

#[test]
fn a_retry_that_settled_later_is_still_the_same_turn() {
    let t = held();
    let s = SessionId::new("s1");
    t.insert(&s, &turn(0, "a")).expect("first");
    let later = Turn {
        at: 1_756_000_999,
        ..turn(0, "a")
    };
    assert_eq!(t.insert(&s, &later).expect("retry"), crate::Inserted::Same);
}

#[test]
fn a_revision_keeps_what_the_turn_said_before() {
    let mut t = held();
    let s = SessionId::new("s1");
    t.write(&s, &turn(0, "running")).expect("write");
    t.write(&s, &turn(0, "done")).expect("revise");
    t.write(&s, &turn(0, "done")).expect("the same again");
    let before = t.revisions_of(&s, 0).expect("revisions");
    assert_eq!(before.len(), 1, "the same write again keeps nothing new");
    assert_eq!(before[0].text, "running");
    assert_eq!(t.at(&s, 0).expect("at").expect("there").revisions, 1);
}

#[test]
fn what_was_noticed_sits_beside_the_turn_in_arrival_order() {
    let t = held();
    let s = SessionId::new("s1");
    t.insert(&s, &turn(3, "the call")).expect("turn");
    assert_eq!(t.notice(&s, 3, 1, "permission", "allowed").expect("one"), 0);
    assert_eq!(t.notice(&s, 3, 2, "retry", "again").expect("two"), 1);
    let seen = t.noticed(&s, 0, 10).expect("noticed");
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].kind, "retry");
    assert_eq!(t.at(&s, 3).expect("at").expect("there").text, "the call");
}

#[test]
fn a_raw_record_comes_back_in_parts_cut_on_characters() {
    let t = held();
    let s = SessionId::new("s1");
    let raw = "añb€c😀d".repeat(3);
    let said = Turn {
        raw: Some(raw.clone()),
        ..turn(0, "x")
    };
    t.insert(&s, &said).expect("turn");
    let mut whole = String::new();
    let mut from = 0_u64;
    loop {
        let part = t.raw_part(&s, 0, from, 5).expect("part").expect("a raw");
        assert_eq!(part.total, raw.len() as u64);
        assert!(part.text.len() <= 5, "{:?}", part.text);
        if part.text.is_empty() {
            break;
        }
        from += part.text.len() as u64;
        whole.push_str(&part.text);
    }
    assert_eq!(whole, raw);
    assert!(t.raw_part(&s, 0, 2, 5).is_err(), "byte 2 is inside ñ");

    let bare = Turn {
        raw: None,
        ..turn(1, "y")
    };
    t.insert(&s, &bare).expect("turn");
    assert_eq!(t.raw_part(&s, 1, 0, 5).expect("read"), None);
    assert_eq!(t.raw_part(&s, 9, 0, 5).expect("read"), None);
}

#[test]
fn opening_a_prepared_scrollback_takes_no_write_lock() {
    let dir = balthasar_model::scratch::Scratch::new("balthasar-transcript", "lockless");
    let path = dir.join("transcript.db");
    drop(Transcript::open(&path).expect("first open prepares"));
    let writer = rusqlite::Connection::open(&path).expect("writer");
    writer
        .execute_batch("BEGIN IMMEDIATE")
        .expect("hold the write lock");
    let started = std::time::Instant::now();
    let again = Transcript::open(&path);
    assert!(again.is_ok(), "{:?}", again.err());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "it waited on a lock it had no need of"
    );
    writer.execute_batch("ROLLBACK").expect("release");
}
