//! What a serving balthasar leaves behind when it is told to stop.
//!
//! Against the real binary: `Drop` does not run on a signal, and what has to be true is that the
//! socket file is gone afterwards, looked at by something that is not the process that made it.

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
    /// Spawned directly, never through a shell: a shell without job control sets `SIGINT` to
    /// `SIG_IGN` in every background job, and the disposition survives `exec`.
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
/// The state field rather than the `/proc` entry: a reparented balthasar may sit as a zombie.
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

/// Both signals, one body: what is asserted is identical and the difference is the name.
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
    // Zero rather than 128+n: the signal woke a waiting thread and `main` returned normally.
    assert_eq!(ended.code(), Some(0), "{named} should end this cleanly");
}

#[test]
fn a_balthasar_told_to_terminate_takes_its_socket_with_it() {
    ends_cleanly("TERM", "term");
}

#[test]
fn a_balthasar_interrupted_takes_its_socket_with_it() {
    ends_cleanly("INT", "int");
}

#[test]
fn a_tied_balthasar_leaves_no_socket_when_its_caller_is_killed() {
    // The only case where the signal comes from the kernel rather than from this test.
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

    // The pid the caller wrote down: watching the shell would pass on the wrong process.
    assert!(alive(served), "it was running before its caller was killed");

    // Checked, not attempted: a kill that silently did nothing would report the bug as present.
    caller.kill().expect("kill the caller outright");
    caller.wait().expect("reap the caller");

    let gone = waited_for(|| !socket.exists());
    // Both: unlinking the socket and carrying on serving nothing is not what the tie promises.
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
