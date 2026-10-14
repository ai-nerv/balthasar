//! `handle:call`: a family call framed, sent, read and decoded by the host, against peers that
//! answer properly, short, too much, or not at all.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_lua::Engine;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// A socket in the memory role's fallback directory, which a config's stream may dial.
fn socket(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("memory-{}", balthasar_model::runtime::uid()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir.join(format!("call-{name}-{}.sock", std::process::id()))
}

/// Read one framed call, or `None` when the peer went away.
fn heard(socket: &mut UnixStream) -> Option<serde_json::Value> {
    let mut head = [0_u8; 4];
    socket.read_exact(&mut head).ok()?;
    let mut body = vec![0_u8; u32::from_be_bytes(head) as usize];
    socket.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// Write one framed reply.
fn answer(socket: &mut UnixStream, body: &[u8]) {
    let mut out = u32::try_from(body.len())
        .expect("fits")
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(body);
    let _ = socket.write_all(&out);
}

/// A peer that answers each call by naming every call it has heard, then does `last` with the
/// call after `calls` of them.
fn peer(
    name: &str,
    calls: usize,
    last: fn(&mut UnixStream),
) -> (PathBuf, std::thread::JoinHandle<()>) {
    let path = socket(name);
    let _ = std::fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
    let served = std::thread::spawn(move || {
        let Ok((mut socket, _)) = listener.accept() else {
            return;
        };
        let mut names = Vec::new();
        while let Some(call) = heard(&mut socket) {
            names.push(call["call"].as_str().unwrap_or_default().to_owned());
            if names.len() > calls {
                last(&mut socket);
                return;
            }
            let reply = serde_json::json!({
                "ok": true, "family": 1, "n": 1, "result": [names.join(",")]
            });
            answer(&mut socket, reply.to_string().as_bytes());
        }
    });
    (path, served)
}

/// Run `script` against a handle connected to `path`, and take what it left in `balthasar.answer`.
fn through(path: &std::path::Path, script: &str) -> String {
    let mut engine = Engine::new();
    let source = format!(
        "local h = assert(balthasar.stream.connect({path:?}, 2000))\n{script}",
        path = path.to_string_lossy()
    );
    engine
        .run(&source, "call.lua")
        .expect("the call must not raise");
    engine.harvest();
    let said = engine
        .config()
        .string("answer")
        .unwrap_or_default()
        .to_owned();
    let _ = std::fs::remove_file(path);
    said
}

#[test]
fn a_call_is_framed_and_its_reply_decoded() {
    let (path, served) = peer("plain", 2, |_| {});
    let said = through(
        &path,
        "local one = h:call({ call = 'verbs' })\n\
         local two = h:call({ call = 'recall', args = { 'x' } }, { deadline_ms = 500 })\n\
         balthasar.answer = tostring(one.ok) .. '|' .. one.n .. '|' .. two.result[1]",
    );
    assert_eq!(said, "true|1|verbs,recall");
    drop(served);
}

#[test]
fn a_call_one_byte_over_the_cap_is_refused_before_it_is_sent() {
    let (path, served) = peer("over", 2, |_| {});
    let said = through(
        &path,
        "local body = string.rep('x', 16777216 - #'{\"args\":[\"\"],\"call\":\"r\"}' + 1)\n\
         local none, why = h:call({ call = 'r', args = { body } })\n\
         local after = h:call({ call = 'verbs' })\n\
         balthasar.answer = tostring(none) .. '|' .. why .. '|' .. after.result[1]",
    );
    let parts: Vec<&str> = said.split('|').collect();
    assert_eq!(parts[0], "nil", "{said}");
    assert!(parts[1].contains("16777217 bytes"), "{said}");
    assert!(parts[1].contains("not sent"), "{said}");
    assert_eq!(
        parts[2], "verbs",
        "nothing of the big call reached the peer"
    );
    drop(served);
}

#[test]
fn a_reply_cut_short_closes_the_handle() {
    let (path, served) = peer("short", 0, |socket| {
        let _ = socket.write_all(&100_u32.to_be_bytes());
        let _ = socket.write_all(b"{\"ok\":");
    });
    let said = through(
        &path,
        "local none, why = h:call({ call = 'verbs' })\n\
         local _, after = h:call({ call = 'verbs' })\n\
         balthasar.answer = tostring(none) .. '|' .. why .. '|' .. after",
    );
    let parts: Vec<&str> = said.split('|').collect();
    assert_eq!(parts[0], "nil", "{said}");
    assert!(parts[1].contains("closed after 6 bytes"), "{said}");
    assert_eq!(parts[2], "this connection is closed");
    let _ = served.join();
}

#[test]
fn a_reply_over_the_cap_is_refused_before_its_body_is_read() {
    let (path, served) = peer("big", 0, |socket| {
        let _ = socket.write_all(&(16_u32 * 1024 * 1024 + 1).to_be_bytes());
    });
    let said = through(
        &path,
        "local none, why = h:call({ call = 'verbs' })\n\
         local _, after = h:call({ call = 'verbs' })\n\
         balthasar.answer = tostring(none) .. '|' .. why .. '|' .. after",
    );
    let parts: Vec<&str> = said.split('|').collect();
    assert_eq!(parts[0], "nil", "{said}");
    assert!(parts[1].contains("16777217 bytes"), "{said}");
    assert_eq!(parts[2], "this connection is closed");
    let _ = served.join();
}

#[test]
fn a_peer_that_never_answers_is_given_up_on_at_the_deadline() {
    let (path, served) = peer("quiet", 0, |socket| {
        let _ = socket.set_read_timeout(Some(std::time::Duration::from_secs(3)));
        let _ = socket.read(&mut [0_u8; 1]);
    });
    let began = std::time::Instant::now();
    let said = through(
        &path,
        "local none, why = h:call({ call = 'verbs' }, { deadline_ms = 200 })\n\
         balthasar.answer = tostring(none) .. '|' .. why",
    );
    assert!(said.starts_with("nil|"), "{said}");
    assert!(
        began.elapsed() < std::time::Duration::from_secs(2),
        "it waited past its deadline"
    );
    let _ = served.join();
}
