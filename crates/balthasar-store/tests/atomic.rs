use balthasar_model::{SessionId, scratch::Scratch};
use balthasar_store::{Change, StoreError, Transcript, Turn};
use serde_json::json;

fn write_pair(store: &Transcript) -> Result<(), StoreError> {
    let fields = json!({"title":"Atomic note", "text":"One committed result.", "pinned":false});
    let note = store.put_note(None, Some(&fields), 1)?;
    store.record_change(
        &Change {
            note,
            op: "add".into(),
            after: Some(fields),
            state: "applied".into(),
            by: "test".into(),
            ..Change::default()
        },
        None,
    )?;
    Ok(())
}

#[test]
fn errors_and_panics_roll_back_every_write_and_release_the_transaction() {
    let dir = Scratch::new("transcript-atomic", "rollback");
    let path = dir.join("transcript.db");
    let store = Transcript::open(&path).expect("store");
    let result: Result<(), StoreError> = store.atomic(|store| {
        write_pair(store)?;
        Err(StoreError::Unknown("injected failure".into()))
    });
    assert!(result.is_err());
    assert!(store.notes().expect("notes").is_empty());
    assert!(store.changes(20).expect("audit").is_empty());
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<(), StoreError> = store.atomic(|store| {
            write_pair(store)?;
            panic!("injected panic");
        });
    }));
    assert!(panic.is_err());
    drop(store);
    let store = Transcript::open(&path).expect("reopened");
    assert!(store.notes().expect("notes").is_empty());
    assert!(store.changes(20).expect("audit").is_empty());
    store.atomic(write_pair).expect("retry");
    assert_eq!(store.notes().expect("notes").len(), 1);
    assert_eq!(store.changes(20).expect("audit").len(), 1);
}

#[test]
fn source_writes_are_excluded_and_partial_notes_are_invisible_until_commit() {
    let dir = Scratch::new("transcript-atomic", "isolation");
    let path = dir.join("transcript.db");
    let mut store = Transcript::open(&path).expect("store");
    let session = SessionId::new("source");
    store
        .write(
            &session,
            &Turn {
                cursor: 1,
                role: "user".into(),
                kind: "user".into(),
                text: "Always use uv.".into(),
                ..Turn::default()
            },
        )
        .expect("source");
    let other = rusqlite::Connection::open(&path).expect("other connection");
    other
        .busy_timeout(std::time::Duration::ZERO)
        .expect("no wait");
    store.atomic(|store| {
        assert_eq!(store.at(&session, 1)?.expect("source").text, "Always use uv.");
        let error = other.execute("UPDATE turn SET text = 'Changed' WHERE cursor = 1", []).expect_err("writer excluded");
        assert!(matches!(error, rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::DatabaseBusy));
        write_pair(store)?;
        let count: i64 = other.query_row("SELECT count(*) FROM note", [], |r| r.get(0)).expect("reader");
        assert_eq!(count, 0);
        Ok(())
    }).expect("commit");
    other
        .execute("UPDATE turn SET text = 'Changed' WHERE cursor = 1", [])
        .expect("writer resumes");
    let count: i64 = other
        .query_row("SELECT count(*) FROM note", [], |r| r.get(0))
        .expect("reader");
    assert_eq!(count, 1);
}

#[test]
fn a_nested_transaction_is_refused_without_running_its_callback() {
    let store = Transcript::ephemeral().expect("store");
    store
        .atomic(|store| {
            let mut ran = false;
            let inner = store.atomic(|_| {
                ran = true;
                Ok(())
            });
            assert!(inner.is_err());
            assert!(!ran);
            write_pair(store)
        })
        .expect("outer transaction remains usable");
    assert_eq!(store.notes().expect("notes").len(), 1);
}

#[test]
fn atomic_child() {
    let Some(path) = std::env::var_os("BALTHASAR_ATOMIC_TEST") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    let store = Transcript::open(&path).expect("child store");
    store
        .atomic(|store| {
            write_pair(store)?;
            std::fs::write(path.with_extension("ready"), b"uncommitted").expect("marker");
            std::thread::sleep(std::time::Duration::from_secs(60));
            Ok(())
        })
        .expect("child transaction");
}

struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_killed_process_leaves_no_half_committed_note_or_audit() {
    let dir = Scratch::new("transcript-atomic", "crash");
    let path = dir.join("transcript.db");
    drop(Transcript::open(&path).expect("owned store"));
    let mut child = Child(
        std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "atomic_child", "--nocapture"])
            .env("BALTHASAR_ATOMIC_TEST", &path)
            .current_dir(&*dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("owned child"),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !path.with_extension("ready").exists() {
        assert!(
            child.0.try_wait().expect("status").is_none(),
            "child exited before fault point"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "child never reached fault point"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    child.0.kill().expect("kill at fault point");
    child.0.wait().expect("reap child");
    let reopened = Transcript::open(&path).expect("crash recovery");
    assert!(reopened.notes().expect("notes").is_empty());
    assert!(reopened.changes(20).expect("audit").is_empty());
    reopened.atomic(write_pair).expect("retry after crash");
    assert_eq!(reopened.notes().expect("notes").len(), 1);
    assert_eq!(reopened.changes(20).expect("audit").len(), 1);
}
