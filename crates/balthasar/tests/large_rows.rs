//! Rows near the frame cap, observed and replayed over the real `serve` socket: one that fits a
//! reply comes back whole, and one that does not comes back cut with its raw record read after it
//! a piece at a time.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::scratch::Scratch;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

const MIB: usize = 1024 * 1024;

/// A `serve` of this test's own, ended when the test ends.
struct Serving {
    child: Child,
    socket: PathBuf,
    _dir: Scratch,
}

impl Serving {
    fn start(name: &str) -> Self {
        let dir = Scratch::new("blr", name);
        let run = dir.join("run");
        std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
            .create(&run)
            .expect("mkdir");
        let child = Command::new(env!("CARGO_BIN_EXE_balthasar"))
            .args(["serve", "--instance", "lr", "--scope", "project"])
            .current_dir(&*dir)
            .env("XDG_RUNTIME_DIR", &run)
            .env("XDG_DATA_HOME", dir.join("data"))
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("serve");
        let socket = run.join("memory/api@lr.sock");
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

    /// One call on a connection of its own, and the reply decoded.
    fn ask(&self, verb: &str, args: serde_json::Value) -> serde_json::Value {
        let body =
            serde_json::to_vec(&serde_json::json!({ "call": verb, "args": args })).expect("encode");
        assert!(
            body.len() <= balthasar_ipc::MAX_FRAME,
            "{verb} is over the cap"
        );
        let mut stream = UnixStream::connect(&self.socket).expect("connect");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(60)))
            .expect("a deadline");
        let length = u32::try_from(body.len()).expect("fits");
        stream.write_all(&length.to_be_bytes()).expect("length");
        stream.write_all(&body).expect("body");
        let mut header = [0_u8; 4];
        stream.read_exact(&mut header).expect("reply length");
        let length = u32::from_be_bytes(header) as usize;
        assert!(
            length <= balthasar_ipc::MAX_FRAME,
            "a reply of {length} bytes"
        );
        let mut reply = vec![0_u8; length];
        stream.read_exact(&mut reply).expect("reply body");
        serde_json::from_slice(&reply).expect("decode")
    }

    /// Every row of one replay page from `from`.
    fn page(&self, from: u64) -> Vec<serde_json::Value> {
        let reply = self.ask(
            "replay",
            serde_json::json!(["s", { "from": from, "bytes": 4 * MIB }]),
        );
        assert_eq!(reply["ok"], true, "{}", reply["error"]);
        reply["result"].as_array().expect("rows").clone()
    }

    /// The raw record at `cursor`, read back a piece at a time.
    fn raw_of(&self, cursor: u64) -> String {
        let mut whole = String::new();
        let mut offset = Some(0_u64);
        while let Some(at) = offset {
            let reply = self.ask(
                "replay",
                serde_json::json!(["s", { "from": cursor, "raw": at, "bytes": 4 * MIB }]),
            );
            assert_eq!(reply["ok"], true, "{}", reply["error"]);
            let piece = &reply["result"][0];
            assert_eq!(piece["offset"], at);
            whole.push_str(piece["raw"].as_str().expect("a piece"));
            offset = piece["next"].as_u64();
        }
        whole
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// An observed turn whose text is `text` and whose raw record carries it again, as a harness's does.
fn observed(serving: &Serving, cursor: u64, text: &str) -> serde_json::Value {
    let raw = serde_json::json!({ "entry": { "text": text } });
    let turn = serde_json::json!({
        "cursor": cursor, "role": "tool", "kind": "tool_result", "text": text, "raw": raw,
    });
    let said = serving.ask("observe", serde_json::json!(["s", turn]));
    assert_eq!(said["ok"], true, "{}", said["error"]);
    raw
}

#[test]
fn a_nine_mebibyte_row_comes_back_whole() {
    let serving = Serving::start("nine");
    let raw = observed(&serving, 1, &"a".repeat(9 * MIB / 2));
    observed(&serving, 2, "after");

    let first = serving.page(0);
    assert_eq!(first.len(), 1, "a row over the budget is a page of its own");
    assert!(first[0].get("truncated").is_none(), "it fits a frame");
    assert_eq!(first[0]["raw"], raw.to_string());
    assert_eq!(serving.page(2)[0]["text"], "after");
}

#[test]
fn a_fifteen_mebibyte_row_is_cut_and_its_raw_record_read_after_it() {
    let serving = Serving::start("fifteen");
    let text = "said \"this\"\n".repeat(15 * MIB / 30);
    let raw = observed(&serving, 1, &text);
    observed(&serving, 2, "after");

    let cut = serving.page(1);
    assert_eq!(cut.len(), 1);
    assert_eq!(
        cut[0]["truncated"], true,
        "escaped twice, it is over a frame"
    );
    assert!(cut[0].get("raw").is_none());
    let stored = raw.to_string();
    assert_eq!(cut[0]["raw_bytes"], stored.len());
    assert!(text.starts_with(cut[0]["text"].as_str().expect("text")));
    assert_eq!(
        serving.raw_of(1),
        stored,
        "every byte of the record comes back"
    );
    assert_eq!(
        serving.page(2)[0]["text"],
        "after",
        "and the stream goes on"
    );

    let none = serving.ask("replay", serde_json::json!(["s", { "from": 7, "raw": 0 }]));
    assert_eq!(
        none["ok"], false,
        "a turn that is not there has no raw record"
    );
}
