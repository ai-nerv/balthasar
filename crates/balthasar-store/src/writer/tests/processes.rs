use super::*;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Child(std::process::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn until<T>(mut ready: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(value) = ready() {
            return value;
        }
        assert!(Instant::now() < deadline, "writer child deadline");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn child_holds_writer_until_input_or_process_exit() {
    let Some(root) = std::env::var_os("BALTHASAR_WRITER_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let store = Transcript::open(&root.join("transcript.db")).expect("child store");
    let held = store
        .reserve_writer(&key())
        .expect("child reserve")
        .expect("free");
    held.check().expect("child identity");
    std::fs::write(root.join("ready"), b"held").expect("barrier");
    std::io::stdin().read_exact(&mut [0]).expect("release");
    held.check().expect("held until release");
}

#[test]
fn independent_processes_exclude_until_release_or_crash() {
    for crash in [false, true] {
        let root = Scratch::new("writer", if crash { "crash" } else { "release" });
        let store = Transcript::open(&root.join("transcript.db")).expect("parent store");
        let mut child = Child(
            Command::new(std::env::current_exe().expect("test executable"))
                .args([
                    "--exact",
                    "writer::tests::processes::child_holds_writer_until_input_or_process_exit",
                ])
                .env("BALTHASAR_WRITER_TEST_ROOT", &*root)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .expect("child"),
        );
        until(|| {
            assert!(
                child.0.try_wait().expect("child status").is_none(),
                "child exited before barrier"
            );
            root.join("ready").exists().then_some(())
        });
        assert!(store.reserve_writer(&key()).expect("busy").is_none());
        if crash {
            child.0.kill().expect("kill holder");
        } else {
            child
                .0
                .stdin
                .take()
                .expect("input")
                .write_all(b"x")
                .expect("release holder");
        }
        let status = until(|| child.0.try_wait().expect("wait child"));
        assert_eq!(status.success(), !crash);
        let held = store
            .reserve_writer(&key())
            .expect("released")
            .expect("free after exit");
        held.check().expect("valid after process release");
        assert!(root.join("transcript.db.writers").is_dir());
    }
}
