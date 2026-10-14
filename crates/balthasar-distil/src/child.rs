//! Running a child with a deadline: input written on a thread, output read on threads and capped,
//! and the whole process group killed and reaped when the deadline passes.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long output is waited for once the child itself has exited.
const GRACE: Duration = Duration::from_millis(250);

/// What an empty pipe always takes without blocking: one page.
const FRESH_PIPE: usize = 4096;

/// What a finished child left.
#[derive(Debug)]
pub struct Ran {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Whether the input went in whole.
    pub written: std::io::Result<()>,
}

/// Why a child gave no result.
#[derive(Debug, thiserror::Error)]
pub enum Stopped {
    #[error("{0}")]
    Spawn(std::io::Error),
    #[error("ran past its {0} ms deadline and was stopped")]
    Deadline(u64),
    #[error("{0}")]
    Wait(std::io::Error),
}

/// Run `argv` with `input` on its stdin, keeping at most `cap` bytes of its stdout.
///
/// # Errors
/// When it cannot start, cannot be waited on, or outlives `timeout`; in the last two cases it
/// has been killed with its process group and reaped.
pub fn run(argv: &[String], input: &[u8], timeout: Duration, cap: usize) -> Result<Ran, Stopped> {
    let Some((program, args)) = argv.split_first() else {
        return Err(Stopped::Spawn(std::io::Error::other("an empty command")));
    };
    let deadline = Instant::now() + timeout;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(Stopped::Spawn)?;
    let group = rustix::process::Pid::from_child(&child);

    let stdout = drain(child.stdout.take(), cap);
    let stderr = drain(child.stderr.take(), 4096);
    // What fits in a fresh pipe goes in now, before the child can have exited; the rest on a
    // thread, so a child that writes before it reads cannot wedge the two of them.
    let (wrote, written) = mpsc::channel();
    let mut stdin = child.stdin.take();
    let head = input.len().min(FRESH_PIPE);
    let first = stdin
        .as_mut()
        .map_or(Ok(()), |pipe| pipe.write_all(&input[..head]));
    let rest = input[head..].to_owned();
    std::thread::spawn(move || {
        let done = first.and_then(|()| stdin.map_or(Ok(()), |mut pipe| pipe.write_all(&rest)));
        let _ = wrote.send(done);
    });

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                stop(&mut child, group);
                return Err(Stopped::Deadline(
                    u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(why) => {
                stop(&mut child, group);
                return Err(Stopped::Wait(why));
            }
        }
    };

    // A grandchild can hold the pipes open after the child is gone: it is not waited for.
    let settle = Instant::now() + GRACE;
    let left = || settle.saturating_duration_since(Instant::now());
    let out = stdout.recv_timeout(left()).ok();
    let err = stderr.recv_timeout(left()).ok();
    if out.is_none() || err.is_none() {
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
    Ok(Ran {
        status,
        stdout: out
            .or_else(|| stdout.recv_timeout(GRACE).ok())
            .unwrap_or_default(),
        stderr: err
            .or_else(|| stderr.recv_timeout(GRACE).ok())
            .unwrap_or_default(),
        written: written
            .recv_timeout(GRACE)
            .unwrap_or_else(|_| Err(std::io::Error::other("the input was not taken"))),
    })
}

/// Kill the child's whole group, then reap the child.
fn stop(child: &mut std::process::Child, group: rustix::process::Pid) {
    let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    let _ = child.kill();
    let _ = child.wait();
}

/// Read a pipe to its end on a thread, keeping at most `cap` bytes and discarding the rest so
/// the writer never blocks on a full pipe.
fn drain(pipe: Option<impl Read + Send + 'static>, cap: usize) -> mpsc::Receiver<Vec<u8>> {
    let (sent, taken) = mpsc::channel();
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        if let Some(mut pipe) = pipe {
            let mut buffer = [0_u8; 8192];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let room = cap.saturating_sub(kept.len()).min(n);
                        kept.extend_from_slice(&buffer[..room]);
                    }
                    Err(why) if why.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        }
        let _ = sent.send(kept);
    });
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Vec<String> {
        vec!["sh".to_owned(), "-c".to_owned(), script.to_owned()]
    }

    #[test]
    fn a_child_that_writes_before_it_reads_does_not_deadlock() {
        let prompt = vec![b'p'; 1024 * 1024];
        let ran = run(
            &sh("head -c 1000000 /dev/zero | tr '\\0' a; cat > /dev/null"),
            &prompt,
            Duration::from_secs(20),
            64,
        )
        .expect("it finishes");
        assert!(ran.status.success());
        assert_eq!(ran.stdout.len(), 64, "and what is kept is capped");
        assert!(ran.written.is_ok());
    }

    #[test]
    fn a_child_past_its_deadline_is_killed_and_reaped() {
        let started = Instant::now();
        let stopped =
            run(&sh("sleep 1000"), b"", Duration::from_millis(200), 64).expect_err("stopped");
        assert!(matches!(stopped, Stopped::Deadline(200)), "{stopped}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_grandchild_holding_the_pipe_does_not_hold_the_answer() {
        let started = Instant::now();
        let ran = run(
            &sh("sleep 1000 & echo said"),
            b"",
            Duration::from_secs(20),
            64,
        )
        .expect("the child answered");
        assert_eq!(String::from_utf8_lossy(&ran.stdout).trim(), "said");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_child_that_never_reads_its_input_is_still_waited_out() {
        let ran = run(
            &sh("exec 0<&-; sleep 0.1; echo done"),
            &vec![b'x'; 1024 * 1024],
            Duration::from_secs(20),
            64,
        )
        .expect("finishes");
        assert_eq!(String::from_utf8_lossy(&ran.stdout).trim(), "done");
        assert!(ran.written.is_err(), "the input was not all taken");
    }
}
