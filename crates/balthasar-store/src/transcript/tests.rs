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
