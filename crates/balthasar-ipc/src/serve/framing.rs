//! The frame cap on the serving side: too large is answered, never a silent close.

use super::converse;
use crate::{MAX_FRAME, Peer, Reply, Request};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

/// A connection served by `answer` on its own thread.
fn served(answer: fn(&Peer, Request) -> Reply) -> UnixStream {
    let (client, mut server) = UnixStream::pair().expect("pair");
    let peer = Peer::of(&server).expect("the kernel names us");
    let key = crate::key::for_tests().0;
    std::thread::spawn(move || converse(&mut server, &peer, &key, &answer));
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
    send(stream, &serde_json::json!({ "call": call, "key": HEX }));
}

/// The key every call in these tests carries, unless it is the one under test.
const HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn send(stream: &mut UnixStream, call: &serde_json::Value) {
    let body = serde_json::to_vec(call).expect("encode");
    let mut framed = (body.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(&body);
    stream.write_all(&framed).expect("write");
}

#[test]
fn a_call_without_the_key_or_with_the_wrong_one_is_refused_and_the_next_is_answered() {
    let mut client = served(echo);
    let wrong = HEX.replace('0', "1");
    for call in [
        serde_json::json!({ "call": "verbs" }),
        serde_json::json!({ "call": "verbs", "key": wrong }),
        serde_json::json!({ "call": "verbs", "key": 7 }),
    ] {
        send(&mut client, &call);
        let told = reply(&mut client).expect("a refusal, not a close");
        assert_eq!(told["fault"], "refused", "{call}: {told}");
        assert_eq!(told["error"], crate::KEY_REFUSED, "{call}: {told}");
    }
    ask(&mut client, "verbs");
    assert_eq!(reply(&mut client).expect("answered")["result"][0], "verbs");
}

#[test]
fn a_call_in_a_newer_family_is_refused_by_name_and_the_next_is_answered() {
    let mut client = served(echo);
    send(
        &mut client,
        &serde_json::json!({ "call": "verbs", "family": 2, "key": HEX }),
    );
    let told = reply(&mut client).expect("a refusal, not a close");
    assert_eq!(told["fault"], "refused", "{told}");
    assert!(
        told["error"]
            .as_str()
            .unwrap_or_default()
            .contains("family 2"),
        "{told}"
    );
    send(
        &mut client,
        &serde_json::json!({ "call": "older", "family": 1, "key": HEX }),
    );
    assert_eq!(reply(&mut client).expect("answered")["result"][0], "older");
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
    let shell = serde_json::json!({ "call": "big", "args": [""], "key": HEX })
        .to_string()
        .len();
    let padding = "x".repeat(MAX_FRAME - shell);
    let call = serde_json::json!({ "call": "big", "args": [padding], "key": HEX });
    let body = serde_json::to_vec(&call).expect("encode");
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

/// A connection served on its own thread with short deadlines, and a way to see it end.
fn hurried(
    answer: fn(&Peer, Request) -> Reply,
    patience: super::Patience,
) -> (UnixStream, std::sync::mpsc::Receiver<()>) {
    let (client, mut server) = UnixStream::pair().expect("pair");
    server
        .set_read_timeout(Some(std::time::Duration::from_millis(20)))
        .expect("read deadline");
    server
        .set_write_timeout(Some(patience.write))
        .expect("write deadline");
    let peer = Peer::of(&server).expect("the kernel names us");
    let (ended, ending) = std::sync::mpsc::channel();
    let key = crate::key::for_tests().0;
    std::thread::spawn(move || {
        super::converse_within(&mut server, &peer, &key, &answer, patience);
        let _ = ended.send(());
    });
    (client, ending)
}

const SHORT: super::Patience = super::Patience {
    frame: std::time::Duration::from_millis(200),
    idle: std::time::Duration::from_millis(200),
    write: std::time::Duration::from_millis(200),
};

#[test]
fn a_connection_that_says_nothing_is_closed_once_idle() {
    let (mut client, ending) = hurried(echo, SHORT);
    ask(&mut client, "verbs");
    assert!(reply(&mut client).is_some());
    ending
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("closed after going quiet");
}

#[test]
fn a_frame_that_stalls_part_way_is_given_up_on() {
    let (mut client, ending) = hurried(echo, SHORT);
    client.write_all(&[0, 0, 0, 40, b'{']).expect("a start");
    ending
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("closed rather than waiting on the rest");
}

fn sizeable(_peer: &Peer, _request: Request) -> Reply {
    Reply::one(serde_json::json!("w".repeat(8 * 1024 * 1024)))
}

#[test]
fn a_peer_that_never_reads_its_reply_is_let_go() {
    let (mut client, ending) = hurried(sizeable, SHORT);
    ask(&mut client, "big");
    ending
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the write gave up and the connection closed");
    drop(client);
}
