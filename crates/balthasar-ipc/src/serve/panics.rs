//! A call that panics is answered `failed`, and the daemon keeps answering.

use crate::{Listener, Peer, Reply, Request};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

fn say(stream: &mut UnixStream, call: &str) -> serde_json::Value {
    let body = serde_json::to_vec(&serde_json::json!({ "call": call })).expect("encode");
    let mut framed = (body.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(&body);
    stream.write_all(&framed).expect("write");
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header).expect("header");
    let mut answer = vec![0_u8; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut answer).expect("body");
    serde_json::from_slice(&answer).expect("decode")
}

#[test]
fn a_panic_while_answering_is_a_failure_and_the_next_call_is_answered() {
    let (_root, dirs) = super::aside("panics");
    let listener = Listener::bind_in(&dirs, "p").expect("bind");
    let paths: Vec<std::path::PathBuf> = listener
        .paths()
        .into_iter()
        .map(std::path::Path::to_owned)
        .collect();
    std::thread::spawn(move || {
        let _ = listener.serve(|_peer: &Peer, request: Request| {
            assert!(request.call != "boom", "a bug in a verb");
            Reply::one(serde_json::json!(request.call))
        });
    });
    let mut stream = UnixStream::connect(&paths[0]).expect("connect");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .expect("a deadline");
    let told = say(&mut stream, "boom");
    assert_eq!(told["fault"], "failed", "{told}");
    assert!(told["error"].as_str().unwrap_or_default().contains("boom"));
    assert_eq!(say(&mut stream, "verbs")["result"][0], "verbs");
}
