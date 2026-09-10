//! Listening, and telling other programs where to find us.

use crate::{Peer, Reply, Request, frame, stop};
use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// The role this program fills, and so the name its sockets live under — see `ROLES.md`.
const ROLE: &str = "memory";

/// The program's own name, which the socket directory used to be called.
const PROGRAM: &str = "balthasar";

/// Where the memory role binds its sockets.
///
/// `$XDG_RUNTIME_DIR/memory`, falling back to a per-user directory in the temporary one.
#[must_use]
pub fn socket_dir() -> PathBuf {
    named_dir(ROLE)
}

/// Where the memory role bound its sockets when the directory was the program's name.
///
/// Still bound, for one release: a caller that has not been rebuilt looks here and nowhere else.
#[must_use]
pub fn legacy_socket_dir() -> PathBuf {
    named_dir(PROGRAM)
}

/// Both directories, the role's own first.
#[must_use]
pub fn socket_dirs() -> [PathBuf; 2] {
    [socket_dir(), legacy_socket_dir()]
}

/// One socket directory, under the runtime directory when there is one.
fn named_dir(name: &str) -> PathBuf {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(runtime).join(name);
    }
    let uid = rustix::process::getuid().as_raw();
    std::env::temp_dir().join(format!("{name}-{uid}"))
}

/// The socket one named instance listens on.
#[must_use]
pub fn socket_path(instance: &str) -> PathBuf {
    socket_dir().join(format!("api@{instance}.sock"))
}

/// The socket that same instance also listens on, under the old name.
#[must_use]
pub fn legacy_socket_path(instance: &str) -> PathBuf {
    legacy_socket_dir().join(format!("api@{instance}.sock"))
}

/// Write the descriptor a caller with no socket uses to spawn us, and say where the first went.
///
/// The path written is balthasar's own, absolute — never a name resolved through `$PATH`. Into
/// both directories: it is looked up under the program's name, so the old one has to keep working.
pub fn tool_descriptor() -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let body = serde_json::json!({
        "exec": exe.to_string_lossy(),
        "args": ["api"],
        "version": 1,
    });
    let mut written = None;
    let mut refused = None;
    for dir in socket_dirs() {
        let path = dir.join(format!("{PROGRAM}.tool"));
        match std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, body.to_string())) {
            Ok(()) => written = written.or(Some(path)),
            Err(why) => refused = refused.or(Some(why)),
        }
    }
    written.ok_or_else(|| refused.unwrap_or_else(|| std::io::Error::other("nowhere to write")))
}

/// How long a read waits before looking up.
///
/// Not a deadline on the caller: a wait that expires is asked again.
const QUIET: std::time::Duration = std::time::Duration::from_secs(30);

/// How many callers may be connected at once.
///
/// A thread per connection with no ceiling is a file-descriptor exhaustion away from the store.
const CALLERS: usize = 8;

/// How many callers are connected right now.
type Crowd = std::sync::Arc<std::sync::atomic::AtomicUsize>;

/// One connection's place in the crowd, given back when its thread ends.
struct Seat(Crowd);

impl Seat {
    fn take(crowd: &Crowd) -> Option<Self> {
        crowd
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |taken| (taken < CALLERS).then_some(taken + 1),
            )
            .ok()
            .map(|_| Self(std::sync::Arc::clone(crowd)))
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// One call on its way to the one thread that answers, and the way back.
struct Asked {
    peer: Peer,
    request: Request,
    answered: std::sync::mpsc::Sender<Reply>,
}

/// What the answering thread wakes up for.
///
/// A stop travels the same queue as a call, so the loop can only leave between two calls and no
/// SQLite transaction is ever half-written on the way out.
enum Heard {
    Call(Asked),
    Stop(&'static str),
}

/// The sockets one instance is bound to, all of them reaching the one store behind them.
pub struct Listener {
    bound: Vec<(UnixListener, PathBuf)>,
}

impl Listener {
    /// Bind, replacing a socket left behind by something that is no longer running.
    ///
    /// A stale socket file looks exactly like a live one until something connects, so binding
    /// tries a connection first: if anything answers, this instance refuses rather than
    /// stealing the name.
    ///
    /// Two names for one process, so a caller looking under either finds *this* store. Both are
    /// checked before either is taken: an instance name is one daemon across both directories, or
    /// the compatibility window is itself a way to fork the store. The role's own name must bind;
    /// the old one is a courtesy, and [`Listener::paths`] comes back short when it failed.
    pub fn bind(instance: &str) -> std::io::Result<Self> {
        let wanted = [socket_path(instance), legacy_socket_path(instance)];
        for path in &wanted {
            if live(path) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("an balthasar is already listening on {}", path.display()),
                ));
            }
        }

