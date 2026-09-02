//! Listening, and telling other programs where to find us.

use crate::{Peer, Reply, Request, frame};
use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// Where balthasar binds its sockets.
///
/// `$XDG_RUNTIME_DIR/balthasar`, falling back to a per-user directory in the temporary one. Named
/// for the tool rather than for whoever is asking, because a descriptor has to be findable by
/// the sibling looking for it and not only by its author.
#[must_use]
pub fn socket_dir() -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(runtime).join("balthasar");
    }
    let uid = rustix::process::getuid().as_raw();
    std::env::temp_dir().join(format!("balthasar-{uid}"))
}

/// The socket one named instance listens on.
#[must_use]
pub fn socket_path(instance: &str) -> PathBuf {
    socket_dir().join(format!("api@{instance}.sock"))
}

/// Write the descriptor a caller with no socket uses to spawn us.
///
/// balthasar's state is SQLite on disk, outside the process, which is exactly the condition the
/// family names for a spawnable tool: a fresh process knows everything the running one does.
/// The path written is balthasar's own, absolute — never a name resolved through `$PATH`. Executing
/// whatever answers to a name, on the failure path where nothing was listening, is not a risk
/// worth taking.
pub fn tool_descriptor() -> std::io::Result<PathBuf> {
    let dir = socket_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("balthasar.tool");
    let exe = std::env::current_exe()?;
    let body = serde_json::json!({
        "exec": exe.to_string_lossy(),
        "args": ["api"],
        "version": 1,
    });
    std::fs::write(&path, body.to_string())?;
    Ok(path)
}

/// A bound socket.
pub struct Listener {
    listener: UnixListener,
    path: PathBuf,
}

impl Listener {
    /// Bind, replacing a socket left behind by something that is no longer running.
    ///
    /// A stale socket file looks exactly like a live one until something connects, so binding
    /// tries a connection first: if anything answers, this instance refuses rather than
    /// stealing the name.
    pub fn bind(instance: &str) -> std::io::Result<Self> {
        let path = socket_path(instance);
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;

        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("an balthasar is already listening on {}", path.display()),
                ));
            }
            std::fs::remove_file(&path)?;
        }

        let listener = UnixListener::bind(&path)?;
        Ok(Self { listener, path })
    }

    /// Where it is listening.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Serve until something stops us.
    ///
    /// One connection at a time, and each connection may carry many calls. balthasar is asked
    /// several times per turn, unlike a control socket that is polled occasionally, so the
    /// handle is held rather than reconnected — the sibling shape, and the reason to prefer it.
    pub fn serve(&self, mut answer: impl FnMut(&Peer, Request) -> Reply) -> std::io::Result<()> {
        for incoming in self.listener.incoming() {
            let Ok(mut stream) = incoming else { continue };

            let Some(peer) = Peer::of(&stream) else {
                // A caller the kernel will not identify gets nothing. There is no safe way to
                // apply a write ceiling to somebody who cannot be told apart from anybody.
                let _ = refuse(&mut stream, "balthasar cannot identify this caller");
                continue;
            };
            if !peer.is_owner() {
                let _ = refuse(&mut stream, "this store belongs to somebody else");
                continue;
            }

            // Bounded, so a peer that connects and says nothing does not hold the loop.
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
            loop {
                let Ok(body) = frame::recv(&mut stream) else {
                    break;
                };
                let reply = match serde_json::from_slice::<Request>(&body) {
                    Ok(request) => answer(&peer, request),
                    Err(why) => Reply::refused(format!("that is not a request: {why}")),
                };
                let Ok(encoded) = serde_json::to_vec(&reply) else {
                    break;
                };
                if frame::send(&mut stream, &encoded).is_err() {
                    break;
                }
            }
        }
        Ok(())
    }
}

impl Drop for Listener {
    /// Take the socket file with us.
    ///
    /// A file left behind is one the next instance has to prove is dead before it can bind.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Answer a refusal and close.
fn refuse(stream: &mut UnixStream, why: &str) -> std::io::Result<()> {
    let reply = serde_json::to_vec(&Reply::refused(why)).unwrap_or_default();
    let _ = frame::send(stream, &reply);
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socket_lives_under_the_tools_own_name() {
        // A descriptor has to be findable by whoever is asking, not only by its author.
        let path = socket_path("default");
        assert!(
            path.to_string_lossy().contains("/balthasar/"),
            "{}",
            path.display()
        );
        assert!(path.to_string_lossy().ends_with("api@default.sock"));
    }

    #[test]
    fn binding_twice_refuses_rather_than_stealing_the_name() {
        let instance = format!("test-{}", std::process::id());
        let first = Listener::bind(&instance).expect("bind");
        let second = Listener::bind(&instance);
        assert!(
            second.is_err(),
            "the second must not steal the first's socket"
        );
        drop(first);
    }

    #[test]
    fn a_socket_left_behind_by_something_dead_is_replaced() {
        let instance = format!("stale-{}", std::process::id());
        let path = socket_path(&instance);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(&path, "not a socket").expect("leave something behind");
        let listener = Listener::bind(&instance).expect("a stale file is not a live peer");
        drop(listener);
    }

    #[test]
    fn a_listener_takes_its_socket_with_it() {
        let instance = format!("gone-{}", std::process::id());
        let path = {
            let listener = Listener::bind(&instance).expect("bind");
            listener.path().to_owned()
        };
        assert!(
            !path.exists(),
            "a file left behind is one the next instance must disprove"
        );
    }
}
