//! A caller that holds its connection must not be the only one that can have it.

use super::*;
use std::io::{Read, Write};

/// A served socket in directories of the test's own, removed with them however the test
/// ends: the accept loop never returns, so the [`Listener`] moved into the serving thread
/// never drops.
struct Bound {
    paths: Vec<PathBuf>,
    _root: balthasar_model::scratch::Scratch,
}

impl Bound {
    /// Bind, and serve every call by echoing back what was asked.
    fn serving(name: &str) -> Self {
        let (root, dirs) = aside(name);
        let listener = Listener::bind_in(&dirs, "c").expect("bind");
        let paths = listener.paths().into_iter().map(Path::to_owned).collect();
        std::thread::spawn(move || {
            let _ = listener.serve(
                crate::key::for_tests().0,
                |_peer: &Peer, request: Request| {
                    Reply::one(serde_json::json!(format!("heard {}", request.call)))
                },
            );
        });
        Self { paths, _root: root }
    }

    /// The name a caller in these tests dials.
    fn path(&self) -> &Path {
        &self.paths[0]
    }
}

/// One call with the key, framed.
fn keyed(call: &str) -> Vec<u8> {
    let hex = crate::key::for_tests().1;
    let body = serde_json::json!({ "call": call, "args": [], "key": hex });
    let body = serde_json::to_vec(&body).expect("encode");
    let mut framed = (body.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(&body);
    framed
}

/// Ask, taking a closed connection for an answer: over the ceiling, `connect` still succeeds
/// and the write races the server's close.
fn try_say(stream: &mut UnixStream, call: &str) -> Option<serde_json::Value> {
    stream.write_all(&keyed(call)).ok()?;
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header).ok()?;
    let mut answer = vec![0_u8; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut answer).ok()?;
    serde_json::from_slice(&answer).ok()
}

fn say(stream: &mut UnixStream, call: &str) -> serde_json::Value {
    stream.write_all(&keyed(call)).expect("write");

    let mut header = [0_u8; 4];
    stream.read_exact(&mut header).expect("header");
    let mut answer = vec![0_u8; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut answer).expect("body");
    serde_json::from_slice(&answer).expect("decode")
}

#[test]
fn one_caller_holding_its_connection_does_not_shut_the_next_one_out() {
    let bound = Bound::serving("crowd");
    let path = bound.path();

    let mut scribe = UnixStream::connect(path).expect("the scribe connects");
    assert_eq!(
        say(&mut scribe, "remember")["result"][0],
        serde_json::json!("heard remember")
    );

    let mut probe = UnixStream::connect(path).expect("the child's probe connects");
    probe
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("a deadline");
    let answered = say(&mut probe, "verbs");
    assert_eq!(answered["ok"], serde_json::json!(true));
    assert_eq!(answered["result"][0], serde_json::json!("heard verbs"));

    assert_eq!(
        say(&mut scribe, "keep")["result"][0],
        serde_json::json!("heard keep")
    );

    drop(probe);
    drop(scribe);
}

#[test]
fn the_caller_over_the_ceiling_is_told_so_rather_than_left_hanging() {
    // A refusal is a frame; a dropped connection is what gets the socket unlinked.
    let bound = Bound::serving("ceiling");
    let path = bound.path();

    let mut held: Vec<UnixStream> = Vec::new();
    for _ in 0..CALLERS {
        let mut caller = UnixStream::connect(path).expect("connect");
        assert_eq!(
            say(&mut caller, "verbs")["result"][0],
            serde_json::json!("heard verbs")
        );
        held.push(caller);
    }

    let mut over = UnixStream::connect(path).expect("connect");
    over.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .expect("a deadline");
    let mut header = [0_u8; 4];
    over.read_exact(&mut header)
        .expect("a refusal, not silence");
    let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
    over.read_exact(&mut body).expect("the refusal's body");
    let told: serde_json::Value = serde_json::from_slice(&body).expect("decode");
    assert_eq!(told["ok"], serde_json::json!(false));
    assert_eq!(told["fault"], serde_json::json!("failed"));
    assert!(
        told["error"]
            .as_str()
            .unwrap_or_default()
            .starts_with("busy"),
        "{told}"
    );

    // The seat comes back when the reading thread notices the close, a moment after it.
    drop(held.pop());
    let mut taken = None;
    for _ in 0..100 {
        let mut next = UnixStream::connect(path).expect("connect");
        next.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("a deadline");
        if try_say(&mut next, "verbs").is_some_and(|said| said["ok"] == serde_json::json!(true)) {
            taken = Some(next);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(taken.is_some(), "a seat given back was never taken");

    drop(taken);
    drop(held);
}
