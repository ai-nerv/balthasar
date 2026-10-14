//! A `serve` of a test's own, told its key on standard input as a coordinator tells it, and ended
//! when the test ends.

use balthasar_model::scratch::Scratch;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The key every `serve` here is started with.
pub const KEY: &str = "5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e5e7e";

pub struct Serving {
    child: Child,
    pub socket: PathBuf,
    _dir: Scratch,
}

impl Serving {
    /// Started with [`KEY`], and bound.
    pub fn start(name: &str) -> Self {
        Self::fed(name, &format!("{KEY}\n"), &[]).unwrap_or_else(|said| panic!("serve: {said}"))
    }

    /// Started with `stdin` as everything it is told there and `env` beside the test's own
    /// directories; what it said on stderr when it exited without binding.
    pub fn fed(name: &str, stdin: &str, env: &[(&str, &str)]) -> Result<Self, String> {
        let dir = Scratch::new("bsk", name);
        let run = dir.join("run");
        std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
            .create(&run)
            .expect("mkdir");
        let mut child = Command::new(env!("CARGO_BIN_EXE_balthasar"))
            .args(["serve", "--instance", "k", "--scope", "project"])
            .current_dir(&*dir)
            .env("XDG_RUNTIME_DIR", &run)
            .env("XDG_DATA_HOME", dir.join("data"))
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .envs(env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("serve");
        let mut told = child.stdin.take().expect("serve's stdin");
        told.write_all(stdin.as_bytes()).expect("stdin");
        drop(told);
        let socket = run.join("memory/api@k.sock");
        let deadline = Instant::now() + Duration::from_secs(10);
        while UnixStream::connect(&socket).is_err() {
            if child.try_wait().expect("status").is_some() {
                let mut said = String::new();
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut said);
                }
                return Err(said);
            }
            assert!(Instant::now() < deadline, "serve did not bind");
            std::thread::sleep(Duration::from_millis(20));
        }
        if let Some(mut err) = child.stderr.take() {
            std::thread::spawn(move || std::io::copy(&mut err, &mut std::io::sink()));
        }
        Ok(Self {
            child,
            socket,
            _dir: dir,
        })
    }

    /// One call with [`KEY`], on a connection of its own.
    pub fn ask(&self, verb: &str, args: serde_json::Value) -> serde_json::Value {
        self.ask_with(Some(KEY), verb, args)
    }

    /// One call carrying `key`, or none, on a connection of its own, and the reply decoded.
    pub fn ask_with(
        &self,
        key: Option<&str>,
        verb: &str,
        args: serde_json::Value,
    ) -> serde_json::Value {
        let mut call = serde_json::json!({ "call": verb, "args": args });
        if let Some(key) = key {
            call["key"] = serde_json::json!(key);
        }
        let body = serde_json::to_vec(&call).expect("encode");
        assert!(
            body.len() <= balthasar_ipc::MAX_FRAME,
            "{verb} is over the cap"
        );
        let mut stream = UnixStream::connect(&self.socket).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .expect("a deadline");
        let length = u32::try_from(body.len()).expect("fits");
        stream.write_all(&length.to_be_bytes()).expect("length");
        stream.write_all(&body).expect("body");
        let mut header = [0_u8; 4];
        stream
            .read_exact(&mut header)
            .expect("a reply, not a close");
        let length = u32::from_be_bytes(header) as usize;
        assert!(
            length <= balthasar_ipc::MAX_FRAME,
            "a reply of {length} bytes"
        );
        let mut reply = vec![0_u8; length];
        stream.read_exact(&mut reply).expect("reply body");
        serde_json::from_slice(&reply).expect("decode")
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
