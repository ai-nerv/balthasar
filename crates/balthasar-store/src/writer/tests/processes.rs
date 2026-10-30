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

#[test]
fn independent_reader_checks_committed_rows() {
    let Some(path) = std::env::var_os("BALTHASAR_REPLAY_TEST_PATH") else {
        return;
    };
    let count: usize = std::env::var("BALTHASAR_REPLAY_TEST_COUNT")
        .expect("expected count")
        .parse()
        .expect("count");
    let store = Transcript::open(Path::new(&path)).expect("reader store");
    let rows = store.replay(&key()).expect("reader replay");
    assert_eq!(rows.len(), count, "all acknowledged rows remain visible");
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.cursor, index as u64 + 1);
        assert_eq!(row.text, format!("turn-{}", row.cursor));
    }
}

fn read_in_process(path: &Path, count: usize) {
    let mut child = Child(
        Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "writer::tests::processes::independent_reader_checks_committed_rows",
                "--nocapture",
            ])
            .env("BALTHASAR_REPLAY_TEST_PATH", path)
            .env("BALTHASAR_REPLAY_TEST_COUNT", count.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("reader child"),
    );
    assert!(until(|| child.0.try_wait().expect("reader status")).success());
}

fn replay_visibility(boundary: &str) {
    let root = Scratch::new("writer-replay", boundary);
    let path = root.join("transcript.db");
    let store = Transcript::open(&path).expect("writer store");
    for cursor in 1..=3 {
        let turn = crate::Turn {
            cursor,
            role: "assistant".into(),
            kind: "prose".into(),
            text: format!("turn-{cursor}"),
            ..crate::Turn::default()
        };
        assert_eq!(
            store.insert(&key(), &turn).expect("commit"),
            crate::Inserted::Fresh
        );
        match boundary {
            "reservation" => {
                let held = store
                    .reserve_writer(&key())
                    .expect("reserve")
                    .expect("free");
                held.check().expect("validate");
                read_in_process(&path, cursor as usize);
            }
            "mutation" => {
                let _held = store
                    .reserve_mutation(&key())
                    .expect("mutation")
                    .expect("free");
                read_in_process(&path, cursor as usize);
            }
            "maintenance" => {
                let held = store.maintenance().expect("maintenance").expect("free");
                assert!(held.available(key().as_str()).expect("probe"));
                read_in_process(&path, cursor as usize);
            }
            "reopen" => {
                drop(Transcript::open(&path).expect("same-process reader"));
                read_in_process(&path, cursor as usize);
            }
            _ => panic!("unknown boundary"),
        }
    }
    drop(store);
    read_in_process(&path, 3);
}

#[test]
fn writer_reservation_preserves_sqlite_locks_and_live_replay() {
    replay_visibility("reservation");
}

#[test]
fn mutation_guards_preserve_sqlite_locks_and_live_replay() {
    replay_visibility("mutation");
}

#[test]
fn maintenance_guards_preserve_sqlite_locks_and_live_replay() {
    replay_visibility("maintenance");
}

#[test]
fn reopening_a_store_preserves_sqlite_locks_and_live_replay() {
    replay_visibility("reopen");
}
