use super::*;
use crate::Transcript;
use balthasar_model::{SessionId, scratch::Scratch};
use std::os::unix::fs::{PermissionsExt, symlink};

mod maintenance;
mod processes;

fn key() -> SessionId {
    SessionId::new("transcript")
}

#[test]
fn dropping_a_writer_releases_it_even_with_a_duplicated_descriptor() {
    let root = Scratch::new("writer", "duplicate");
    let store = Transcript::open(&root.join("transcript.db")).expect("store");
    let held = store
        .reserve_writer(&key())
        .expect("reserve")
        .expect("free");
    let duplicate = rustix::io::fcntl_dupfd_cloexec(&held.held, 3).expect("inherited reference");
    drop(held);
    let next = store
        .reserve_writer(&key())
        .expect("logical owner released")
        .expect("free despite inherited descriptor");
    next.check().expect("new owner");
    drop(duplicate);
    assert!(
        store
            .reserve_writer(&key())
            .expect("new owner retained")
            .is_none()
    );
}

#[test]
fn ordinary_mutations_share_access_but_exclude_persistent_writers() {
    let root = Scratch::new("writer", "shared");
    let a = Transcript::open(&root.join("transcript.db")).expect("store");
    let b = Transcript::open(a.path()).expect("second handle");
    let first = a.reserve_mutation(&key()).expect("mutation").expect("free");
    let second = b
        .reserve_mutation(&key())
        .expect("concurrent mutation")
        .expect("shared");
    assert!(a.reserve_writer(&key()).expect("busy writer").is_none());
    drop(first);
    assert!(
        a.reserve_writer(&key())
            .expect("still busy writer")
            .is_none()
    );
    drop(second);
    let writer = a.reserve_writer(&key()).expect("writer").expect("free");
    assert!(
        b.reserve_mutation(&key())
            .expect("excluded mutation")
            .is_none()
    );
    assert!(b.reserve_writer(&key()).expect("excluded writer").is_none());
    drop(writer);
    assert!(b.reserve_mutation(&key()).expect("released").is_some());
}

#[test]
fn one_writer_per_transcript_across_independent_store_handles() {
    let root = Scratch::new("writer", "exclusive");
    let store = Transcript::open(&root.join("transcript.db")).expect("store");
    let other = Transcript::open(store.path()).expect("second handle");
    let held = store
        .reserve_writer(&key())
        .expect("reserve")
        .expect("free");
    assert!(other.reserve_writer(&key()).expect("busy").is_none());
    assert!(
        other
            .reserve_writer(&SessionId::new("another"))
            .expect("other key")
            .is_some()
    );
    held.check().expect("valid");
    assert!(
        other
            .reserve_writer(&key())
            .expect("still busy after check")
            .is_none()
    );
    let before = inode(&held.held.metadata().expect("metadata"));
    let file = root.join("transcript.db.writers").join(&held.name);
    drop(held);
    assert_eq!(
        inode(&std::fs::metadata(&file).expect("permanent lock inode")),
        before
    );
    let next = other
        .reserve_writer(&key())
        .expect("released")
        .expect("free");
    assert_eq!(inode(&next.held.metadata().expect("metadata")), before);
}

#[test]
fn separate_stores_do_not_conflict_and_symbolic_aliases_do() {
    let root = Scratch::new("writer", "aliases");
    let a = Transcript::open(&root.join("one.db")).expect("one");
    let b = Transcript::open(&root.join("two.db")).expect("two");
    symlink(a.path(), root.join("alias.db")).expect("alias");
    let alias = Transcript::open(&root.join("alias.db")).expect("alias store");
    let _a = a.reserve_writer(&key()).expect("reserve a").expect("free");
    let _b = b.reserve_writer(&key()).expect("reserve b").expect("free");
    assert!(alias.reserve_writer(&key()).expect("same store").is_none());
}

