use balthasar_model::{SessionId, scratch::Scratch};
use balthasar_store::Transcript;
use serde_json::json;

fn ack(store: &Transcript, session: &SessionId, job: &str, from: u64, to: u64) {
    store
        .atomic(|tx| tx.acknowledge_extraction(session, job, from, to))
        .expect("acknowledge");
}

#[test]
fn out_of_order_ranges_preserve_gaps_across_reopen_and_duplicate_completion() {
    let dir = Scratch::new("extraction", "gaps");
    let path = dir.join("transcript.db");
    let session = SessionId::new("session");
    let store = Transcript::open(&path).expect("open");
    ack(&store, &session, "later", 5, 9);
    assert_eq!(
        store
            .extraction_progress(&session)
            .expect("store result")
            .through,
        None
    );
    ack(&store, &session, "first", 0, 2);
    assert_eq!(
        store
            .extraction_progress(&session)
            .expect("store result")
            .through,
        Some(2)
    );
    drop(store);
    let store = Transcript::open(&path).expect("reopen");
    ack(&store, &session, "first", 0, 2);
    assert_eq!(
        store
            .extraction_progress(&session)
            .expect("store result")
            .through,
        Some(2)
    );
    ack(&store, &session, "gap", 3, 4);
    assert_eq!(
        store
            .extraction_progress(&session)
            .expect("store result")
            .through,
        Some(9)
    );
    assert_eq!(
        store.counter(&session, "extracted").expect("store result"),
        Some(9)
    );
    let count: u64 = rusqlite::Connection::open(&path)
        .expect("store result")
        .query_row("SELECT count(*) FROM extraction_ack", [], |r| r.get(0))
        .expect("store result");
    assert_eq!(count, 3);
}

#[test]
fn acknowledgment_identity_cannot_change_range_or_session() {
    let store = Transcript::ephemeral().expect("store");
    let session = SessionId::new("original");
    let other = SessionId::new("other");
    ack(&store, &session, "job", 0, 2);
    for (owner, from, to) in [(&session, 0, 3), (&other, 0, 2), (&session, 3, 2)] {
        assert!(
            store
                .atomic(|tx| tx.acknowledge_extraction(owner, "job", from, to))
                .is_err()
        );
    }
    assert_eq!(
        store
            .extraction_progress(&session)
            .expect("store result")
            .through,
        Some(2)
    );
    assert_eq!(
        store
            .extraction_progress(&other)
            .expect("store result")
            .through,
        None
    );
}

#[test]
fn legacy_frontier_stays_explicitly_uncertain_after_new_acknowledgment() {
    let dir = Scratch::new("extraction", "legacy");
    let path = dir.join("transcript.db");
    let store = Transcript::open(&path).expect("store");
    let session = SessionId::new("legacy");
    store
        .set_counter(&session, "extracted", 7)
        .expect("store result");
    let progress = store.extraction_progress(&session).expect("store result");
    assert_eq!(progress.through, Some(7));
    assert_eq!(progress.legacy_through, Some(7));
    let rows: u64 = rusqlite::Connection::open(&path)
        .expect("store result")
        .query_row("SELECT count(*) FROM extraction_progress", [], |r| r.get(0))
        .expect("store result");
    assert_eq!(rows, 0, "inspection does not migrate or mutate");
    ack(&store, &session, "new", 8, 9);
    let progress = store.extraction_progress(&session).expect("store result");
    assert_eq!(progress.through, Some(9));
    assert_eq!(progress.legacy_through, Some(7));
}

#[test]
fn maximum_sqlite_cursor_is_idempotent_and_invalid_larger_values_roll_back() {
    let store = Transcript::ephemeral().expect("store");
    let session = SessionId::new("maximum");
    let max = i64::MAX as u64;
    ack(&store, &session, "all", 0, max);
    ack(&store, &session, "all", 0, max);
    assert!(
        store
            .atomic(|tx| tx.acknowledge_extraction(&session, "overflow", max, max + 1))
            .is_err()
    );
    assert_eq!(
        store
            .extraction_progress(&session)
            .expect("store result")
            .through,
        Some(max)
    );
}

#[test]
fn root_purge_removes_child_acknowledgments_and_blocked_progress() {
    let dir = Scratch::new("extraction", "purge");
    let path = dir.join("transcript.db");
    let store = Transcript::open(&path).expect("store");
    let root = SessionId::new("root");
    let child = SessionId::new("child");
    let other = SessionId::new("other");
    store.bind_run(&child, Some(root.as_str())).expect("bind");
    ack(&store, &root, "root-job", 0, 1);
    ack(&store, &child, "child-job", 0, 2);
    ack(&store, &other, "other-job", 0, 3);
    store
        .block_extraction(&child, Some(&json!({"cursor":3,"reason":"oversized"})))
        .expect("store result");
    balthasar_store::purge_run(&store, &root).expect("purge");
    for session in [&root, &child] {
        let progress = store.extraction_progress(session).expect("store result");
        assert_eq!(progress.through, None);
        assert_eq!(progress.blocked, None);
    }
    let rows: u64 = rusqlite::Connection::open(&path)
        .expect("store result")
        .query_row("SELECT count(*) FROM extraction_ack", [], |r| r.get(0))
        .expect("store result");
    assert_eq!(rows, 1);
    assert_eq!(
        store
            .extraction_progress(&other)
            .expect("store result")
            .through,
        Some(3)
    );
}
