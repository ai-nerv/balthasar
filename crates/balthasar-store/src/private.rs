//! Store files are the owner's alone: directories 0700, databases and their WAL and SHM 0600,
//! made so when created and tightened whenever they are opened.

use crate::StoreError;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// What SQLite keeps beside a database.
const BESIDE: &[&str] = &["", "-wal", "-shm", "-journal"];

/// Create `dir` and any missing parents mode 0700.
pub fn dir(dir: &Path) -> Result<(), StoreError> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| StoreError::Io(dir.to_owned(), e))
}

/// Make a database file the owner's alone before SQLite opens it: its directory created 0700,
/// the file created 0600, and what is already there tightened.
pub fn before_open(path: &Path) -> Result<(), StoreError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        dir(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| StoreError::Io(path.to_owned(), e))?;
    tighten(path)
}

/// Tighten a database and every file SQLite keeps beside it to 0600, where this uid owns them.
pub fn tighten(path: &Path) -> Result<(), StoreError> {
    let uid = rustix::process::getuid().as_raw();
    for suffix in BESIDE {
        let file = beside(path, suffix);
        let Ok(meta) = std::fs::symlink_metadata(&file) else {
            continue;
        };
        if meta.is_file() && meta.uid() == uid && meta.mode() & 0o077 != 0 {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| StoreError::Io(file.clone(), e))?;
        }
    }
    Ok(())
}

fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use balthasar_model::scratch::Scratch;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).expect("there").permissions().mode() & 0o777
    }

    #[test]
    fn a_new_store_is_the_owners_alone() {
        let root = Scratch::new("balthasar-private", "new");
        let path = root.join("fresh/memory.db");
        let store = crate::Store::open(&path).expect("open");
        store.bytes().expect("written");
        assert_eq!(mode(&root.join("fresh")), 0o700);
        for suffix in ["", "-wal", "-shm"] {
            let file = beside(&path, suffix);
            if file.exists() {
                assert_eq!(mode(&file), 0o600, "{}", file.display());
            }
        }
    }

    #[test]
    fn an_old_store_left_open_to_others_is_tightened_on_open() {
        let root = Scratch::new("balthasar-private", "old");
        let path = root.join("project-transcript.db");
        drop(crate::Transcript::open(&path).expect("first"));
        for suffix in ["", "-wal", "-shm"] {
            let file = beside(&path, suffix);
            if file.exists() {
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644))
                    .expect("loosen");
            }
        }
        drop(crate::Transcript::open(&path).expect("again"));
        assert_eq!(mode(&path), 0o600);
        for suffix in ["-wal", "-shm"] {
            let file = beside(&path, suffix);
            if file.exists() {
                assert_eq!(mode(&file), 0o600, "{}", file.display());
            }
        }
    }
}