#[test]
fn hardlinked_databases_and_replaced_databases_are_refused() {
    let root = Scratch::new("writer", "database");
    let store = Transcript::open(&root.join("transcript.db")).expect("store");
    std::fs::hard_link(store.path(), root.join("alias.db")).expect("hard link");
    assert!(store.reserve_writer(&key()).is_err());
    std::fs::remove_file(root.join("alias.db")).expect("unlink alias");
    let held = store
        .reserve_writer(&key())
        .expect("reserve")
        .expect("free");
    std::fs::rename(store.path(), root.join("retained.db")).expect("move store");
    let _new = Transcript::open(store.path()).expect("replacement");
    assert!(held.check().is_err());
    assert!(store.reserve_writer(&key()).is_err());
}

#[test]
fn unsafe_lock_files_are_refused_without_touching_the_target() {
    let root = Scratch::new("writer", "unsafe-file");
    let store = Transcript::open(&root.join("transcript.db")).expect("store");
    let held = store
        .reserve_writer(&key())
        .expect("reserve")
        .expect("free");
    let file = root.join("transcript.db.writers").join(&held.name);
    drop(held);
    let target = root.join("target");
    std::fs::rename(&file, &target).expect("move lock");
    std::fs::write(&target, "unchanged").expect("target");
    symlink(&target, &file).expect("symlink");
    assert!(store.reserve_writer(&key()).is_err());
    assert_eq!(std::fs::read_to_string(&target).expect("read"), "unchanged");
    std::fs::remove_file(&file).expect("remove symlink");
    std::fs::hard_link(&target, &file).expect("hard link");
    assert!(store.reserve_writer(&key()).is_err());
    std::fs::remove_file(&file).expect("unlink");
    std::fs::rename(&target, &file).expect("restore");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).expect("mode");
    assert!(store.reserve_writer(&key()).is_err());
}

#[test]
fn replaced_or_public_reservation_directories_are_refused() {
    let root = Scratch::new("writer", "unsafe-directory");
    let store = Transcript::open(&root.join("transcript.db")).expect("store");
    let held = store
        .reserve_writer(&key())
        .expect("reserve")
        .expect("free");
    let directory = root.join("transcript.db.writers");
    std::fs::rename(&directory, root.join("retained")).expect("rename");
    symlink(root.join("retained"), &directory).expect("link");
    assert!(held.check().is_err());
    assert!(store.reserve_writer(&key()).is_err());
    std::fs::remove_file(&directory).expect("remove link");
    std::fs::create_dir(&directory).expect("replacement");
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).expect("private");
    assert!(held.check().is_err());
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).expect("public");
    assert!(store.reserve_writer(&key()).is_err());
}

#[test]
fn replaced_lock_inode_invalidates_the_existing_reservation() {
    let root = Scratch::new("writer", "replace-lock");
    let store = Transcript::open(&root.join("transcript.db")).expect("store");
    let held = store
        .reserve_writer(&key())
        .expect("reserve")
        .expect("free");
    let file = root.join("transcript.db.writers").join(&held.name);
    std::fs::rename(&file, root.join("retained.lock")).expect("move");
    let _replacement = store
        .reserve_writer(&key())
        .expect("new inode")
        .expect("free");
    assert!(held.check().is_err());
}

#[test]
fn bounds_and_nonpersistent_stores_fail_before_creating_a_namespace() {
    let root = Scratch::new("writer", "bounds");
    let store = Transcript::open(&root.join("transcript.db")).expect("store");
    for key in [String::new(), "x".repeat(1025)] {
        assert!(store.reserve_writer(&SessionId::new(key)).is_err());
    }
    assert!(!root.join("transcript.db.writers").exists());
    assert!(
        Transcript::ephemeral()
            .expect("memory")
            .reserve_writer(&key())
            .is_err()
    );
    let held = store
        .reserve_writer(&SessionId::new("../path/secret"))
        .expect("opaque key")
        .expect("free");
    assert_eq!(held.name.len(), 69);
    assert!(!held.name.contains("secret"));
    assert_eq!(held.held.metadata().expect("file").mode() & 0o777, 0o600);
    assert_eq!(
        held.directory.metadata().expect("directory").mode() & 0o777,
        0o700
    );
}
