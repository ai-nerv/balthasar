use super::*;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Default)]
struct State {
    calls: usize,
    dropped: Option<mpsc::Sender<usize>>,
    retaining: bool,
}

impl ConnectionState for State {
    fn retains_connection(&self) -> bool {
        self.retaining
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(dropped) = &self.dropped {
            let _ = dropped.send(self.calls);
        }
    }
}

fn joined(thread: JoinHandle<()>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !thread.is_finished() {
        assert!(Instant::now() < deadline, "connection cleanup deadline");
        std::thread::sleep(Duration::from_millis(5));
    }
    thread.join().expect("served thread");
}

struct Socket {
    stream: UnixStream,
    thread: Option<JoinHandle<()>>,
    retained: Arc<AtomicBool>,
}

impl Socket {
    fn new(asked: &mpsc::Sender<Heard>, ids: &AtomicU64) -> Self {
        Self::within(asked, ids, super::super::PATIENCE)
    }

    fn within(
        asked: &mpsc::Sender<Heard>,
        ids: &AtomicU64,
        patience: super::super::Patience,
    ) -> Self {
        Self::observed(asked, ids, patience, None)
    }

    fn observed(
        asked: &mpsc::Sender<Heard>,
        ids: &AtomicU64,
        patience: super::super::Patience,
        revision: Option<u8>,
    ) -> Self {
        let (stream, mut serving) = UnixStream::pair().expect("socket pair");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("read bound");
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .expect("write bound");
        serving
            .set_read_timeout(Some(Duration::from_millis(20)))
            .expect("server bound");
        serving
            .set_write_timeout(Some(Duration::from_secs(3)))
            .expect("server write bound");
        let peer = Peer::of(&serving).expect("peer");
        assert_eq!(peer.pid, std::process::id() as i32);
        let connection = Connection::new(asked.clone(), ids).expect("identity");
        let retained = Arc::clone(&connection.retained);
        let thread = std::thread::spawn(move || {
            connection.converse(
                &mut serving,
                &peer,
                &crate::key::guard_for_tests().with_request_context(revision),
                patience,
            );
        });
        Self {
            stream,
            thread: Some(thread),
            retained,
        }
    }

    fn send(&mut self, keyed: bool, call: &str) {
        let mut request = serde_json::json!({"call": call, "args": []});
        if keyed {
            request["key"] = serde_json::json!(crate::key::for_tests().1);
        }
        crate::frame::send(
            &mut self.stream,
            &serde_json::to_vec(&request).expect("encode"),
        )
        .expect("send");
    }

    fn reply(&mut self) -> serde_json::Value {
        let body = crate::frame::recv_within(&mut self.stream, Some(Duration::from_secs(3)))
            .expect("reply frame");
        serde_json::from_slice(&body).expect("reply JSON")
    }

    fn call(&mut self) -> serde_json::Value {
        self.send(true, "observe");
        self.reply()
    }
}

#[test]
fn authenticated_metadata_travels_with_each_queued_request_without_persistent_connection_state() {
    let (asked, asking) = mpsc::channel();
    let dispatcher = std::thread::spawn(move || {
        dispatch_observed::<State>(asking, |_, state, request, context| {
            state.calls += 1;
            Reply::one(serde_json::json!({"context":context,"request":request,"calls":state.calls}))
        });
    });
    let ids = AtomicU64::new(1);
    let mut socket = Socket::observed(&asked, &ids, super::super::PATIENCE, Some(1));
    for (key, context, expected) in [
        (
            Some(crate::key::for_tests().1),
            serde_json::json!({"turn":1}),
            serde_json::json!({"turn":1}),
        ),
        (None, serde_json::json!({"turn":2}), serde_json::Value::Null),
        (
            Some("wrong"),
            serde_json::json!({"turn":3}),
            serde_json::Value::Null,
        ),
        (
            Some(crate::key::for_tests().1),
            serde_json::Value::Null,
            serde_json::Value::Null,
        ),
        (
            Some(crate::key::for_tests().1),
            serde_json::json!({"turn":4}),
            serde_json::json!({"turn":4}),
        ),
    ] {
        let request = serde_json::json!({"call":"verbs","args":[{"event_context":"model-data"}],"key":key,"event_context":context});
        crate::frame::send(
            &mut socket.stream,
            &serde_json::to_vec(&request).expect("wire"),
        )
        .expect("send");
        let reply = socket.reply();
        assert_eq!(reply["request_context"], 1);
        assert_eq!(reply["result"][0]["context"], expected);
        assert_eq!(reply["result"][0]["request"]["args"], request["args"]);
        assert!(reply["result"][0]["request"].get("event_context").is_none());
    }
    socket.send(false, "observe");
    assert_eq!(socket.reply()["ok"], false);
    socket.send(true, "observe");
    let reply = socket.reply();
    assert_eq!(reply["result"][0]["calls"], 6);
    assert!(reply.get("request_context").is_none());
    assert!(reply["result"][0]["context"].is_null());
    drop(socket);
    drop(asked);
    joined(dispatcher);
}

impl Drop for Socket {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
        if let Some(thread) = self.thread.take() {
            joined(thread);
        }
    }
}

