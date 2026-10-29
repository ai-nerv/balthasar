#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::scratch::Scratch;
use serde_json::{Value, json};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const KEY: &str = "5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e";

fn command(root: &Path) -> Command {
    let binary = std::env::var_os("BALTHASAR_TEST_BINARY")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_balthasar").into());
    let mut command = Command::new(binary);
    command
        .arg("--store")
        .arg(root.join("store.db"))
        .args(["--scope", "project"])
        .current_dir(root)
        .env("HOME", root)
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("NERV_GIVEN", root.join("given"));
    command
}

struct Process(Child);

impl Process {
    fn wait(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.0.try_wait().expect("wait") {
                return status;
            }
            assert!(Instant::now() < deadline, "process did not terminate");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Server {
    process: Process,
    socket: PathBuf,
}

impl Server {
    fn start(root: &Path, name: &str) -> Self {
        let run = root.join(name);
        balthasar_model::runtime::private_dir(&run).expect("private runtime");
        let mut cmd = command(root);
        cmd.args(["serve", "--instance", "w"])
            .env("XDG_RUNTIME_DIR", &run)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let selected = std::fs::canonicalize(cmd.get_program()).expect("selected binary");
        let mut process = Process(cmd.spawn().expect("server"));
        process
            .0
            .stdin
            .take()
            .expect("key input")
            .write_all(format!("{KEY}\n").as_bytes())
            .expect("key");
        let socket = run.join("memory/api@w.sock");
        let deadline = Instant::now() + Duration::from_secs(10);
        while UnixStream::connect(&socket).is_err() {
            assert!(
                process.0.try_wait().expect("status").is_none(),
                "server exited before binding"
            );
            assert!(Instant::now() < deadline, "server did not bind");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            std::fs::read_link(format!("/proc/{}/exe", process.0.id())).expect("actual executable"),
            selected
        );
        Self { process, socket }
    }

    fn connect(&self) -> UnixStream {
        let stream = UnixStream::connect(&self.socket).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("read bound");
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .expect("write bound");
        stream
    }

    fn stop(&mut self, crash: bool) {
        if crash {
            self.process.0.kill().expect("kill");
        } else {
            let status = Command::new("kill")
                .args(["-TERM", &self.process.0.id().to_string()])
                .status()
                .expect("terminate");
            assert!(status.success());
        }
        let status = self.process.wait();
        assert_eq!(status.success(), !crash);
    }
}

fn ask(stream: &mut UnixStream, keyed: bool, call: &str, args: Value) -> Value {
    let mut request = json!({ "call": call, "args": args });
    if keyed {
        request["key"] = json!(KEY);
    }
    balthasar_ipc::send(stream, &serde_json::to_vec(&request).expect("encode")).expect("send");
    let body = balthasar_ipc::recv_within(stream, Some(Duration::from_secs(3))).expect("reply");
    serde_json::from_slice(&body).expect("decode")
}

fn writer(stream: &mut UnixStream, operation: &str) -> Value {
    ask(
        stream,
        true,
        "writer",
        json!(["run", {"revision":1,"operation":operation}]),
    )
}

fn acquire_after_disconnect(stream: &mut UnixStream) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let reply = writer(stream, "reserve");
        if reply["ok"] == true {
            return;
        }
        assert!(
            reply["error"]
                .as_str()
                .expect("reason")
                .contains("already has a writer"),
            "{reply}"
        );
        assert!(
            Instant::now() < deadline,
            "reservation was not released: {reply}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn native_connections_require_authentication_and_release_only_their_own_writer() {
    let root = Scratch::new("bw", "connections");
    let server = Server::start(&root, "one");
    let (mut a, mut b) = (server.connect(), server.connect());
    let refused = ask(
        &mut a,
        false,
        "writer",
        json!(["run", {"revision":1,"operation":"reserve"}]),
    );
    assert_eq!(refused["error"], balthasar_ipc::KEY_REFUSED);
    assert_eq!(writer(&mut a, "reserve")["ok"], true);
    assert_eq!(writer(&mut b, "reserve")["ok"], false);
    assert_eq!(writer(&mut b, "release")["ok"], false);
    let turn = json!(["run", {"cursor":0,"role":"user","text":"owned"}]);
    let blocked = ask(&mut b, true, "observe", turn.clone());
    assert_eq!(blocked["ok"], false);
    assert_eq!(blocked["fault"], "failed");
    assert_eq!(ask(&mut a, true, "observe", turn)["ok"], true);
    drop(b);
    let mut c = server.connect();
    assert_eq!(writer(&mut c, "reserve")["ok"], false);
    drop(a);
    acquire_after_disconnect(&mut c);
    let replay = ask(&mut c, false, "replay", json!(["run"]));
    assert_eq!(replay["result"][0]["text"], "owned");
    assert_eq!(writer(&mut c, "release")["ok"], true);
}

#[test]
fn independent_native_daemons_share_the_store_lock_and_release_on_exit_or_crash() {
    for crash in [false, true] {
        let root = Scratch::new("bw", "daemons");
        let mut one = Server::start(&root, "one");
        let two = Server::start(&root, "two");
        let (mut a, mut b) = (one.connect(), two.connect());
        assert_eq!(writer(&mut a, "reserve")["ok"], true);
        assert_eq!(writer(&mut b, "reserve")["ok"], false);
        assert_eq!(
            ask(
                &mut b,
                true,
                "observe",
                json!(["run", {"cursor":0,"text":"blocked"}])
            )["ok"],
            false
        );
        one.stop(crash);
        assert_eq!(writer(&mut b, "reserve")["ok"], true);
        assert_eq!(
            ask(
                &mut b,
                true,
                "observe",
                json!(["run", {"cursor":0,"text":"successor"}])
            )["ok"],
            true
        );
        assert_eq!(writer(&mut b, "release")["ok"], true);
    }
}

#[test]
fn one_shot_api_cannot_bypass_a_native_reservation() {
    let root = Scratch::new("bw", "oneshot");
    let server = Server::start(&root, "one");
    let mut owner = server.connect();
    assert_eq!(writer(&mut owner, "reserve")["ok"], true);
    let mut cmd = command(&root);
    cmd.args([
        "api",
        "observe",
        "run",
        r#"{"cursor":0,"role":"user","text":"bypass"}"#,
        "--json",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    let mut child = Process(cmd.spawn().expect("one shot"));
    assert!(child.wait().success());
    let reply: Value = serde_json::from_reader(child.0.stdout.take().expect("reply output"))
        .expect("one shot reply");
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(reply["error"].as_str().expect("reason").contains("writer"));
    assert_eq!(ask(&mut owner, true, "replay", json!(["run"]))["n"], 0);
}
