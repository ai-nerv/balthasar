//! Searching what was actually said.
//!
//! The half of recall no memory can answer. A claim stated once, never repeated, never marked
//! and never extracted lives in the transcript and nowhere else — and until this index existed
//! it was reachable only by replaying the whole run.

use memo_model::SessionId;
use memo_store::{Transcript, Turn};

const NOW: memo_model::Timestamp = 1_756_000_000;

fn held() -> Transcript {
    Transcript::ephemeral().expect("a transcript")
}

fn said(held: &mut Transcript, session: &str, cursor: u64, at: i64, text: &str) {
    let id = SessionId::new(session);
    let _ = held.open_run(&id, "/w/thing", "/w/thing", "test", NOW);
    held.write(
        &id,
        &Turn {
            cursor,
            at,
            role: "user".into(),
            kind: "prose".into(),
            text: text.to_owned(),
            ..Turn::default()
        },
    )
    .expect("write");
}

#[test]
fn a_claim_nobody_extracted_is_still_findable() {
    // The whole point. No marker, no repetition, no rule fires on it — and the words are there.
    let mut h = held();
    said(
        &mut h,
        "01A",
        1,
        NOW,
        "we moved the deploy to fly.io last Tuesday",
    );
    said(&mut h, "01A", 2, NOW, "anyway, back to the parser");

    let found = h.spans_matching("\"deploy\"", 10).expect("search");
    assert_eq!(found.len(), 1);
    assert!(found[0].text.contains("fly.io"));
    assert_eq!(found[0].cursor, 1, "and it can be quoted");
    assert_eq!(found[0].session, SessionId::new("01A"));
}

#[test]
fn a_relative_reference_survives_because_the_words_do() {
    // 14.9% of what extraction loses is this: "last Tuesday" means nothing once lifted out of
    // the sentence it was said in, so it only survives in situ.
    let mut h = held();
    said(&mut h, "01A", 1, NOW, "we switched the runner last Tuesday");
    let found = h.spans_matching("\"Tuesday\"", 10).expect("search");
    assert_eq!(found.len(), 1);
}

#[test]
fn correcting_a_turn_does_not_leave_its_first_wording_searchable() {
    // `write` is idempotent on (session, cursor) — a harness re-sending a turn is correcting it.
    // If the index kept both, a search would surface words nobody ever said.
    let mut h = held();
    said(&mut h, "01A", 1, NOW, "we deploy with heroku");
    said(&mut h, "01A", 1, NOW, "we deploy with fly.io");

    assert!(
        h.spans_matching("\"heroku\"", 10)
            .expect("search")
            .is_empty()
    );
    assert_eq!(h.spans_matching("\"fly.io\"", 10).expect("search").len(), 1);
}

#[test]
fn the_later_of_two_equal_matches_comes_first() {
    // Two spans that match equally well are not equally useful: the later one is what somebody
    // believes now.
    let mut h = held();
    said(&mut h, "01A", 1, NOW, "the database is postgres");
    said(&mut h, "01B", 1, NOW + 86_400, "the database is postgres");

    let found = h.spans_matching("\"database\"", 10).expect("search");
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].session, SessionId::new("01B"), "newest first");
}

#[test]
fn a_turn_with_no_words_is_not_indexed() {
    let mut h = held();
    said(&mut h, "01A", 1, NOW, "   ");
    said(&mut h, "01A", 2, NOW, "real content here about the parser");
    assert_eq!(h.spans_matching("\"parser\"", 10).expect("search").len(), 1);
}

#[test]
fn a_search_is_bounded_however_much_was_said() {
    let mut h = held();
    for cursor in 0..400 {
        said(&mut h, "01A", cursor, NOW, "the deploy target matters here");
    }
    assert_eq!(
        h.spans_matching("\"deploy\"", 12).expect("search").len(),
        12
    );
}

#[test]
fn a_scrollback_written_before_the_index_can_be_rebuilt() {
    let mut h = held();
    said(&mut h, "01A", 1, NOW, "we deploy with fly.io");
    said(&mut h, "01A", 2, NOW, "and the database is postgres");
    assert_eq!(h.reindex().expect("reindex"), 2);
    assert_eq!(
        h.spans_matching("\"postgres\"", 10).expect("search").len(),
        1
    );
}

#[test]
fn a_query_of_only_stopwords_matches_nothing_rather_than_erroring() {
    // The sentinel a query with no searchable terms falls back to. It has to be a term FTS5 can
    // parse and nothing can match — a NUL inside a quoted string is neither.
    let mut h = held();
    said(&mut h, "01A", 1, NOW, "we deploy with fly.io");
    let sentinel = memo_store::fts_query("is it a");
    assert!(
        h.spans_matching(&sentinel, 10).is_ok(),
        "the empty-query sentinel must parse: {sentinel:?}"
    );
    assert!(h.spans_matching(&sentinel, 10).expect("search").is_empty());
}
