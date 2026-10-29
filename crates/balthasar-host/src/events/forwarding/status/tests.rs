#![allow(clippy::expect_used, clippy::indexing_slicing)]
use super::super::tests::pair;
use super::*;

#[test]
fn status_wire_is_exact_and_final_count_excludes_status_frames() {
    let (socket, mut sender) = pair();
    sender.producer = Id::try_from("00000000000000000000000000000002".to_owned()).expect("id");
    assert_eq!(sender.send(b"{}"), Sent::Queued);
    let mut bytes = [0; 4096];
    assert_eq!(socket.recv(&mut bytes).expect("event"), 2);
    assert_eq!(
        sender.report(Loss::default(), true),
        Report::Sent(Sent::Queued)
    );
    let len = socket.recv(&mut bytes).expect("status");
    assert_eq!(&bytes[..len], br#"{"schema":1,"program":"balthasar","producer":"00000000000000000000000000000002","status":{"revision":1,"events_sent":1,"queue_loss":{"abandoned":0,"full":0,"closed":0,"encoding":0,"exhausted":0},"transport_loss":{"full":0,"unavailable":0,"invalid":0},"finished":true}}"#);
    assert_eq!(sender.report(Loss::default(), true), Report::Closed);
    assert_eq!(sender.send(b"{}"), Sent::Invalid);
    assert!(sender.into_fresh().is_none());
}

#[test]
fn all_attempts_including_failed_final_are_rate_limited() {
    let (socket, mut sender) = pair();
    let alias = socket.try_clone().expect("inherited collector descriptor");
    socket
        .shutdown(std::net::Shutdown::Both)
        .expect("close collector");
    drop(socket);
    let now = Instant::now();
    assert_eq!(
        sender.report_at(Loss::default(), true, now),
        Report::Sent(Sent::Unavailable)
    );
    assert_eq!(
        sender.report_at(Loss::default(), true, now),
        Report::RateLimited
    );
    assert_eq!(
        sender.report_at(Loss::default(), true, now + REPORT_INTERVAL),
        Report::Sent(Sent::Unavailable)
    );
    assert_eq!(sender.next_revision, 3);
    assert_eq!(sender.loss().unavailable, 2);
    assert!(sender.into_fresh().is_none());
    drop(alias);
}

#[test]
fn counters_do_not_wrap_or_reset_after_invalid_sends() {
    let (_socket, mut sender) = pair();
    sender.next_revision = u64::MAX;
    assert_eq!(sender.report(Loss::default(), false), Report::Exhausted);
    assert_eq!(sender.next_revision, u64::MAX);
    sender.events_sent = u64::MAX;
    assert_eq!(sender.send(b"{}"), Sent::Invalid);
    assert_eq!(sender.events_sent, u64::MAX);
    assert!(sender.into_fresh().is_none());
    let (_socket, mut sender) = pair();
    assert_eq!(sender.send(b""), Sent::Invalid);
    assert!(sender.into_fresh().is_none());
}
