//! Taking a socket name, and telling a corpse from a live program.
//!
//! A name is taken under an exclusive `flock` on `<socket>.lock`, held for as long as the socket
//! listens. An existing path is removed only when connecting to it is refused outright and it is
//! a socket of this uid's; any other answer is a live program having a bad moment.

use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// One socket name, bound and held.
#[derive(Debug)]
pub struct Claimed {
    pub listener: UnixListener,
    pub path: PathBuf,
    /// `(dev, ino)` of what this bound, so leaving removes only that.
    inode: (u64, u64),
    lock: Option<(File, PathBuf)>,
}

impl Drop for Claimed {
    fn drop(&mut self) {
        if inode_of(&self.path) == Some(self.inode) {
            let _ = std::fs::remove_file(&self.path);
        }
        if let Some((held, path)) = self.lock.take() {
            let _ = std::fs::remove_file(&path);
            drop(held);
        }
    }
}

/// `<socket>.lock`, beside it.
#[must_use]
pub fn lock_path(socket: &Path) -> PathBuf {
    let mut name = socket.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// Take the lock for a socket name without waiting. `None` when another process holds it.
///
/// # Errors
/// When the lock file cannot be opened.
pub fn lock(socket: &Path) -> std::io::Result<Option<(File, PathBuf)>> {
    let path = lock_path(socket);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some((file, path))),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(why)) => Err(why),
    }
}

/// Whether `path` is a socket of this uid's that nothing is bound to.
#[must_use]
pub fn corpse(path: &Path, uid: u32) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.file_type().is_socket() || meta.uid() != uid {
        return false;
    }
    matches!(
        UnixStream::connect(path),
        Err(why) if why.kind() == std::io::ErrorKind::ConnectionRefused
    )
}

fn inode_of(path: &Path) -> Option<(u64, u64)> {
    std::fs::symlink_metadata(path)
        .ok()
        .map(|meta| (meta.dev(), meta.ino()))
}

fn in_use(path: &Path, why: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::AddrInUse,
        format!("{}: {why}", path.display()),
    )
}

/// Bind `path`, clearing only a corpse, and hold its lock for as long as the result lives.
///
/// # Errors
/// `AddrInUse` when the lock is held or anything other than a refusal answers on the path;
/// otherwise whatever opening, removing or binding said.
pub fn claim(path: &Path) -> std::io::Result<Claimed> {
    let Some((held, lock)) = lock(path)? else {
        return Err(in_use(path, "another balthasar is taking this name"));
    };
    let bound = bind_held(path);
    if bound.is_err() {
        // Still holding it, so nobody else can be relying on this lock file.
        let _ = std::fs::remove_file(&lock);
    }
    let (listener, inode) = bound?;
    Ok(Claimed {
        listener,
        path: path.to_owned(),
        inode,
        lock: Some((held, lock)),
    })
}

/// Clear a corpse at `path` and bind there, the lock already held.
fn bind_held(path: &Path) -> std::io::Result<(UnixListener, (u64, u64))> {
    if inode_of(path).is_some() {
        if !corpse(path, rustix::process::getuid().as_raw()) {
            return Err(in_use(path, "a balthasar is already listening here"));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    let inode =
        inode_of(path).ok_or_else(|| in_use(path, "the socket vanished as it was bound"))?;
    Ok((listener, inode))
}

#[cfg(test)]
mod tests {
    use super::*;
    use balthasar_model::scratch::Scratch;

    fn uid() -> u32 {
        rustix::process::getuid().as_raw()
    }

    #[test]
    fn a_socket_nothing_is_bound_to_is_a_corpse() {
        let dir = Scratch::new("balthasar-stale", "corpse");
        let path = dir.join("api@x.sock");
        drop(UnixListener::bind(&path).expect("bind"));
        assert!(corpse(&path, uid()));
    }

    #[test]
    fn a_regular_file_is_never_a_corpse() {
        let dir = Scratch::new("balthasar-stale", "file");
        let path = dir.join("api@x.sock");
        std::fs::write(&path, "not a socket").expect("write");
        assert!(!corpse(&path, uid()));
        assert!(claim(&path).is_err(), "and is not removed to make room");
        assert!(path.exists());
        assert!(
            !lock_path(&path).exists(),
            "a claim that failed leaves no lock file"
        );
    }

    #[test]
    fn a_socket_of_another_uid_is_never_a_corpse() {
        let dir = Scratch::new("balthasar-stale", "other");
        let path = dir.join("api@x.sock");
        drop(UnixListener::bind(&path).expect("bind"));
        assert!(!corpse(&path, uid() + 1));
    }

    #[test]
    fn a_held_lock_refuses_a_second_claim() {
        let dir = Scratch::new("balthasar-stale", "locked");
        let path = dir.join("api@x.sock");
        let first = claim(&path).expect("first");
        let second = claim(&path).expect_err("second");
        assert_eq!(second.kind(), std::io::ErrorKind::AddrInUse);
        drop(first);
        assert!(!path.exists());
        assert!(!lock_path(&path).exists(), "the lock goes with the socket");
    }

    #[test]
    fn leaving_does_not_remove_a_socket_somebody_else_bound_since() {
        let dir = Scratch::new("balthasar-stale", "rebound");
        let path = dir.join("api@x.sock");
        let mine = claim(&path).expect("claim");
        std::fs::remove_file(&path).expect("taken away");
        let theirs = UnixListener::bind(&path).expect("rebound by someone else");
        drop(mine);
        assert!(path.exists(), "their socket is still there");
        drop(theirs);
    }

    #[test]
    fn two_threads_racing_for_one_stale_name_bind_it_once() {
        let dir = Scratch::new("balthasar-stale", "race");
        let path = dir.join("api@x.sock");
        drop(UnixListener::bind(&path).expect("leave a corpse"));
        let won: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || claim(&path).ok())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|racer| racer.join().expect("joined"))
            .collect();
        assert_eq!(won.len(), 1, "exactly one took the name");
    }
}
