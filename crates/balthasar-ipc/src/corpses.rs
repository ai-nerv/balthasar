//! Sockets left behind by balthasars that are no longer running.
//!
//! `$XDG_RUNTIME_DIR/balthasar` is shared by every instance on the machine, and a death with no
//! way out leaves a socket file there that a probe reads as a live daemon.

use std::os::unix::net::UnixStream;
use std::path::Path;

/// Unlink every `api@*.sock` in `dir` that nothing answers on, and say how many went.
///
/// Only `api@*.sock`: `balthasar.tool` and `given.lua` are not corpses and do not answer.
pub fn swept(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| is_socket_name(&entry.file_name().to_string_lossy()))
        .filter(|entry| refused(&entry.path()))
        // Already gone is another balthasar sweeping the same file, not an error, and not counted.
        .filter(|entry| std::fs::remove_file(entry.path()).is_ok())
        .count()
}

/// Whether a name in the runtime directory is one of our sockets.
fn is_socket_name(name: &str) -> bool {
    name.starts_with("api@") && name.ends_with(".sock")
}

/// Whether connecting is refused outright, which is the only proof that nothing is bound.
///
/// `ECONNREFUSED` alone, never `is_ok()`: `EMFILE` is this process out of descriptors, and
/// deleting on it takes a live balthasar name.
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
        // A real socket file rather than a regular one: this is what a `SIGKILL` leaves.
        let bound = std::os::unix::net::UnixListener::bind(&corpse).expect("bind");
        drop(bound);
        assert!(corpse.exists(), "the kernel leaves the file behind");

        assert_eq!(swept(&dir), 1);
        assert!(!corpse.exists());
    }

    #[test]
    fn a_socket_something_is_listening_on_is_left_alone() {
        let dir = Scratch::new("balthasar-corpses", "live");
        let live = dir.join("api@live.sock");
        let _bound = std::os::unix::net::UnixListener::bind(&live).expect("bind");

        assert_eq!(swept(&dir), 0);
        assert!(live.exists());
    }

    #[test]
    fn a_socket_that_cannot_be_dialled_at_all_is_left_alone() {
        // The test above passes against `Err(_) => true` too. This one does not: mode 0 makes
        // `connect` return `EACCES`, which says nothing about whether the far end is alive.
        use std::os::unix::fs::PermissionsExt;
        let dir = Scratch::new("balthasar-corpses", "unreachable");
        let closed = dir.join("api@closed.sock");
        let _bound = std::os::unix::net::UnixListener::bind(&closed).expect("bind");
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        // Root ignores the mode, and would prove nothing either way.
        if std::os::unix::net::UnixStream::connect(&closed).is_ok() {
            return;
        }

        assert_eq!(swept(&dir), 0);
        assert!(closed.exists(), "an undiagnosed socket is not a corpse");
    }

    #[test]
    fn what_is_not_a_socket_of_ours_is_not_touched() {
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

    #[test]
    fn binding_does_not_sweep_the_neighbours() {
        // The sweep dials, and a dial takes one of the eight seats on whatever answers it.
        let instance = format!("neighbour-{}", std::process::id());
        let corpse = crate::socket_dir().join(format!("api@corpse-{}.sock", std::process::id()));
        std::fs::create_dir_all(crate::socket_dir()).expect("mkdir");
        let bound = std::os::unix::net::UnixListener::bind(&corpse).expect("bind");
        drop(bound);

        let listener = crate::Listener::bind(&instance).expect("bind");
        assert!(corpse.exists(), "binding swept a name that was not its own");

        drop(listener);
        std::fs::remove_file(&corpse).expect("clean up");
    }
}
