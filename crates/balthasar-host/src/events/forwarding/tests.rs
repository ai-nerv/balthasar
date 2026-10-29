#![allow(clippy::expect_used, clippy::indexing_slicing)]
use super::*;
use crate::events::{
    Correlation, Emitted, Emitter, Fact, Id, LayoutOutcome, Program, QUEUED_EVENTS,
};
use rustix::net::{self, sockopt};
use std::os::unix::net::UnixDatagram;

pub(super) fn pair() -> (UnixDatagram, Sender) {
    let (read, write) = UnixDatagram::pair().expect("private pair");
    write.set_nonblocking(true).expect("nonblocking");
    read.set_nonblocking(true).expect("nonblocking");
    for fd in [&read, &write] {
        sockopt::set_socket_send_buffer_size(fd, 16384).expect("send buffer");
        sockopt::set_socket_recv_buffer_size(fd, 16384).expect("receive buffer");
    }
    let sender = transport::Sender::adopt(write.into()).expect("adopt");
    (
        read,
        Sender::new(sender, Program::Balthasar, Id::generate().expect("id")),
    )
}

fn prepared() -> (Correlation, Fact) {
    (
        Correlation::default(),
        Fact::Layout {
            outcome: LayoutOutcome::Decided,
            duration_ms: None,
        },
    )
}

fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready() {
        assert!(Instant::now() < deadline, "forwarder barrier");
        thread::sleep(Duration::from_millis(5));
    }
}

fn frame(socket: &UnixDatagram) -> serde_json::Value {
    let mut bytes = [0; 4096];
    let len = socket.recv(&mut bytes).expect("frame");
    serde_json::from_slice(&bytes[..len]).expect("json")
}

#[test]
fn native_forwarding_preserves_identity_and_confirms_final_counts() {
    let (socket, sender) = pair();
    let id = sender.identity();
    let (emitter, receiver) = Emitter::new(id, [1, 2, 3]);
    let worker = Forwarder::spawn(receiver, sender, run).expect("worker");
    assert_eq!(emitter.emit(prepared), Emitted::Queued);
    wait(|| emitter.loss.dequeued.load(Ordering::Relaxed) == 1);
    let outcome = worker.shutdown().expect("joined");
    assert!(!outcome.incomplete());
    let event = frame(&socket);
    assert_eq!(event["producer"], String::from(id));
    assert_eq!(event["program"], "balthasar");
    let status = frame(&socket);
    assert_eq!(status["status"]["finished"], true);
    assert_eq!(status["status"]["events_sent"], 1);
    assert_eq!(
        emitter.emit(|| panic!("closed payload evaluated")),
        Emitted::Closed
    );
}

#[test]
fn queued_abandonment_is_counted_without_unbounded_flush() {
    let (_socket, sender) = pair();
    let (emitter, receiver) = Emitter::new(sender.identity(), [1, 2, 3]);
    for _ in 0..QUEUED_EVENTS {
        assert_eq!(emitter.emit(prepared), Emitted::Queued);
    }
    assert_eq!(emitter.emit(prepared), Emitted::Full);
    let worker = Forwarder::spawn(receiver, sender, |r, s, _| (r, s)).expect("worker");
    let outcome = worker.shutdown().expect("joined");
    assert_eq!(outcome.queue_loss.abandoned, QUEUED_EVENTS as u64);
    assert_eq!(outcome.queue_loss.full, 1);
    assert!(outcome.incomplete());
}

#[test]
fn in_flight_payload_remains_unconfirmed_without_waiting_for_it() {
    let (_socket, sender) = pair();
    let (emitter, receiver) = Emitter::new(sender.identity(), [1, 2, 3]);
    let emitter = Arc::new(emitter);
    let worker = Forwarder::spawn(receiver, sender, run).expect("worker");
    let (entered, entering) = std::sync::mpsc::sync_channel(1);
    let (release, released) = std::sync::mpsc::sync_channel(1);
    let producer = thread::spawn({
        let emitter = emitter.clone();
        move || {
            emitter.emit(|| {
                entered.send(()).expect("entered");
                released
                    .recv_timeout(Duration::from_secs(3))
                    .expect("release");
                prepared()
            })
        }
    });
    entering
        .recv_timeout(Duration::from_secs(3))
        .expect("entered");
    let outcome = worker.shutdown().expect("joined");
    release.send(()).expect("released");
    assert_eq!(producer.join().expect("producer"), Emitted::Closed);
    assert!(outcome.local_unconfirmed && outcome.incomplete());
}

#[test]
fn panicking_payload_records_encoding_loss_and_clears_active_guard() {
    let (_socket, sender) = pair();
    let (emitter, _receiver) = Emitter::new(sender.identity(), [1, 2, 3]);
    assert!(std::panic::catch_unwind(|| emitter.emit(|| panic!("payload"))).is_err());
    assert_eq!(emitter.loss.active.load(Ordering::SeqCst), 0);
    assert_eq!(emitter.loss.snapshot().encoding, 1);
}

#[test]
fn full_and_closed_transport_do_not_block_or_reconnect() {
    let (socket, mut sender) = pair();
    let mut full = false;
    for _ in 0..10000 {
        if sender.send(&[b'x'; 4096]) == Sent::Full {
            full = true;
            break;
        }
    }
    assert!(full);
    assert_eq!(
        sender.report(Loss::default(), true),
        Report::Sent(Sent::Full)
    );
    assert_eq!(sender.report(Loss::default(), true), Report::RateLimited);
    drop(socket);
    let (socket, mut sender) = pair();
    net::shutdown(&socket, net::Shutdown::Both).expect("close empty collector");
    assert_eq!(sender.send(b"{}"), Sent::Unavailable);
    assert_eq!(sender.loss().unavailable, 1);
}

#[test]
fn used_sender_cannot_reset_accounting_on_activation() {
    let (_socket, mut sender) = pair();
    assert_eq!(sender.send(b"{}"), Sent::Queued);
    assert!(matches!(
        Forwarder::activate(sender),
        Err(ForwardError::UsedSender)
    ));
}
