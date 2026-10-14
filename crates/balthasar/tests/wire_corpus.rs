//! The family's wire corpus, against the real `serve`: every case answered as `cases.json` says,
//! and the connection left open or closed as it says.
//!
//! A write on the socket is answered only with the key `serve` was started with. So a case whose
//! body decodes as a call is re-framed in its own encoding with the key added; every other case
//! goes as it is.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::scratch::Scratch;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

const KEY: &str = "c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/wire-corpus")
}

/// `bytes` as this socket takes it: a whole frame whose body is a call gets the key, in the
/// body's own encoding. Anything else is returned as it is.
fn keyed(bytes: &[u8]) -> Vec<u8> {
    let Some((head, body)) = bytes.split_first_chunk::<4>() else {
        return bytes.to_vec();
    };
    if body.is_empty() || u32::from_be_bytes(*head) as usize != body.len() {
        return bytes.to_vec();
    }
    let wire = balthasar_ipc::Wire::of(body);
    let Ok(serde_json::Value::Object(mut call)) = wire.read::<serde_json::Value>(body) else {
        return bytes.to_vec();
    };
    call.insert("key".into(), KEY.into());
    let body = wire.write(&call).expect("encode");
    let mut framed = u32::try_from(body.len())
        .expect("fits")
        .to_be_bytes()
        .to_vec();
    framed.extend_from_slice(&body);
    framed
}

/// A `serve` of this test's own, ended when the test ends.
struct Serving {
    child: Child,
    socket: PathBuf,
    _dir: Scratch,
}

impl Serving {
    fn start() -> Self {
        let dir = Scratch::new("bwc", "s");
        let run = dir.join("run");
        std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
            .create(&run)
            .expect("mkdir");
        let instance = "wc".to_owned();
        let mut child = Command::new(env!("CARGO_BIN_EXE_balthasar"))
            .args(["serve", "--instance", &instance, "--scope", "project"])
            .current_dir(&*dir)
            .env("XDG_RUNTIME_DIR", &run)
            .env("XDG_DATA_HOME", dir.join("data"))
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("serve");
        let mut stdin = child.stdin.take().expect("serve's stdin");
        stdin
            .write_all(format!("{KEY}\n").as_bytes())
            .expect("the key");
        drop(stdin);
        let socket = run.join("memory").join(format!("api@{instance}.sock"));
        for _ in 0..500 {
            if UnixStream::connect(&socket).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Self {
            child,
            socket,
            _dir: dir,
        }
    }

    fn connect(&self) -> UnixStream {
        let stream = UnixStream::connect(&self.socket).expect("connect");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .expect("a deadline");
        stream
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One reply, or `None` when the server closed instead.
fn reply(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut header = [0_u8; 4];
    stream.read_exact(&mut header).ok()?;
    let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut body).ok()?;
    Some(body)
}

fn decoded(body: &[u8]) -> (serde_json::Value, &'static str) {
    let wire = balthasar_ipc::Wire::of(body);
    let value = wire
        .read(body)
        .expect("a reply in one of the two encodings");
    let encoding = match wire {
        balthasar_ipc::Wire::Cbor => "cbor",
        balthasar_ipc::Wire::Json => "json",
    };
    (value, encoding)
}

fn send(stream: &mut UnixStream, bytes: &[u8], bytewise: bool) {
    if bytewise {
        for byte in bytes {
            stream.write_all(&[*byte]).expect("write");
            stream.flush().expect("flush");
        }
    } else {
        stream.write_all(bytes).expect("write");
    }
}

#[test]
fn every_server_case_of_the_corpus_is_answered_as_it_says() {
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(corpus().join("cases.json")).expect("cases"))
            .expect("parse");
    let verbs = keyed(&std::fs::read(corpus().join("server/verbs-json.frame")).expect("verbs"));
    let serving = Serving::start();
    for case in cases["server"].as_array().expect("server cases") {
        let file = case["file"].as_str().expect("file");
        let bytes = keyed(&std::fs::read(corpus().join(file)).expect("case"));
        let mut stream = serving.connect();
        send(&mut stream, &bytes, case["split"] == "bytewise");
        let body = reply(&mut stream).unwrap_or_else(|| panic!("{file}: no answer"));
        let (said, encoding) = decoded(&body);
        let expect = &case["expect"];
        assert_eq!(said["ok"], expect["ok"], "{file}: {said}");
        if let Some(wanted) = expect["encoding"].as_str() {
            assert_eq!(encoding, wanted, "{file}");
        }
        if let Some(fault) = expect["fault"].as_str() {
            assert_eq!(said["fault"], fault, "{file}: {said}");
        }
        if let Some(part) = expect["error_contains"].as_str() {
            let error = said["error"].as_str().unwrap_or_default();
            assert!(error.contains(part), "{file}: {said}");
        }
        match case["then"].as_str() {
            Some("open") => {
                send(&mut stream, &verbs, false);
                let next = reply(&mut stream).unwrap_or_else(|| panic!("{file}: closed"));
                assert_eq!(decoded(&next).0["ok"], true, "{file}: out of step after");
            }
            Some("closed") => assert!(reply(&mut stream).is_none(), "{file}: left open"),
            _ => {}
        }
    }
}

#[test]
fn a_corpus_read_as_it_is_needs_no_key() {
    let serving = Serving::start();
    for file in ["server/verbs-json.frame", "server/verbs-cbor.frame"] {
        let bytes = std::fs::read(corpus().join(file)).expect("case");
        let mut stream = serving.connect();
        send(&mut stream, &bytes, false);
        let (said, _) = decoded(&reply(&mut stream).expect("a reply, not a close"));
        assert_eq!(said["ok"], true, "{file}: {said}");
    }
}

#[test]
fn the_corpus_names_the_cap_this_program_reads_at() {
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(corpus().join("cases.json")).expect("cases"))
            .expect("parse");
    assert_eq!(
        cases["cap"].as_u64(),
        u64::try_from(balthasar_ipc::MAX_FRAME).ok()
    );
}