#[test]
fn same_pid_connections_have_independent_state_and_disconnect_drops_only_its_owner() {
    let (asked, asking) = mpsc::channel();
    let (dropped, deaths) = mpsc::channel();
    let dispatcher = std::thread::spawn(move || {
        dispatch::<State>(asking, |_, state, _| {
            state.dropped = Some(dropped.clone());
            state.calls += 1;
            Reply::one(serde_json::json!(state.calls))
        })
    });
    let ids = AtomicU64::new(1);
    let mut a = Socket::new(&asked, &ids);
    let mut b = Socket::new(&asked, &ids);
    assert_eq!(a.call()["result"][0], 1);
    assert_eq!(a.call()["result"][0], 2);
    assert_eq!(b.call()["result"][0], 1);
    drop(a);
    assert_eq!(
        deaths.recv_timeout(Duration::from_secs(3)).expect("drop a"),
        2
    );
    assert!(matches!(deaths.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert_eq!(b.call()["result"][0], 2);
    drop(b);
    assert_eq!(
        deaths.recv_timeout(Duration::from_secs(3)).expect("drop b"),
        2
    );
    drop(asked);
    joined(dispatcher);
}

#[test]
fn rejected_calls_never_create_dispatcher_state() {
    let (asked, asking) = mpsc::channel();
    let (dropped, deaths) = mpsc::channel();
    let calls = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&calls);
    let dispatcher = std::thread::spawn(move || {
        dispatch::<State>(asking, |_, state, _| {
            counted.fetch_add(1, Ordering::Relaxed);
            state.dropped = Some(dropped.clone());
            Reply::none()
        })
    });
    let mut socket = Socket::new(&asked, &AtomicU64::new(1));
    socket.send(false, "observe");
    assert_eq!(socket.reply()["fault"], "refused");
    drop(socket);
    drop(asked);
    joined(dispatcher);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(deaths.try_recv().is_err());
}

#[test]
fn dispatcher_stop_drops_state_even_when_client_keeps_its_socket() {
    let (asked, asking) = mpsc::channel();
    let (dropped, deaths) = mpsc::channel();
    let dispatcher = std::thread::spawn(move || {
        dispatch::<State>(asking, |_, state, _| {
            state.dropped = Some(dropped.clone());
            state.calls += 1;
            state.retaining = true;
            Reply::none()
        })
    });
    let mut socket = Socket::new(&asked, &AtomicU64::new(1));
    assert_eq!(socket.call()["ok"], true);
    assert!(socket.retained.load(Ordering::Acquire));
    asked.send(Heard::Stop("fixture")).expect("stop");
    assert_eq!(
        deaths
            .recv_timeout(Duration::from_secs(3))
            .expect("release"),
        1
    );
    joined(dispatcher);
    assert!(!socket.retained.load(Ordering::Acquire));
    assert_eq!(socket.call()["fault"], "refused");
}

#[test]
fn disconnect_during_a_call_keeps_state_until_that_call_settles() {
    let (asked, asking) = mpsc::channel();
    let (dropped, deaths) = mpsc::channel();
    let (entered, entering) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let dispatcher = std::thread::spawn(move || {
        dispatch::<State>(asking, |_, state, _| {
            state.dropped = Some(dropped.clone());
            state.calls += 1;
            entered.send(()).expect("entered");
            released
                .recv_timeout(Duration::from_secs(3))
                .expect("release call");
            Reply::none()
        })
    });
    let mut socket = Socket::new(&asked, &AtomicU64::new(1));
    socket.send(true, "observe");
    entering
        .recv_timeout(Duration::from_secs(3))
        .expect("active call");
    socket.stream.shutdown(Shutdown::Both).expect("disconnect");
    assert!(matches!(deaths.try_recv(), Err(mpsc::TryRecvError::Empty)));
    release.send(()).expect("settle");
    drop(socket);
    assert_eq!(
        deaths
            .recv_timeout(Duration::from_secs(3))
            .expect("owner released"),
        1
    );
    drop(asked);
    joined(dispatcher);
}

#[test]
fn unused_connections_do_not_queue_closures_and_ids_never_wrap() {
    let (asked, asking) = mpsc::channel();
    let ids = AtomicU64::new(u64::MAX - 1);
    let connection = Connection::new(asked.clone(), &ids).expect("last identity");
    assert_eq!(connection.id, u64::MAX - 1);
    assert!(Connection::new(asked, &ids).is_none());
    drop(connection);
    assert!(matches!(
        asking.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
}

#[test]
fn owned_idle_connections_keep_the_lock_but_release_and_partial_frames_still_end_them() {
    use std::io::Write;
    for partial in [false, true] {
        let (asked, asking) = mpsc::channel();
        let (dropped, deaths) = mpsc::channel();
        let dispatcher = std::thread::spawn(move || {
            dispatch::<State>(asking, |_, state, request| {
                state.dropped = Some(dropped.clone());
                state.calls += 1;
                match request.call.as_str() {
                    "hold" => state.retaining = true,
                    "release" => state.retaining = false,
                    _ => {}
                }
                Reply::none()
            })
        });
        let patience = super::super::Patience {
            idle: Duration::from_millis(80),
            frame: Duration::from_millis(80),
            write: Duration::from_secs(3),
        };
        let mut socket = Socket::within(&asked, &AtomicU64::new(1), patience);
        socket.send(true, "hold");
        assert_eq!(socket.reply()["ok"], true);
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(socket.call()["ok"], true);
        assert!(socket.retained.load(Ordering::Acquire));
        if partial {
            socket.stream.write_all(&[0, 0]).expect("partial frame");
        } else {
            socket.send(true, "release");
            assert_eq!(socket.reply()["ok"], true);
            assert!(!socket.retained.load(Ordering::Acquire));
        }
        assert!(
            crate::frame::recv_within(&mut socket.stream, Some(Duration::from_secs(3))).is_err()
        );
        assert_eq!(
            deaths
                .recv_timeout(Duration::from_secs(3))
                .expect("state released"),
            if partial { 2 } else { 3 }
        );
        drop(socket);
        drop(asked);
        joined(dispatcher);
    }
}