        let mut bound = Vec::new();
        for path in wanted {
            match claim(&path) {
                Ok(listener) => bound.push((listener, path)),
                Err(why) if bound.is_empty() => return Err(why),
                Err(_) => {}
            }
        }
        Ok(Self { bound })
    }

    /// Where it is listening: the role's own name.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.bound[0].1
    }

    /// Every name it is listening under, the role's own first.
    #[must_use]
    pub fn paths(&self) -> Vec<&Path> {
        self.bound.iter().map(|(_, path)| path.as_path()).collect()
    }

    /// Serve until something stops us.
    ///
    /// Each connection gets a thread of its own to be read on, and every call those threads read
    /// is handed to this thread, which answers them one at a time.
    ///
    /// An expired read timeout means ask again. A connection ends on end-of-file —
    /// [`frame::WireError::Closed`] — not on quiet.
    ///
    /// A stop signal ends this the way running out of callers would, when the caller called
    /// [`crate::hold_stop_signals`] first, so that [`Listener::drop`] and whatever the answering
    /// closure holds still run on the way out.
    ///
    /// # Errors
    /// When the listener cannot be handed to the thread that accepts on it.
    pub fn serve(&self, mut answer: impl FnMut(&Peer, Request) -> Reply) -> std::io::Result<()> {
        // Duplicates rather than borrows, so a panic out of `answer` unwinds instead of joining.
        let mut listening = Vec::with_capacity(self.bound.len());
        for (bound, _) in &self.bound {
            listening.push(bound.try_clone()?);
        }
        let (asked, asking) = std::sync::mpsc::channel::<Heard>();
        if stop::holding() {
            // Its own sender: a balthasar nobody connects to still has to hear it is time to go.
            let stopping = asked.clone();
            std::thread::spawn(move || {
                if let Some(named) = stop::awaited() {
                    let _ = stopping.send(Heard::Stop(named));
                }
            });
        }
        // One accept thread per name, all feeding the queue the one answering thread reads: two
        // doors into one store, never two stores.
        for one in listening {
            let asked = asked.clone();
            std::thread::spawn(move || accept(&one, &asked));
        }
        drop(asked);

        while let Ok(heard) = asking.recv() {
            match heard {
                Heard::Call(Asked {
                    peer,
                    request,
                    answered,
                }) => {
                    let _ = answered.send(answer(&peer, request));
                }
                Heard::Stop(named) => {
                    eprintln!("balthasar: stopped on {named}");
                    break;
                }
            }
        }
        Ok(())
    }
}

