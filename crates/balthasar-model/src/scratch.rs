//! A temporary directory that removes itself.

use std::path::{Path, PathBuf};

/// Distinguishes two scratches made in one process.
///
/// The pid alone is not enough: two tests in one binary may choose the same name.
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A directory under the temporary directory, removed when this is dropped.
///
/// Derefs to [`Path`]; a caller that wants to *own* the path wants [`Scratch::leak`].
#[derive(Debug)]
pub struct Scratch {
    path: PathBuf,
}

impl Scratch {
    /// A fresh directory, named after `prefix` and `name`.
    ///
    /// # Panics
    /// If the directory cannot be created.
    #[must_use]
    pub fn new(prefix: &str, name: &str) -> Self {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{n}-{name}", std::process::id()));
        // A pid is reused, and a run that was killed rather than unwound left its directory.
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Self { path }
    }

    /// Keep the directory, and stop owning it.
    ///
    /// Whoever calls this owns the cleanup.
    #[must_use]
    pub fn leak(self) -> PathBuf {
        let path = self.path.clone();
        std::mem::forget(self);
        path
    }
}

/// A path *inside* a scratch directory, where the directory is what is removed. Derefs to the file.
#[derive(Debug)]
pub struct ScratchFile {
    _dir: Scratch,
    path: PathBuf,
}

impl Scratch {
    /// A named file inside a fresh scratch directory.
    #[must_use]
    pub fn file(prefix: &str, name: &str, file: &str) -> ScratchFile {
        let dir = Scratch::new(prefix, name);
        let path = dir.join(file);
        ScratchFile { _dir: dir, path }
    }
}

impl std::ops::Deref for ScratchFile {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for ScratchFile {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl std::ops::Deref for Scratch {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Ignored: a cleanup that panicked during an unwind would abort the process.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::Scratch;

    #[test]
    fn a_scratch_removes_itself() {
        let path = {
            let dir = Scratch::new("balthasar-scratch", "gone");
            std::fs::write(dir.join("f"), "x").expect("write");
            dir.to_path_buf()
        };
        assert!(!path.exists(), "{}", path.display());
    }

    #[test]
    fn a_scratch_removes_itself_when_a_test_panics() {
        let path = std::panic::catch_unwind(|| {
            let dir = Scratch::new("balthasar-scratch", "panicked");
            let path = dir.to_path_buf();
            std::fs::write(dir.join("f"), "x").expect("write");
            std::panic::panic_any(path);
        })
        .expect_err("the closure panics");
        let path = path.downcast::<std::path::PathBuf>().expect("the path");
        assert!(!path.exists(), "{}", path.display());
    }

    #[test]
    fn two_scratches_of_one_name_are_two_directories() {
        let a = Scratch::new("balthasar-scratch", "same");
        let b = Scratch::new("balthasar-scratch", "same");
        assert_ne!(a.to_path_buf(), b.to_path_buf());
        assert!(a.exists() && b.exists());
    }

    #[test]
    fn a_leaked_scratch_outlives_the_guard() {
        let path = Scratch::new("balthasar-scratch", "leaked").leak();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&path);
    }
}
