//! `--tied`: whether the kernel takes this balthasar with the process that started it.
//!
//! Against the real binary, because `PR_SET_PDEATHSIG` is a property of a live process and its
//! parent. Every test here kills the parent outright rather than asking it to stop, because
//! nothing in a caller runs when it is killed.

use balthasar_model::scratch::Scratch;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// How long to wait for a process to notice its parent is gone.
const NOTICES_WITHIN: Duration = Duration::from_secs(10);

/// A parent that starts one balthasar and then does nothing at all.
///
/// A shell rather than this process, because the parent has to be something the test can kill
/// outright. `$!` is the pid of the balthasar itself, which is what has to be watched.
struct Caller {
    shell: Child,
    /// Where the shell wrote the pid of the balthasar it started.
    pids: std::path::PathBuf,
}

impl Caller {
    /// Start a shell that starts `balthasar serve`, with whatever extra arguments are given.
    fn starting(dir: &Path, instance: &str, extra: &str) -> Self {
        let pids = dir.join("pid");
        let script = format!(
            "{binary} serve --instance {instance} --scope project {extra} \
             >/dev/null 2>&1 & echo $! > {pids}; wait",
            binary = env!("CARGO_BIN_EXE_balthasar"),
            pids = pids.display(),
        );
        let shell = Command::new("sh")
            .arg("-c")
            .arg(script)
            .current_dir(dir)
            .env("XDG_RUNTIME_DIR", dir.join("run"))
            .env("XDG_DATA_HOME", dir.join("data"))
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .spawn()
            .expect("start the caller");
        Self { shell, pids }
    }

    /// The balthasar this caller started, once it has told us which one it is.
    fn served(&self) -> u32 {
        let deadline = Instant::now() + NOTICES_WITHIN;
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(&self.pids)
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                return pid;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the caller never said which balthasar it started");
    }

    /// End the caller the way a crash would: with nothing running inside it.
    fn killed(&mut self) {
        let _ = self.shell.kill();
        let _ = self.shell.wait();
    }
}

/// Whether a process exists and is not merely a corpse waiting to be reaped.
///
/// The state field rather than the directory's existence: every process here is started by a
/// shell that is about to be killed, so zombies are the expected shape of "gone".
fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.split_whitespace().nth(2) != Some("Z"))
}

/// Wait for `pid` to go away, and say whether it did.
fn gone_within(pid: u32, patience: Duration) -> bool {
    let deadline = Instant::now() + patience;
    while Instant::now() < deadline {
        if !alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !alive(pid)
}

/// Leave nothing running, whatever the assertions did.
fn end(pid: u32) {
    let _ = Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        // Already gone is the passing case, and its complaint on stderr reads like a failure.
        .stderr(std::process::Stdio::null())
        .status();
}

#[test]
fn a_tied_balthasar_goes_when_its_caller_is_killed() {
    let dir = Scratch::new("balthasar-tied", "killed");
    std::fs::create_dir_all(dir.join("run")).expect("mkdir");
    let mut caller = Caller::starting(&dir, "tied-killed", "--tied $$");
    let served = caller.served();
    assert!(alive(served), "it started");

    caller.killed();

    let went = gone_within(served, NOTICES_WITHIN);
    end(served);
    assert!(
        went,
        "a tied balthasar must not outlive the process that started it"
    );
}

#[test]
fn an_untied_balthasar_stays_up() {
    // The flag is opt-in: a balthasar started at a terminal is meant to outlive the command.
    let dir = Scratch::new("balthasar-tied", "untied");
    std::fs::create_dir_all(dir.join("run")).expect("mkdir");
    let mut caller = Caller::starting(&dir, "untied", "");
    let served = caller.served();
    assert!(alive(served), "it started");

    caller.killed();

    // Long enough that a tie would have fired several times over.
    std::thread::sleep(Duration::from_secs(2));
    let outlived = alive(served);
    end(served);
    assert!(
        outlived,
        "without `--tied` a balthasar is nobody's child and stays up"
    );
}

#[test]
fn a_tie_asked_for_after_the_caller_is_already_gone_is_not_missed() {
    // `PR_SET_PDEATHSIG` watches from the moment it is set, so a caller that died between the
    // spawn and that call is a death no signal was ever sent for.
    let dir = Scratch::new("balthasar-tied", "raced");
    std::fs::create_dir_all(dir.join("run")).expect("mkdir");

    // A caller that is gone before its balthasar has finished starting.
    let pids = dir.join("pid");
    let script = format!(
        "{binary} serve --instance raced --scope project --tied $$ >/dev/null 2>&1 & \
         echo $! > {pids}",
        binary = env!("CARGO_BIN_EXE_balthasar"),
        pids = pids.display(),
    );
    let mut shell = Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(&*dir)
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .spawn()
        .expect("start the caller");
    let _ = shell.wait();

    let served = std::fs::read_to_string(&pids)
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok())
        .expect("the caller said which balthasar it started");

    let went = gone_within(served, NOTICES_WITHIN);
    end(served);
    assert!(
        went,
        "a balthasar whose caller was already gone must not serve on regardless"
    );
}
