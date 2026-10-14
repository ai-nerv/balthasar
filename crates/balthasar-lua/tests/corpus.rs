//! The family's wire corpus, against the client library this program serves: each client case is
//! what a fake server answers the client's second call with, and the client must take it as
//! `cases.json` says.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_lua::{CLIENT, Engine};
use std::io::{Read, Write};
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/wire-corpus")
}

/// A socket in the memory role's fallback directory, which a config's stream may dial.
fn socket(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("memory-{}", balthasar_model::runtime::uid()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir.join(format!("corpus-{name}-{}.sock", std::process::id()))
}

/// Serve one connection: the handshake answered properly, then `case` as the next reply.
fn serve_case(path: &PathBuf, case: Vec<u8>, eof_after: bool) -> std::thread::JoinHandle<()> {
    let _ = std::fs::remove_file(path);
    let listener = std::os::unix::net::UnixListener::bind(path).expect("bind");
    std::thread::spawn(move || {
        let Ok((mut socket, _)) = listener.accept() else {
            return;
        };
        let read_frame = |socket: &mut std::os::unix::net::UnixStream| -> bool {
            let mut head = [0_u8; 4];
            if socket.read_exact(&mut head).is_err() {
                return false;
            }
            let mut body = vec![0_u8; u32::from_be_bytes(head) as usize];
            socket.read_exact(&mut body).is_ok()
        };
        if !read_frame(&mut socket) {
            return;
        }
        let ok = br#"{"ok":true,"family":1,"n":1,"result":["verbs"]}"#;
        let mut framed = (ok.len() as u32).to_be_bytes().to_vec();
        framed.extend_from_slice(ok);
        let _ = socket.write_all(&framed);
        if !read_frame(&mut socket) {
            return;
        }
        let _ = socket.write_all(&case);
        if eof_after {
            return;
        }
        // Held open until the client lets go: what is under test is the client giving up.
        let _ = socket.set_read_timeout(Some(std::time::Duration::from_secs(3)));
        let _ = socket.read(&mut [0_u8; 1]);
    })
}

/// What the client made of `case`: the call's three values, then what a following call said.
fn client_took(name: &str, case: Vec<u8>, eof_after: bool) -> String {
    let path = socket(name);
    let server = serve_case(&path, case, eof_after);
    let mut engine = Engine::new();
    let probe = format!(
        "local client = assert(load({CLIENT:?}))(balthasar.stream) \
         local s = assert(client.connect({{ path = {path:?}, timeout_ms = 1000 }})) \
         local a, b, c = s:call('recall', 'x') \
         local _, after = s:call('verbs') \
         balthasar.answer = tostring(a) .. '|' .. tostring(b) .. '|' .. tostring(c) \
           .. '|' .. tostring(after)",
        path = path.to_string_lossy()
    );
    engine
        .run(&probe, "corpus.lua")
        .expect("the client must not raise");
    engine.harvest();
    let _ = server.join();
    let _ = std::fs::remove_file(&path);
    engine
        .config()
        .string("answer")
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn every_client_case_of_the_corpus_is_taken_as_it_says() {
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(corpus().join("cases.json")).expect("cases"))
            .expect("parse");
    for case in cases["client"].as_array().expect("client cases") {
        let file = case["file"].as_str().expect("file");
        let bytes = std::fs::read(corpus().join(file)).expect("case");
        let eof_after = case["eof_after"].as_bool().unwrap_or(false);
        let name = file.replace(['/', '.'], "-");
        let took = client_took(&name, bytes, eof_after);
        let parts: Vec<&str> = took.split('|').collect();
        match case["expect"].as_str().expect("expect") {
            "accepted" => assert_eq!(parts[0], "x", "{file}: {took}"),
            "failed" => assert_eq!(parts[2], "failed", "{file}: {took}"),
            "refused" => assert_eq!(parts[2], "refused", "{file}: {took}"),
            "error_and_close" | "error_and_close_without_allocating" => {
                assert_eq!(parts[0], "nil", "{file}: {took}");
                assert_eq!(parts[3], "this connection is closed", "{file}: {took}");
            }
            other => panic!("{file}: no such expectation '{other}'"),
        }
    }
}
