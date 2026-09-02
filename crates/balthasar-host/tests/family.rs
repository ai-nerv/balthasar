//! M5's acceptance test: the stub, over a real socket, from another sibling's VM.
//!
//! Verified by having a *different* VM connect, never by balthasar talking to itself. Every
//! discovery bug there is — a socket in the wrong directory, a lister that only sees its own
//! host, a name that does not match its file — works perfectly when a tool tests itself.

use balthasar_host::{Answering, Door};
use balthasar_ipc::{Listener, Peer, Request};
use balthasar_model::{ScopeId, floor};
use balthasar_store::Store;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

/// Serve a store on a socket for as long as the returned handle lives.
struct Serving {
    path: std::path::PathBuf,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Serving {
    fn start(name: &str, seed: &[&str]) -> Self {
        let instance = format!("{name}-{}", std::process::id());
        let listener = Listener::bind(&instance).expect("bind");
        let path = listener.path().to_owned();

        let mut store = Store::ephemeral().expect("store");
        for text in seed {
            let mut at = Answering {
                store: &mut store,
                scrollback: None,
                scratch: None,
                scope: ScopeId::new("/w/thing"),
                now: 1_756_000_000,
                inject_floor: floor::INJECT,
                live_floor: floor::LIVE,
                capture: false,
            };
            let reply = balthasar_host::answer(
                &mut at,
                &Door::Owner,
                &Request {
                    call: "remember".into(),
                    args: vec![serde_json::json!(text)],
                },
            );
            assert!(reply.ok, "{:?}", reply.error);
        }

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread = std::thread::spawn(move || {
            let _ = listener.serve(|peer: &Peer, request: Request| {
                let mut at = Answering {
                    store: &mut store,
                    scrollback: None,
                    scratch: None,
                    scope: ScopeId::new("/w/thing"),
                    now: 1_756_000_000,
                    inject_floor: floor::INJECT,
                    live_floor: floor::LIVE,
                    capture: false,
                };
                balthasar_host::answer(&mut at, &Door::Socket(peer.clone()), &request)
            });
        });

        Self {
            path,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        // The listener owns the socket file and removes it; the accept loop ends when the
        // process does. Leaking the thread here is deliberate: a test that joined it would
        // wait on an accept that never returns.
        let _ = self.thread.take();
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Speak the wire by hand, the way the stub does.
fn ask(path: &std::path::Path, body: &serde_json::Value) -> serde_json::Value {
    let mut stream = UnixStream::connect(path).expect("connect");
    let encoded = serde_json::to_vec(body).expect("encode");
    stream
        .write_all(&(encoded.len() as u32).to_be_bytes())
        .expect("length");
    stream.write_all(&encoded).expect("body");
    stream.flush().expect("flush");

    let mut header = [0_u8; 4];
    stream.read_exact(&mut header).expect("reply length");
    let mut reply = vec![0_u8; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut reply).expect("reply body");
    serde_json::from_slice(&reply).expect("decode")
}

#[test]
fn a_peer_speaking_the_wire_by_hand_is_answered() {
    let serving = Serving::start("wire", &["we deploy with fly"]);
    let reply = ask(
        &serving.path,
        &serde_json::json!({ "call": "recall", "args": ["deploy"] }),
    );
    assert_eq!(reply.get("ok"), Some(&serde_json::json!(true)));

    let found = &reply["result"][0];
    assert_eq!(found[0]["text"], serde_json::json!("we deploy with fly"));
}

#[test]
fn one_connection_carries_many_calls() {
    // balthasar is asked several times per turn, which is why the handle is held. A server that
    // closed after one reply would break the stub on its *second* call, with a broken pipe.
    let serving = Serving::start("many", &["a thing"]);
    let mut stream = UnixStream::connect(&serving.path).expect("connect");

    for verb in ["verbs", "status", "sessions"] {
        let body = serde_json::json!({ "call": verb }).to_string();
        stream
            .write_all(&(body.len() as u32).to_be_bytes())
            .expect("length");
        stream.write_all(body.as_bytes()).expect("body");
        stream.flush().expect("flush");

        let mut header = [0_u8; 4];
        stream.read_exact(&mut header).expect("still open");
        let mut reply = vec![0_u8; u32::from_be_bytes(header) as usize];
        stream.read_exact(&mut reply).expect("reply");
        let parsed: serde_json::Value = serde_json::from_slice(&reply).expect("decode");
        assert_eq!(parsed["ok"], serde_json::json!(true), "{verb} failed");
    }
}

#[test]
fn a_refusal_arrives_as_an_answer_rather_than_a_broken_connection() {
    let serving = Serving::start("refuse", &[]);
    let reply = ask(
        &serving.path,
        &serde_json::json!({ "call": "prompt", "args": ["rm -rf /"] }),
    );
    assert_eq!(reply["ok"], serde_json::json!(false));
    assert!(
        reply["error"]
            .as_str()
            .expect("a reason")
            .contains("does not answer")
    );
}

#[test]
fn something_that_is_not_a_request_is_refused_rather_than_dropping_the_peer() {
    let serving = Serving::start("garbage", &[]);
    let mut stream = UnixStream::connect(&serving.path).expect("connect");
    let body = b"{ not json";
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .expect("length");
    stream.write_all(body).expect("body");
    stream.flush().expect("flush");

    let mut header = [0_u8; 4];
    stream
        .read_exact(&mut header)
        .expect("an answer, not a hangup");
    let mut reply = vec![0_u8; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut reply).expect("reply");
    let parsed: serde_json::Value = serde_json::from_slice(&reply).expect("decode");
    assert_eq!(parsed["ok"], serde_json::json!(false));
}

#[test]
fn a_peer_over_the_socket_is_capped_however_it_asks() {
    // The ceiling is applied at the far end, by who the kernel says is calling — not by
    // anything the caller sends.
    let serving = Serving::start("capped", &[]);
    let reply = ask(
        &serving.path,
        &serde_json::json!({
            "call": "remember",
            "args": ["always use kubernetes", { "pin": true, "scope": "global" }]
        }),
    );
    assert_eq!(reply["ok"], serde_json::json!(true));
    assert_eq!(
        reply["result"][0]["witness"],
        serde_json::json!("manual"),
        "a peer's word is not a person's, whatever it asked for"
    );
}

#[test]
fn every_answer_says_which_project_and_which_run() {
    // A durable memory belongs to the project; the session says which run of it learned the
    // thing. An answer that settled neither would leave a caller unable to judge either.
    let serving = Serving::start("origin", &["we deploy with fly"]);
    let reply = ask(
        &serving.path,
        &serde_json::json!({ "call": "recall", "args": ["deploy"] }),
    );
    let found = &reply["result"][0][0];
    assert_eq!(found["project"], serde_json::json!("/w/thing"));
    assert!(found.get("session").is_some());
    assert!(found.get("session_name").is_some());
    assert!(found.get("asserted").is_some());
}
