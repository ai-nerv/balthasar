//! Where this user's runtime files live, and the checks a directory must pass to hold them.
//!
//! `$XDG_RUNTIME_DIR` is trusted only when it is absolute, a directory, owned by this uid, and
//! closed to group and other. Otherwise a program's directory is `<tmp>/<program>-<uid>`.

use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Every program of the family, by the name its runtime directory carries.
pub const FAMILY: &[&str] = &["magi", "casper", "melchior", "memory", "balthasar"];

/// This process's real uid.
#[must_use]
pub fn uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// `$XDG_RUNTIME_DIR`, when it is safe to use. Empty is unset.
#[must_use]
pub fn runtime_dir() -> Option<PathBuf> {
    safe_runtime(std::env::var_os("XDG_RUNTIME_DIR"), uid())
}

/// The same decision, from what the environment said and whose it must be.
#[must_use]
pub fn safe_runtime(said: Option<std::ffi::OsString>, uid: u32) -> Option<PathBuf> {
    let path = PathBuf::from(said.filter(|value| !value.is_empty())?);
    if !path.is_absolute() {
        return None;
    }
    let meta = std::fs::metadata(&path).ok()?;
    (meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0).then_some(path)
}

/// Where one program keeps its runtime files.
#[must_use]
pub fn program_dir(program: &str) -> PathBuf {
    program_dir_in(runtime_dir(), program, uid())
}

/// The same, given the runtime directory already decided.
#[must_use]
pub fn program_dir_in(runtime: Option<PathBuf>, program: &str, uid: u32) -> PathBuf {
    match runtime {
        Some(runtime) => runtime.join(program),
        None => std::env::temp_dir().join(format!("{program}-{uid}")),
    }
}

/// Every directory a family program may keep its sockets in, whether or not it exists yet.
#[must_use]
pub fn family_dirs() -> Vec<PathBuf> {
    family_dirs_in(runtime_dir(), &std::env::temp_dir(), uid())
}

/// The same, from a decided runtime directory and temporary directory.
#[must_use]
pub fn family_dirs_in(runtime: Option<PathBuf>, tmp: &Path, uid: u32) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for program in FAMILY {
        if let Some(runtime) = &runtime {
            out.push(runtime.join(program));
        }
        out.push(tmp.join(format!("{program}-{uid}")));
    }
    out
}

/// Create `path` (and any missing parents) mode 0700, then require it to be a real directory,
/// owned by this uid, closed to group and other. One of this uid's own left open by an older
/// build is closed first.
///
/// # Errors
/// When it cannot be created, or when what is there fails the check. The error names the path.
pub fn private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|why| named(path, &why.to_string()))?;
    let meta = std::fs::symlink_metadata(path).map_err(|why| named(path, &why.to_string()))?;
    if meta.is_dir() && meta.uid() == uid() && meta.mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|why| named(path, &why.to_string()))?;
    }
    checked_dir(path, uid())
}

/// The check alone: a directory, not a symlink, owned by `uid`, with no group or other bits.
///
/// # Errors
/// When `path` fails any part of it, naming the path and what was wrong.
pub fn checked_dir(path: &Path, uid: u32) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path).map_err(|why| named(path, &why.to_string()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(named(path, "is not a directory"));
    }
    if meta.uid() != uid {
        return Err(named(
            path,
            &format!("belongs to uid {}, not {uid}", meta.uid()),
        ));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(named(
            path,
            &format!(
                "is mode {:o}; group and other must have nothing",
                meta.mode() & 0o777
            ),
        ));
    }
    Ok(())
}

fn named(path: &Path, why: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!("refusing {}: {why}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch::Scratch;

    #[test]
    fn an_empty_runtime_dir_is_unset() {
        assert_eq!(safe_runtime(Some(std::ffi::OsString::new()), uid()), None);
        assert_eq!(safe_runtime(None, uid()), None);
    }

    #[test]
    fn a_relative_runtime_dir_is_not_trusted() {
        assert_eq!(safe_runtime(Some("run/user".into()), uid()), None);
    }

    #[test]
    fn a_runtime_dir_open_to_others_is_not_trusted() {
        let dir = Scratch::new("balthasar-runtime", "open");
        std::fs::set_permissions(&*dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        assert_eq!(safe_runtime(Some(dir.as_os_str().into()), uid()), None);
        std::fs::set_permissions(&*dir, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        assert_eq!(
            safe_runtime(Some(dir.as_os_str().into()), uid()),
            Some(dir.to_path_buf())
        );
    }

    #[test]
    fn a_runtime_dir_owned_by_somebody_else_is_not_trusted() {
        let dir = Scratch::new("balthasar-runtime", "theirs");
        std::fs::set_permissions(&*dir, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        assert_eq!(safe_runtime(Some(dir.as_os_str().into()), uid() + 1), None);
    }

    #[test]
    fn without_a_runtime_dir_each_program_gets_its_own_uid_suffixed_one() {
        let at = program_dir_in(None, "memory", 1234);
        assert_eq!(at, std::env::temp_dir().join("memory-1234"));
        assert_eq!(
            program_dir_in(Some("/w/runtime".into()), "memory", 7),
            PathBuf::from("/w/runtime/memory")
        );
    }

    #[test]
    fn a_private_dir_is_made_closed_and_checked() {
        let root = Scratch::new("balthasar-runtime", "private");
        let made = root.join("a/b");
        private_dir(&made).expect("made");
        let mode = std::fs::metadata(&made)
            .expect("there")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn a_dir_of_ours_left_open_is_closed_and_one_open_to_others_is_refused_by_name() {
        let root = Scratch::new("balthasar-runtime", "loose");
        std::fs::set_permissions(&*root, std::fs::Permissions::from_mode(0o775)).expect("chmod");
        private_dir(&root).expect("ours, so closed");
        let mode = std::fs::metadata(&*root)
            .expect("there")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        std::fs::set_permissions(&*root, std::fs::Permissions::from_mode(0o775)).expect("chmod");
        let why = checked_dir(&root, uid()).expect_err("refused");
        assert!(
            why.to_string().contains(&root.display().to_string()),
            "{why}"
        );
    }

    #[test]
    fn a_symlink_in_place_of_the_dir_is_refused() {
        let root = Scratch::new("balthasar-runtime", "link");
        let real = root.join("real");
        std::fs::create_dir(&real).expect("mkdir");
        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        assert!(checked_dir(&link, uid()).is_err());
    }

    #[test]
    fn a_dir_owned_by_another_uid_is_refused() {
        let root = Scratch::new("balthasar-runtime", "other");
        std::fs::set_permissions(&*root, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        let why = checked_dir(&root, uid() + 1).expect_err("refused");
        assert!(why.to_string().contains("belongs to uid"), "{why}");
    }
}
