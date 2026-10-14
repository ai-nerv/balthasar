//! The frame cap on the serving side: too large is answered, never a silent close.

use super::converse;
use crate::{MAX_FRAME, Peer, Reply, Request};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

/// A connection served by `answer` on its own thread.
fn served(answer: fn(&Peer, Request) -> Reply) -> UnixStream {
    let (client, mut server) = UnixStream::pair().expect("pair");
    let peer = Peer::of(&server).expect("the kernel names us");
    std::thread::spawn(move || converse(&mut server, &peer, &answer));
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .expect("a deadline");
    client
}

/// Read one reply frame, or `None` at end of file.
fn reply(stream: &mut UnixStream) -> Option<serde_json::Value> {
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header).ok()?;
    let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn ask(stream: &mut UnixStream, call: &str) {
    let body = serde_json::to_vec(&serde_json::json!({ "call": call })).expect("encode");
    let mut framed = (body.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(&body);
    stream.write_all(&framed).expect("write");
}

fn echo(_peer: &Peer, request: Request) -> Reply {
    Reply::one(serde_json::json!(request.call))
}

#[test]
fn an_oversized_call_gets_one_failure_naming_the_cap_and_then_a_close() {
    let mut client = served(echo);
    let header = u32::try_from(MAX_FRAME + 1).expect("fits").to_be_bytes();
    client.write_all(&header).expect("header");
    let told = reply(&mut client).expect("a refusal, not silence");
    assert_eq!(told["ok"], serde_json::json!(false));
    assert_eq!(told["fault"], serde_json::json!("failed"));
    let said = told["error"].as_str().unwrap_or_default();
    assert!(said.contains(&MAX_FRAME.to_string()), "{said}");
    assert!(
        reply(&mut client).is_none(),
        "and then the connection is closed"
    );
}

#[test]
fn a_frame_of_zero_length_is_refused_and_the_connection_stays_in_step() {
    let mut client = served(echo);
    client.write_all(&[0, 0, 0, 0]).expect("empty frame");
    let told = reply(&mut client).expect("a refusal");
    assert_eq!(told["ok"], serde_json::json!(false));
    assert!(
        told["error"].as_str().unwrap_or_default().contains("zero"),
        "{told}"
    );
    ask(&mut client, "verbs");
    assert_eq!(reply(&mut client).expect("next")["result"][0], "verbs");
}

#[test]
fn a_frame_at_the_cap_is_read() {
    let client = served(echo);
    let shell = br#"{"call":"big","args":[""]}"#.len();
    let padding = "x".repeat(MAX_FRAME - shell);
    let body = serde_json::to_vec(&serde_json::json!({ "call": "big", "args": [padding] }))
        .expect("encode");
    assert_eq!(body.len(), MAX_FRAME);
    let mut framed = (body.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(&body);
    let mut client = std::thread::spawn(move || {
        let mut client = client;
        client.write_all(&framed).expect("write");
        client
    })
    .join()
    .expect("written");
    let told = reply(&mut client).expect("an answer");
    assert_eq!(told["result"][0], "big", "{}", told["error"]);
}

fn huge(_peer: &Peer, request: Request) -> Reply {
    if request.call == "huge" {
        return Reply::one(serde_json::json!("y".repeat(MAX_FRAME + 1)));
    }
    Reply::one(serde_json::json!(request.call))
}

#[test]
fn a_reply_over_the_cap_is_replaced_by_a_refusal_and_the_next_call_is_in_step() {
    let mut client = served(huge);
    ask(&mut client, "huge");
    let told = reply(&mut client).expect("a refusal in its place");
    assert_eq!(told["fault"], serde_json::json!("failed"));
    assert!(
        told["error"].as_str().unwrap_or_default().contains("cap"),
        "{told}"
    );
    ask(&mut client, "small");
    assert_eq!(reply(&mut client).expect("next")["result"][0], "small");
}
