//! Sockets left behind by balthasars that are no longer running.
//!
//! `$XDG_RUNTIME_DIR/balthasar` is one directory shared by every instance on the machine, and
//! nothing has ever enumerated it. A balthasar that unlinks its own socket on the way out leaves
//! nothing — but the deaths that matter are the ones with no way out: `SIGKILL`, the OOM killer,
//! a machine that lost power. Those still leave a file, and hundreds of `api@*.sock` from one
//! coordinator's short-lived instances had collected here before anybody looked.
//!
//! A corpse is not untidiness. It is the input to a worse bug: a forked child's probe reads a
//! stale file as a live daemon, waits, gets nothing, and unlinks it — and the socket it unlinks
//! is sometimes the live one. So the pile is swept where balthasar already has the directory
//! open, which is the only moment it looks at it at all.
//!
//! melchior sweeps at the point its roster is *read*, because it has a roster; a dead session is
//! discovered by the next process that asks who is listening. balthasar has no roster and nobody
//! asks, so start-up is the only equivalent moment there is.

use std::os::unix::net::UnixStream;
use std::path::Path;

/// Unlink every `api@*.sock` in `dir` that nothing answers on, and say how many went.
///
/// **Refused, not merely unanswered.** `ECONNREFUSED` on a unix socket means exactly one thing:
/// the file is a socket and no process has it bound. Every other error is a reason to leave the
/// file alone — `EACCES` is somebody else's socket, `EMFILE` is this process out of descriptors,
/// and a timeout is a daemon that is busy. `is_ok()` would have been the shorter test and would
/// have deleted a live balthasar's name the first time this process ran out of file descriptors.
///
/// Only `api@*.sock`. `balthasar.tool` is the descriptor a caller with no socket spawns from and
/// `given.lua` is what a coordinator configured; neither is a corpse and neither answers.
pub fn swept(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| is_socket_name(&entry.file_name().to_string_lossy()))
        .filter(|entry| refused(&entry.path()))
        // A file two balthasars noticed at once is removed by one of them and already gone for
        // the other, which is not an error and is not counted.
        .filter(|entry| std::fs::remove_file(entry.path()).is_ok())
        .count()
}

/// Whether a name in the runtime directory is one of our sockets.
fn is_socket_name(name: &str) -> bool {
    name.starts_with("api@") && name.ends_with(".sock")
}

/// Whether connecting is refused outright, which is the only proof that nothing is bound.
///
/// A listener answers from the moment it is bound — the kernel queues the connection whether or
/// not anybody has called accept — so a balthasar that is merely busy, or one holding all eight
/// of its callers, still completes this connection and is never read as dead.
fn refused(path: &Path) -> bool {
    match UnixStream::connect(path) {
        Ok(_) => false,
        Err(why) => why.kind() == std::io::ErrorKind::ConnectionRefused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use balthasar_model::scratch::Scratch;

    #[test]
    fn a_socket_nothing_is_bound_to_is_swept() {
        let dir = Scratch::new("balthasar-corpses", "dead");
        let corpse = dir.join("api@dead.sock");
        // A real socket file rather than a regular one, and then no listener: this is what a
        // `SIGKILL` leaves, and a regular file would prove nothing about `ECONNREFUSED`.
        let bound = std::os::unix::net::UnixListener::bind(&corpse).expect("bind");
        drop(bound);
        assert!(corpse.exists(), "the kernel leaves the file behind");

        assert_eq!(swept(&dir), 1);
        assert!(!corpse.exists());
    }

    #[test]
    fn a_socket_something_is_listening_on_is_left_alone() {
        // The failure that would matter: a sweep that took a live balthasar's name would leave a
        // running daemon unreachable, which is the bug the whole accept loop was rewritten for.
        let dir = Scratch::new("balthasar-corpses", "live");
        let live = dir.join("api@live.sock");
        let _bound = std::os::unix::net::UnixListener::bind(&live).expect("bind");

        assert_eq!(swept(&dir), 0);
        assert!(live.exists());
    }

    #[test]
    fn a_socket_that_cannot_be_dialled_at_all_is_left_alone() {
        // The test above passes just as happily against `Err(_) => true`, which is the version
        // that deletes a live balthasar's name the first time this process is out of descriptors
        // or the socket belongs to somebody else. So: a socket that fails to connect for a
        // reason that is *not* a refusal. Taking write permission off it makes `connect` return
        // `EACCES` every time, and nothing about that says the far end is dead.
        use std::os::unix::fs::PermissionsExt;
        let dir = Scratch::new("balthasar-corpses", "unreachable");
        let closed = dir.join("api@closed.sock");
        let _bound = std::os::unix::net::UnixListener::bind(&closed).expect("bind");
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        // Root ignores the mode, and a run as root would prove nothing either way.
        if std::os::unix::net::UnixStream::connect(&closed).is_ok() {
            return;
        }

        assert_eq!(swept(&dir), 0);
        assert!(closed.exists(), "an undiagnosed socket is not a corpse");
    }

    #[test]
    fn what_is_not_a_socket_of_ours_is_not_touched() {
        // `balthasar.tool` is how a caller that found no socket spawns one, and `given.lua` is
        // the configuration a coordinator sent. Sweeping either would break the next start-up.
        let dir = Scratch::new("balthasar-corpses", "others");
        for name in ["balthasar.tool", "given.lua", "api@dead.sock.bak", "notes"] {
            std::fs::write(dir.join(name), "x").expect("write");
        }
        assert_eq!(swept(&dir), 0);
        for name in ["balthasar.tool", "given.lua", "api@dead.sock.bak", "notes"] {
            assert!(dir.join(name).exists(), "{name} was swept");
        }
    }

    #[test]
    fn a_regular_file_wearing_a_sockets_name_goes_too() {
        // Not a socket at all, so connecting fails with `ECONNREFUSED` here as well. It is still
        // a name the next instance would have to disprove, and `bind` already replaces one of
        // these when it happens to be its own.
        let dir = Scratch::new("balthasar-corpses", "impostor");
        let impostor = dir.join("api@impostor.sock");
        std::fs::write(&impostor, "not a socket").expect("write");
        assert_eq!(swept(impostor.parent().expect("a parent")), 1);
        assert!(!impostor.exists());
    }

    #[test]
    fn a_directory_that_is_not_there_is_not_an_error() {
        let dir = Scratch::new("balthasar-corpses", "absent");
        assert_eq!(swept(&dir.join("never")), 0);
    }
}
