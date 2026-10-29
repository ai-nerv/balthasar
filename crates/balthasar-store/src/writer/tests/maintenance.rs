use super::*;
use std::{
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn admission_excludes_new_writers_after_probes_without_displacing_old_owners() {
    let root = Scratch::new("maintenance", "admission");
    let store = Transcript::open(&root.join("transcript.db")).expect("store");
    let other = Transcript::open(store.path()).expect("independent handle");
    let owned = SessionId::new("owned");
    let writer = store
        .reserve_writer(&owned)
        .expect("reserve")
        .expect("free");
    let mutation = store
        .reserve_mutation(&key())
        .expect("mutation")
        .expect("free");
    let gate = store
        .maintenance()
        .expect("maintenance")
        .expect("free admission");
    assert!(!gate.available(owned.as_str()).expect("existing writer"));
    assert!(!gate.available(key().as_str()).expect("existing mutation"));
    assert!(gate.available("unowned").expect("probe"));
    assert!(
        other
            .reserve_writer(&SessionId::new("unowned"))
            .expect("writer blocked")
            .is_none()
    );
    assert!(
        other
            .reserve_mutation(&SessionId::new("unowned"))
            .expect("mutation blocked")
            .is_none()
    );
    assert!(other.maintenance().expect("maintenance excluded").is_none());
    store
        .check_writer(&writer, &owned)
        .expect("old owner retains authority");
    drop(mutation);
    assert!(gate.available(key().as_str()).expect("released mutation"));
    drop(writer);
    assert!(gate.available(owned.as_str()).expect("released owner"));
    assert!(
        other
            .reserve_writer(&owned)
            .expect("reacquisition excluded")
            .is_none()
    );
    drop(gate);
    assert!(
        other
            .reserve_writer(&owned)
            .expect("admission reopened")
            .is_some()
    );
}

#[test]
fn replaced_admission_file_invalidates_maintenance_before_another_probe() {
    let root = Scratch::new("maintenance", "inode");
    let store = Transcript::open(&root.join("transcript.db")).expect("store");
    let guard = store.maintenance().expect("maintenance").expect("free");
    let file = root.join("transcript.db.writers/admission.lock");
    std::fs::rename(&file, root.join("old-admission")).expect("replace admission");
    std::fs::write(&file, b"").expect("new file");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("private");
    assert!(guard.available("unowned").is_err());
}

fn seed(store: &mut Transcript, count: usize) -> Vec<SessionId> {
    (0..count)
        .map(|i| {
            let key = SessionId::new(format!("transcript-{i:04}"));
            store.bind_run(&key, Some("root")).expect("binding");
            store
                .write(
                    &key,
                    &crate::Turn {
                        cursor: 1,
                        at: 1,
                        role: "user".into(),
                        kind: "prose".into(),
                        text: "kept".into(),
                        ..Default::default()
                    },
                )
                .expect("turn");
            key
        })
        .collect()
}

#[test]
fn busy_run_purge_preserves_every_member_and_releases_admission() {
    let root = Scratch::new("maintenance", "purge-refusal");
    let mut store = Transcript::open(&root.join("transcript.db")).expect("store");
    let keys = seed(&mut store, 3);
    let last = keys.last().expect("last");
    let writer = store
        .reserve_writer(last)
        .expect("reserve last")
        .expect("free");
    assert!(crate::purge_run(&store, &SessionId::new("root")).is_err());
    for key in &keys {
        assert_eq!(store.replay(key).expect("unchanged history").len(), 1);
    }
    assert!(
        store
            .reserve_mutation(&keys[0])
            .expect("admission released")
            .is_some()
    );
    drop(writer);
    assert_eq!(
        crate::purge_run(&store, &SessionId::new("root")).expect("purge"),
        3
    );
}

#[test]
fn retention_skips_existing_writer_and_releases_admission_after_commit() {
    let root = Scratch::new("maintenance", "prune");
    let mut store = Transcript::open(&root.join("transcript.db")).expect("store");
    let keys = seed(&mut store, 3);
    let writer = store
        .reserve_writer(&keys[0])
        .expect("reserve")
        .expect("free");
    let pruned = crate::prune_turns(&store, None, None, 2, &[]).expect("prune");
    assert_eq!(pruned.sessions, 2);
    assert_eq!(pruned.turns, 2);
    assert_eq!(store.replay(&keys[0]).expect("owned history").len(), 1);
    store
        .check_writer(&writer, &keys[0])
        .expect("owner still valid");
    assert!(
        store
            .reserve_mutation(&keys[1])
            .expect("admission released")
            .is_some()
    );
    drop(writer);
    assert_eq!(
        crate::prune_turns(&store, None, None, 2, &[])
            .expect("final prune")
            .turns,
        1
    );
}

#[test]
fn failed_maintenance_rolls_back_and_reopens_admission() {
    let root = Scratch::new("maintenance", "rollback");
    let mut store = Transcript::open(&root.join("transcript.db")).expect("store");
    let keys = seed(&mut store, 3);
    store.db().execute_batch("CREATE TRIGGER refuse_delete BEFORE DELETE ON turn BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").expect("failure trigger");
    assert!(crate::purge_run(&store, &SessionId::new("root")).is_err());
    assert!(crate::prune_turns(&store, None, None, 2, &[]).is_err());
    for key in &keys {
        assert_eq!(store.replay(key).expect("rollback history").len(), 1);
        assert!(
            store
                .reserve_writer(key)
                .expect("released admission")
                .is_some()
        );
    }
    assert_eq!(
        store
            .spans_matching("kept", 10)
            .expect("search rollback")
            .len(),
        3
    );
}

struct Child(std::process::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(root: &Path, mode: &str) -> Child {
    Child(
        Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "writer::tests::maintenance::maintenance_child",
                "--nocapture",
            ])
            .env("BALTHASAR_MAINTENANCE_TEST_ROOT", root)
            .env("BALTHASAR_MAINTENANCE_TEST_MODE", mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("maintenance child"),
    )
}

fn until<T>(mut ready: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(result) = ready() {
            return result;
        }
        assert!(Instant::now() < deadline, "maintenance child deadline");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn maintenance_child() {
    let Some(root) = std::env::var_os("BALTHASAR_MAINTENANCE_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    match std::env::var("BALTHASAR_MAINTENANCE_TEST_MODE")
        .expect("mode")
        .as_str()
    {
        "hold" => {
            let store = Transcript::open(&root.join("transcript.db")).expect("child store");
            let gate = store.maintenance().expect("maintenance").expect("free");
            assert!(gate.available("unowned").expect("probe"));
            std::fs::write(root.join("ready"), b"held").expect("ready");
            std::io::stdin().read_exact(&mut [0]).expect("release");
            assert!(gate.available("unowned").expect("held until release"));
        }
        "bounded" => {
            use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
            let previous = getrlimit(Resource::Nofile);
            setrlimit(
                Resource::Nofile,
                Rlimit {
                    current: Some(64),
                    maximum: previous.maximum,
                },
            )
            .expect("child descriptor limit");
            for prune in [false, true] {
                let mut store =
                    Transcript::open(&root.join(if prune { "prune.db" } else { "purge.db" }))
                        .expect("bounded store");
                let keys = seed(&mut store, 192);
                if prune {
                    let result =
                        crate::prune_turns(&store, None, None, 2, &[]).expect("bounded prune");
                    assert_eq!(result.sessions, 192);
                    assert_eq!(result.turns, 192);
                } else {
                    assert_eq!(
                        crate::purge_run(&store, &SessionId::new("root")).expect("bounded purge"),
                        192
                    );
                }
                for key in keys {
                    assert!(store.replay(&key).expect("removed").is_empty());
                }
            }
        }
        _ => panic!("unknown child mode"),
    }
}

#[test]
fn purge_and_retention_are_bounded_under_a_64_descriptor_limit() {
    let root = Scratch::new("maintenance", "bounded");
    let mut child = spawn(&root, "bounded");
    assert!(until(|| child.0.try_wait().expect("wait")).success());
}

#[test]
fn independent_process_cannot_reserve_between_probe_and_maintenance_release() {
    use std::io::Write;
    let root = Scratch::new("maintenance", "process");
    let store = Transcript::open(&root.join("transcript.db")).expect("parent store");
    let unrelated = Transcript::open(&root.join("other.db")).expect("other store");
    let mut child = spawn(&root, "hold");
    until(|| {
        assert!(child.0.try_wait().expect("child state").is_none());
        root.join("ready").is_file().then_some(())
    });
    assert!(
        store
            .reserve_writer(&SessionId::new("unowned"))
            .expect("excluded")
            .is_none()
    );
    assert!(
        store
            .reserve_mutation(&key())
            .expect("mutation excluded")
            .is_none()
    );
    assert!(
        unrelated
            .reserve_writer(&key())
            .expect("other store")
            .is_some()
    );
    child
        .0
        .stdin
        .take()
        .expect("release pipe")
        .write_all(b"x")
        .expect("release");
    assert!(until(|| child.0.try_wait().expect("wait")).success());
    assert!(store.reserve_writer(&key()).expect("released").is_some());
}