/// Take callers, one thread each, and pass what they say to whoever is answering.
///
/// Identity is settled per connection before a byte is read, and a caller has no say in it.
fn accept(listening: &UnixListener, asked: &std::sync::mpsc::Sender<Heard>) {
    let crowd: Crowd = Crowd::default();

    for incoming in listening.incoming() {
        let Ok(mut stream) = incoming else { continue };

        let Some(peer) = Peer::of(&stream) else {
            // A caller the kernel will not identify cannot be held to a write ceiling.
            let _ = refuse(&mut stream, "balthasar cannot identify this caller");
            continue;
        };
        if !peer.is_owner() {
            let _ = refuse(&mut stream, "this store belongs to somebody else");
            continue;
        }
        // Refused out loud: this socket has a client that answers silence by deleting it.
        let Some(seat) = Seat::take(&crowd) else {
            let _ = refuse(&mut stream, "balthasar is holding all the callers it can");
            continue;
        };

        // A connection with no read timeout never comes up for air; its seat never comes back.
        if stream.set_read_timeout(Some(QUIET)).is_err() {
            let _ = refuse(
                &mut stream,
                "balthasar could not make this connection interruptible",
            );
            continue;
        }
        let asked = asked.clone();
        std::thread::spawn(move || {
            let _seat = seat;
            converse(&mut stream, &peer, &|peer: &Peer, request: Request| {
                answering(&asked, peer, request)
            });
        });
    }
}

/// Hand one call to the answering thread and wait for what it says.
///
/// A refusal rather than a dropped connection when there is nobody left to ask: "no answer" and
/// "the far end has gone" look identical to a client, and one of them is worth retrying.
fn answering(asked: &std::sync::mpsc::Sender<Heard>, peer: &Peer, request: Request) -> Reply {
    let (answered, hearing) = std::sync::mpsc::channel();
    let sent = asked.send(Heard::Call(Asked {
        peer: peer.clone(),
        request,
        answered,
    }));
    match sent.ok().and_then(|()| hearing.recv().ok()) {
        Some(reply) => reply,
        None => Reply::refused("balthasar has stopped answering"),
    }
}

/// Answer one connection's calls until it closes.
///
/// `answer` is a way to ask the one thread that answers, taken per call rather than per
/// connection, so that holding a connection holds nothing.
fn converse(stream: &mut UnixStream, peer: &Peer, answer: &impl Fn(&Peer, Request) -> Reply) {
    loop {
        let body = match frame::recv(stream) {
            Ok(body) => body,
            // Only a wait that began at a frame boundary is resumable: one that expired mid-frame
            // has lost bytes the next read would misread as a header.
            Err(frame::WireError::Idle) => continue,
            Err(_) => break,
        };
        // Answered in the encoding it was asked in, read from the body's first byte.
        let wire = crate::Wire::of(&body);
        let reply = match wire.read::<Request>(&body) {
            Ok(request) => answer(peer, request),
            Err(why) => Reply::refused(format!("that is not a request: {why}")),
        };
        let Ok(encoded) = wire.write(&reply) else {
            break;
        };
        if frame::send(stream, &encoded).is_err() {
            break;
        }
    }
}

