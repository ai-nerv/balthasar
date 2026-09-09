//! What a serving balthasar leaves behind when it is told to stop.
//!
//! Against the real binary, for the reason `tied.rs` gives about its own subject: a signal is a
//! property of a live process, and a unit test could only assert that a function calls a
//! function. What has to be true is that the socket file is *gone* afterwards — the file itself,
//! on disk, looked at by something that is not the process that made it.
//!
//! `Drop` does not run on a signal, and the tie sends one: `PR_SET_PDEATHSIG` is `SIGTERM`, so
//! every tied balthasar used to leave its socket in the runtime directory. Hundreds accumulated
//! there. A forked child's probe meeting one of them reads a stale file as a live daemon, and
//! that is the failure the accept loop was rewritten for — so a corpse here is not untidiness,
//! it is the input to a worse bug.
//!
//! The third test is the one that matters most and is the hardest to fake: the signal arrives
//! from the kernel because a parent died, not from a test that decided to send one.

use balthasar_model::scratch::Scratch;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// How long to wait for a process to start, or to notice it has been told to go.
const WITHIN: Duration = Duration::from_secs(10);

/// A `balthasar serve` of our own, and where it said it would listen.
struct Serving {
    child: Child,
    socket: PathBuf,
}

impl Serving {
    /// Start one and wait until its socket is really there.
    ///
    /// Spawned directly rather than through a shell, and that is load-bearing for the `SIGINT`
    /// case: a shell without job control sets `SIGINT` to `SIG_IGN` in every background job it
    /// starts, and the disposition survives `exec`. A test that went through `sh -c … &` would
    /// send a signal to a process that had been told to ignore it, watch nothing happen, and
    /// report a corpse that the fix cannot prevent because the signal never arrived.
    fn starting(dir: &Path, instance: &str) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_balthasar"))
            .args(["serve", "--instance", instance, "--scope", "project"])
            .current_dir(dir)
            .env("XDG_RUNTIME_DIR", dir.join("run"))
            .env("XDG_DATA_HOME", dir.join("data"))
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("start a balthasar");
        let socket = dir
            .join("run")
            .join("balthasar")
            .join(format!("api@{instance}.sock"));
        let listening = Self { child, socket };
        assert!(
            waited_for(|| listening.socket.exists()),
            "it never bound {}",
            listening.socket.display()
        );
        listening
    }
}

impl Drop for Serving {
    /// Leave nothing running, whatever the assertions did.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Wait until `ready` says so, and report whether it ever did.
fn waited_for(ready: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + WITHIN;
    while Instant::now() < deadline {
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    ready()
}

/// Whether a process exists and is not merely a corpse waiting to be reaped.
///
/// The state field rather than the directory's existence, as in `tied.rs`: the caller below is
/// killed, so the balthasar under it is reparented and may sit as a zombie with a `/proc` entry
/// of its own for a moment.
fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.split_whitespace().nth(2) != Some("Z"))
}

/// Send a signal by name, the way anything outside this process would have to.
fn signal(pid: u32, named: &str) {
    let sent = Command::new("kill")
        .arg(format!("-{named}"))
        .arg(pid.to_string())
        .status()
        .expect("send a signal");
    assert!(sent.success(), "{named} was not delivered to {pid}");
}

/// Both signals, one body: what is being asserted is identical and the difference is the name.
fn ends_cleanly(named: &str, instance: &str) {
    let dir = Scratch::new("balthasar-signalled", instance);
    std::fs::create_dir_all(dir.join("run")).expect("mkdir");
    let mut serving = Serving::starting(&dir, instance);
    let socket = serving.socket.clone();
    assert!(socket.exists(), "before: {}", socket.display());

    signal(serving.child.id(), named);

    let ended = serving.child.wait().expect("it ended");
    assert!(
        waited_for(|| !socket.exists()),
        "{named} left {} behind",
        socket.display()
    );
    // Zero rather than 128+n. The signal is not what ended this process: it woke a thread that
    // was waiting for it, and the process then returned from `main` the way it does when it runs
    // out of anything to serve. A stop asked for is not a failure.
    assert_eq!(ended.code(), Some(0), "{named} should end this cleanly");
}

#[test]
fn a_balthasar_told_to_terminate_takes_its_socket_with_it() {
    ends_cleanly("TERM", "term");
}

#[test]
fn a_balthasar_interrupted_takes_its_socket_with_it() {
    // Ctrl-C at a terminal. The same intention as `SIGTERM` arriving by another road, and it
    // should not leave a different amount of mess behind.
    ends_cleanly("INT", "int");
}

#[test]
fn a_tied_balthasar_leaves_no_socket_when_its_caller_is_killed() {
    // The case the whole thing exists for, and the only one where the signal comes from the
    // kernel rather than from this test: `--tied` asks for `SIGTERM` on the death of the process
    // that started us, the caller is killed outright so nothing in it runs, and what is left in
    // the runtime directory afterwards is the datum. `tied.rs` proves the process goes; this
    // proves it does not leave its name behind on the way.
    let dir = Scratch::new("balthasar-signalled", "tied");
    std::fs::create_dir_all(dir.join("run")).expect("mkdir");
    let pids = dir.join("pid");
    let script = format!(
        "{binary} serve --instance tied --scope project --tied $$ >/dev/null 2>&1 & \
         echo $! > {pids}; wait",
        binary = env!("CARGO_BIN_EXE_balthasar"),
        pids = pids.display(),
    );
    let mut caller = Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(&*dir)
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .spawn()
        .expect("start the caller");

    let socket = dir.join("run").join("balthasar").join("api@tied.sock");
    assert!(waited_for(|| socket.exists()), "it never bound");
    let served: u32 = std::fs::read_to_string(&pids)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .expect("the caller said which balthasar it started");

    // Asked of the pid the caller wrote down, before anything happens to it. A later version of
    // this that watched the shell, or a pid that never parsed, would pass on a process that was
    // never the subject.
    assert!(alive(served), "it was running before its caller was killed");

    // Checked, not attempted. This kill is the whole experiment: if it silently did nothing, the
    // socket would still be there afterwards and the test would report the bug it exists to
    // catch.
    caller.kill().expect("kill the caller outright");
    caller.wait().expect("reap the caller");

    let gone = waited_for(|| !socket.exists());
    // Both, and about the process the caller named rather than about any process at hand. A
    // balthasar that unlinked its socket and then carried on serving nothing would satisfy the
    // first of these on its own, and is not what the tie promises.
    let ended = waited_for(|| !alive(served));
    let _ = Command::new("kill")
        .args(["-9", &served.to_string()])
        // Already gone is the passing case, and its complaint reads like a failure.
        .stderr(std::process::Stdio::null())
        .status();
    assert!(
        gone,
        "a tied balthasar must not leave {} for the next one to disprove",
        socket.display()
    );
    assert!(ended, "and it must not still be running afterwards");
}