impl Drop for Listener {
    /// Take every socket file with us; one left behind is one the next instance has to disprove.
    fn drop(&mut self) {
        for (_, path) in &self.bound {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Whether something is listening on `path` right now.
///
/// A connection rather than a stat: a socket file outlives the process that bound it, and the two
/// look identical until something dials.
fn live(path: &Path) -> bool {
    path.exists() && UnixStream::connect(path).is_ok()
}

/// Take one name, clearing a file left behind by something that is no longer running.
fn claim(path: &Path) -> std::io::Result<UnixListener> {
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    UnixListener::bind(path)
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
    fn a_socket_lives_under_the_roles_name() {
        let path = socket_path("default");
        assert!(
            path.to_string_lossy().contains("/memory/"),
            "{}",
            path.display()
        );
        assert!(path.to_string_lossy().ends_with("api@default.sock"));
    }

    #[test]
    fn the_name_it_used_to_live_under_is_still_a_place_to_look() {
        let path = legacy_socket_path("default");
        assert!(
            path.to_string_lossy().contains("/balthasar/"),
            "{}",
            path.display()
        );
        assert_eq!(socket_dirs(), [socket_dir(), legacy_socket_dir()]);
        assert_ne!(socket_dir(), legacy_socket_dir());
    }

    #[test]
    fn a_connection_is_answered_in_whatever_it_was_asked_in() {
        use std::io::{Read, Write};

        for wire in [crate::Wire::Json, crate::Wire::Cbor] {
            let (mut client, mut server) = UnixStream::pair().expect("pair");
            let peer = Peer::of(&server).expect("the kernel names us");

            let serving = std::thread::spawn(move || {
                let answer = |_p: &Peer, request: Request| {
                    Reply::one(serde_json::json!(format!("heard {}", request.call)))
                };
                converse(&mut server, &peer, &answer);
            });

            let asked = wire
                .write(&serde_json::json!({ "call": "verbs", "args": [] }))
                .expect("encode");
            let mut framed = (asked.len() as u32).to_be_bytes().to_vec();
            framed.extend_from_slice(&asked);
            client.write_all(&framed).expect("write");

            let mut header = [0_u8; 4];
            client.read_exact(&mut header).expect("header");
            let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
            client.read_exact(&mut body).expect("body");

            assert_eq!(crate::Wire::of(&body), wire, "answered in another encoding");
            let reply: serde_json::Value = wire.read(&body).expect("decode");
            assert_eq!(reply["ok"], serde_json::json!(true));
            assert_eq!(reply["result"][0], serde_json::json!("heard verbs"));

            drop(client);
            serving.join().expect("the server thread ended");
        }
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
    fn a_listener_takes_every_socket_it_bound_with_it() {
        let instance = format!("gone-{}", std::process::id());
        let paths = {
            let listener = Listener::bind(&instance).expect("bind");
            let paths: Vec<PathBuf> = listener.paths().into_iter().map(Path::to_owned).collect();
            assert_eq!(paths.len(), 2, "{paths:?}");
            for path in &paths {
                assert!(path.exists(), "{} was never bound", path.display());
            }
            paths
        };
        for path in paths {
            assert!(
                !path.exists(),
                "{} left behind is one the next instance must disprove",
                path.display()
            );
        }
    }

    #[test]
    fn the_old_name_being_taken_refuses_the_instance_outright() {
        // The fork this window exists to prevent: a daemon on one name and a fresh one on the
        // other, both answering, each holding a store the other's callers never see.
        let instance = format!("taken-{}", std::process::id());
        let old = legacy_socket_path(&instance);
        std::fs::create_dir_all(old.parent().expect("a parent")).expect("mkdir");
        let squatter = UnixListener::bind(&old).expect("bind the old name");

        let refused = Listener::bind(&instance);
        assert!(
            refused.is_err(),
            "the new name was free and the old one was live"
        );

        drop(squatter);
        std::fs::remove_file(&old).expect("clean up");
    }

    #[test]
    fn both_names_reach_the_same_answering_thread() {
        let instance = format!("both-{}", std::process::id());
        let listener = Listener::bind(&instance).expect("bind");
        let paths: Vec<PathBuf> = listener.paths().into_iter().map(Path::to_owned).collect();
        let counted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let tally = std::sync::Arc::clone(&counted);

        let serving = std::thread::spawn(move || {
            let _ = listener.serve(move |_peer: &Peer, _request: Request| {
                let n = tally.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
                Reply::one(serde_json::json!(n))
            });
        });

        for path in &paths {
            let mut stream = UnixStream::connect(path).expect("connect");
            let asked =
                serde_json::to_vec(&serde_json::json!({ "call": "verbs", "args": [] })).expect("e");
            let mut framed = (asked.len() as u32).to_be_bytes().to_vec();
            framed.extend_from_slice(&asked);
            use std::io::Read;
            stream.write_all(&framed).expect("write");
            let mut header = [0_u8; 4];
            stream.read_exact(&mut header).expect("header");
            let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
            stream.read_exact(&mut body).expect("body");
        }

        // One counter, so both doors were served by one thread over one state.
        assert_eq!(counted.load(std::sync::atomic::Ordering::Acquire), 2);
        for path in &paths {
            let _ = std::fs::remove_file(path);
        }
        drop(serving);
    }
}

/// A caller that holds its connection must not be the only one that can have it.
#[cfg(test)]
mod crowd {
    use super::*;
    use std::io::{Read, Write};

    /// A served socket whose file goes when the test ends, however the test ends: the accept loop
    /// never returns, so the [`Listener`] moved into the serving thread never drops.
    struct Bound(Vec<PathBuf>);

    impl Bound {
        /// Bind, and serve every call by echoing back what was asked.
        fn serving(instance: &str) -> Self {
            let listener = Listener::bind(instance).expect("bind");
            let paths = listener.paths().into_iter().map(Path::to_owned).collect();
            std::thread::spawn(move || {
                let _ = listener.serve(|_peer: &Peer, request: Request| {
                    Reply::one(serde_json::json!(format!("heard {}", request.call)))
                });
            });
            Self(paths)
        }

        /// The name a caller in these tests dials.
        fn path(&self) -> &Path {
            &self.0[0]
        }
    }

    impl Drop for Bound {
        fn drop(&mut self) {
            for path in &self.0 {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    fn say(stream: &mut UnixStream, call: &str) -> serde_json::Value {
        let body =
            serde_json::to_vec(&serde_json::json!({ "call": call, "args": [] })).expect("encode");
        let mut framed = (body.len() as u32).to_be_bytes().to_vec();
        framed.extend_from_slice(&body);
        stream.write_all(&framed).expect("write");

        let mut header = [0_u8; 4];
        stream.read_exact(&mut header).expect("header");
        let mut answer = vec![0_u8; u32::from_be_bytes(header) as usize];
        stream.read_exact(&mut answer).expect("body");
        serde_json::from_slice(&answer).expect("decode")
    }

    #[test]
    fn one_caller_holding_its_connection_does_not_shut_the_next_one_out() {
        let bound = Bound::serving(&format!("crowd-{}", std::process::id()));
        let path = bound.path();

        let mut scribe = UnixStream::connect(path).expect("the scribe connects");
        assert_eq!(
            say(&mut scribe, "remember")["result"][0],
            serde_json::json!("heard remember")
        );

        let mut probe = UnixStream::connect(path).expect("the child's probe connects");
        probe
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("a deadline");
        let answered = say(&mut probe, "verbs");
        assert_eq!(answered["ok"], serde_json::json!(true));
        assert_eq!(answered["result"][0], serde_json::json!("heard verbs"));

        assert_eq!(
            say(&mut scribe, "keep")["result"][0],
            serde_json::json!("heard keep")
        );

        drop(probe);
        drop(scribe);
    }

    #[test]
    fn the_caller_over_the_ceiling_is_told_so_rather_than_left_hanging() {
        // A refusal is a frame; a dropped connection is what gets the socket unlinked.
        let bound = Bound::serving(&format!("ceiling-{}", std::process::id()));
        let path = bound.path();

        let mut held: Vec<UnixStream> = Vec::new();
        for _ in 0..CALLERS {
            let mut caller = UnixStream::connect(path).expect("connect");
            assert_eq!(
                say(&mut caller, "verbs")["result"][0],
                serde_json::json!("heard verbs")
            );
            held.push(caller);
        }

        let mut over = UnixStream::connect(path).expect("connect");
        over.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("a deadline");
        let mut header = [0_u8; 4];
        over.read_exact(&mut header)
            .expect("a refusal, not silence");
        let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
        over.read_exact(&mut body).expect("the refusal's body");
        let told: serde_json::Value = serde_json::from_slice(&body).expect("decode");
        assert_eq!(told["ok"], serde_json::json!(false));

        // The seat comes back when the reading thread notices the close, a moment after it.
        drop(held.pop());
        let mut taken = None;
        for _ in 0..100 {
            let mut next = UnixStream::connect(path).expect("connect");
            next.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .expect("a deadline");
            if say(&mut next, "verbs")["ok"] == serde_json::json!(true) {
                taken = Some(next);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(taken.is_some(), "a seat given back was never taken");

        drop(taken);
        drop(held);
    }
}
