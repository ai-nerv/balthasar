//! Listening, and telling other programs where to find us.

use crate::{Peer, Reply, Request, frame, stop};
use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

mod scoped;
pub use scoped::ConnectionState;

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

/// One socket directory: `<runtime>/<name>` when the runtime directory is safe to use, otherwise
/// `<tmp>/<name>-<uid>`. See [`balthasar_model::runtime`].
fn named_dir(name: &str) -> PathBuf {
    balthasar_model::runtime::program_dir(name)
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
        let wrote = balthasar_model::runtime::private_dir(&dir)
            .and_then(|()| written_alone(&path, body.to_string().as_bytes()));
        match wrote {
            Ok(()) => written = written.or(Some(path)),
            Err(why) => refused = refused.or(Some(why)),
        }
    }
    written.ok_or_else(|| refused.unwrap_or_else(|| std::io::Error::other("nowhere to write")))
}

/// Write `body` to `path` 0600, replacing what is there without following a symlink.
fn written_alone(path: &Path, body: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut fresh = path.as_os_str().to_owned();
    fresh.push(format!(".{}.new", std::process::id()));
    let fresh = PathBuf::from(fresh);
    let wrote = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&fresh)
        .and_then(|mut file| file.write_all(body))
        .and_then(|()| std::fs::rename(&fresh, path));
    if wrote.is_err() {
        let _ = std::fs::remove_file(&fresh);
    }
    wrote
}

/// How long a read waits before looking up.
///
/// Not a deadline on the caller: a wait that expires is asked again, until [`Patience::idle`].
const QUIET: std::time::Duration = std::time::Duration::from_secs(30);

/// How long a connection is waited on, in the family's numbers.
#[derive(Debug, Clone, Copy)]
struct Patience {
    /// From a frame's first byte to its last.
    frame: std::time::Duration,
    /// With nothing said at all, before the connection is closed.
    idle: std::time::Duration,
    /// For one reply to be taken.
    write: std::time::Duration,
}

const PATIENCE: Patience = Patience {
    frame: std::time::Duration::from_secs(30),
    idle: std::time::Duration::from_secs(300),
    write: std::time::Duration::from_secs(30),
};

/// How long to pause after an accept that failed, before accepting again.
const BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);

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
    connection: u64,
    retained: std::sync::Arc<std::sync::atomic::AtomicBool>,
    peer: Peer,
    request: Request,
    context: Option<Box<serde_json::Value>>,
    answered: std::sync::mpsc::Sender<Reply>,
}

/// What the answering thread wakes up for.
///
/// A stop travels the same queue as a call, so the loop can only leave between two calls and no
/// SQLite transaction is ever half-written on the way out.
enum Heard {
    Call(Asked),
    Closed(u64),
    Stop(&'static str),
}

/// The sockets one instance is bound to, all of them reaching the one store behind them.
pub struct Listener {
    /// The first name bound, the role's own.
    path: PathBuf,
    bound: Vec<crate::stale::Claimed>,
    /// The `<socket>.key` files this wrote, removed when it is dropped.
    published: Vec<PathBuf>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        for path in &self.published {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Listener {
    /// Bind, replacing only a socket nothing answers on.
    ///
    /// Each name is taken under its `.lock`, and an existing socket is cleared only when connecting
    /// to it is refused outright. Two names for one process, so a caller
    /// looking under either finds *this* store: the role's own name must bind, the old one is a
    /// courtesy, and a live program on the old one refuses the instance outright.
    pub fn bind(instance: &str) -> std::io::Result<Self> {
        Self::bind_in(&socket_dirs(), instance)
    }

    /// The same, in directories the caller names, the role's own first: what a test binds in, so
    /// nothing it leaves reaches the runtime directory every balthasar on the machine shares.
    pub fn bind_in(dirs: &[PathBuf], instance: &str) -> std::io::Result<Self> {
        let mut bound = Vec::new();
        for dir in dirs {
            let path = dir.join(format!("api@{instance}.sock"));
            let taken = balthasar_model::runtime::private_dir(dir)
                .and_then(|()| crate::stale::claim(&path));
            match taken {
                Ok(claimed) => bound.push(claimed),
                Err(why) if bound.is_empty() || why.kind() == std::io::ErrorKind::AddrInUse => {
                    return Err(why);
                }
                Err(_) => {}
            }
        }
        let path = bound
            .first()
            .map(|first| first.path.clone())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "no directory to bind in")
            })?;
        Ok(Self {
            path,
            bound,
            published: Vec::new(),
        })
    }

    /// Where it is listening: the role's own name.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every name it is listening under, the role's own first.
    #[must_use]
    pub fn paths(&self) -> Vec<&Path> {
        self.bound
            .iter()
            .map(|claimed| claimed.path.as_path())
            .collect()
    }

    /// Write `key` to `<socket>.key` beside every name bound: created 0600 and new, inside the
    /// private directory the socket is in, so a caller this did not start can read it. One left
    /// by an instance that died is replaced, under the lock this holds on the name.
    ///
    /// # Errors
    /// When a file cannot be written; any written before it are removed with the listener.
    pub fn publish(&mut self, key: &crate::Key) -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        for claimed in &self.bound {
            let mut path = claimed.path.as_os_str().to_owned();
            path.push(".key");
            let path = PathBuf::from(path);
            match std::fs::remove_file(&path) {
                Err(why) if why.kind() != std::io::ErrorKind::NotFound => return Err(why),
                _ => {}
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            self.published.push(path);
            file.write_all(format!("{}\n", key.hex()).as_bytes())?;
        }
        Ok(())
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
    /// [`crate::hold_stop_signals`] first, so that the sockets are removed and whatever the answering
    /// closure holds still run on the way out.
    ///
    /// A call whose verb `guard` names reaches `answer` only carrying its key; without it, it is
    /// refused on its connection.
    ///
    /// # Errors
    /// When the listener cannot be handed to the thread that accepts on it.
    pub fn serve(
        &self,
        guard: crate::Guard,
        mut answer: impl FnMut(&Peer, Request) -> Reply,
    ) -> std::io::Result<()> {
        self.serve_scoped::<()>(guard, |peer, (), request| answer(peer, request))
    }
}

/// Take callers, one thread each, and pass what they say to whoever is answering.
///
/// Identity is settled per connection before a byte is read, and a caller has no say in it.
fn accept(
    listening: &UnixListener,
    asked: &std::sync::mpsc::Sender<Heard>,
    guard: &std::sync::Arc<crate::Guard>,
    identities: &std::sync::atomic::AtomicU64,
) {
    let crowd: Crowd = Crowd::default();

    for incoming in listening.incoming() {
        let mut stream = match incoming {
            Ok(stream) => stream,
            // Out of descriptors or buffers is a moment, not an ending: say so, pause, carry on.
            Err(why) => {
                eprintln!("balthasar: accept failed: {why}; trying again shortly");
                std::thread::sleep(BACKOFF);
                continue;
            }
        };
        // Every reply, refusals included, is written under a deadline.
        if stream.set_write_timeout(Some(PATIENCE.write)).is_err() {
            continue;
        }

        let Some(peer) = Peer::of(&stream) else {
            // A caller the kernel will not identify cannot be held to a write ceiling.
            let _ = refuse(&mut stream, "balthasar cannot identify this caller");
            continue;
        };
        if !peer.is_owner() {
            let _ = refuse(&mut stream, "this store belongs to somebody else");
            continue;
        }
        let Some(connection) = scoped::Connection::new(asked.clone(), identities) else {
            let _ = refuse(&mut stream, "balthasar connection identities exhausted");
            continue;
        };
        // Refused out loud: this socket has a client that answers silence by deleting it.
        let Some(seat) = Seat::take(&crowd) else {
            let busy = Reply::failed("busy: balthasar is holding all the callers it can");
            let _ = say_plain(&mut stream, crate::Wire::Json, &busy);
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
        let guard = std::sync::Arc::clone(guard);
        std::thread::spawn(move || {
            let _seat = seat;
            let connection = connection;
            connection.converse(&mut stream, &peer, &guard, PATIENCE);
            drop(connection);
        });
    }
}

/// Hand one call to the answering thread and wait for what it says.
///
/// A refusal rather than a dropped connection when there is nobody left to ask: "no answer" and
/// "the far end has gone" look identical to a client, and one of them is worth retrying.
fn answering(
    asked: &std::sync::mpsc::Sender<Heard>,
    connection: u64,
    retained: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    peer: &Peer,
    request: Request,
    context: Option<serde_json::Value>,
) -> Reply {
    let (answered, hearing) = std::sync::mpsc::channel();
    let sent = asked.send(Heard::Call(Asked {
        connection,
        retained: std::sync::Arc::clone(retained),
        peer: peer.clone(),
        request,
        context: context.map(Box::new),
        answered,
    }));
    match sent.ok().and_then(|()| hearing.recv().ok()) {
        Some(reply) => reply,
        None => Reply::refused("balthasar has stopped answering"),
    }
}

/// Answer one connection's calls until it closes, goes quiet for too long, or stalls mid-frame.
///
/// Calls rejected by `guard` never reach the answering thread.
#[cfg(test)]
fn converse(
    stream: &mut UnixStream,
    peer: &Peer,
    guard: &crate::Guard,
    answer: &impl Fn(&Peer, Request) -> Reply,
) {
    converse_within(stream, peer, guard, answer, PATIENCE);
}

#[cfg(test)]
fn converse_within(
    stream: &mut UnixStream,
    peer: &Peer,
    guard: &crate::Guard,
    answer: &impl Fn(&Peer, Request) -> Reply,
    patience: Patience,
) {
    converse_retained(
        stream,
        peer,
        guard,
        &|peer, request, _| answer(peer, request),
        patience,
        &std::sync::atomic::AtomicBool::new(false),
    );
}

fn converse_retained(
    stream: &mut UnixStream,
    peer: &Peer,
    guard: &crate::Guard,
    answer: &impl Fn(&Peer, Request, Option<serde_json::Value>) -> Reply,
    patience: Patience,
    retained: &std::sync::atomic::AtomicBool,
) {
    let mut heard = std::time::Instant::now();
    loop {
        let body = match frame::recv_within(stream, Some(patience.frame)) {
            Ok(body) => body,
            // Only a wait that began at a frame boundary is resumable: one that expired mid-frame
            // has lost bytes the next read would misread as a header.
            Err(frame::WireError::Idle)
                if heard.elapsed() < patience.idle
                    || retained.load(std::sync::atomic::Ordering::Acquire) =>
            {
                continue;
            }
            // One refusal, then close: a body that is not read cannot be skipped in step.
            Err(why @ frame::WireError::TooLarge(_)) => {
                let _ = say_plain(stream, crate::Wire::Json, &Reply::failed(why.to_string()));
                break;
            }
            Err(_) => break,
        };
        // Answered in the encoding it was asked in, read from the body's first byte.
        let wire = crate::Wire::of(&body);
        let mut revision = None;
        let reply = if body.is_empty() {
            Reply::refused("that is not a request: a frame of zero length is malformed")
        } else {
            crate::key::admitted(guard, wire, &body).map_or_else(
                |refusal| refusal,
                |(request, context)| {
                    if request.call == "verbs" {
                        revision = guard.request_context;
                    }
                    answer(peer, request, context)
                },
            )
        };
        if say(stream, wire, &reply, revision).is_err() {
            break;
        }
        heard = std::time::Instant::now();
    }
}

/// Write one reply. One that would be over the cap is answered with a refusal naming it instead,
/// so the caller can tell "too large" from a closed connection and stays in step.
fn say_plain(
    stream: &mut UnixStream,
    wire: crate::Wire,
    reply: &Reply,
) -> Result<(), frame::WireError> {
    say(stream, wire, reply, None)
}

fn say(
    stream: &mut UnixStream,
    wire: crate::Wire,
    reply: &Reply,
    revision: Option<u8>,
) -> Result<(), frame::WireError> {
    #[derive(serde::Serialize)]
    struct Envelope<'a> {
        #[serde(flatten)]
        reply: &'a Reply,
        #[serde(skip_serializing_if = "Option::is_none")]
        request_context: Option<u8>,
    }
    let mut encoded = wire
        .write(&Envelope {
            reply,
            request_context: revision.filter(|_| reply.ok),
        })
        .map_err(frame::WireError::Malformed)?;
    if encoded.len() > frame::MAX_FRAME {
        let instead = Reply::failed(format!(
            "the reply would be {} bytes, over the {}-byte frame cap",
            encoded.len(),
            frame::MAX_FRAME
        ));
        encoded = wire.write(&instead).map_err(frame::WireError::Malformed)?;
    }
    frame::send(stream, &encoded)
}

/// Answer a refusal and close.
fn refuse(stream: &mut UnixStream, why: &str) -> std::io::Result<()> {
    let reply = serde_json::to_vec(&Reply::refused(why)).unwrap_or_default();
    let _ = frame::send(stream, &reply);
    stream.flush()
}

/// Socket directories of a test's own, the role's and the old name's: private, and removed with
/// what is left in them when the test ends, so no test binds in the shared runtime directory.
#[cfg(test)]
pub(crate) fn aside(name: &str) -> (balthasar_model::scratch::Scratch, [PathBuf; 2]) {
    let root = balthasar_model::scratch::Scratch::new("bs", name);
    let dirs = [root.join("m"), root.join("b")];
    for dir in &dirs {
        balthasar_model::runtime::private_dir(dir).expect("a private socket directory");
    }
    (root, dirs)
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

            let (guard, hex) = (crate::key::guard_for_tests(), crate::key::for_tests().1);
            let serving = std::thread::spawn(move || {
                let answer = |_p: &Peer, request: Request| {
                    Reply::one(serde_json::json!(format!("heard {}", request.call)))
                };
                converse(&mut server, &peer, &guard, &answer);
            });

            let asked = wire
                .write(&serde_json::json!({ "call": "verbs", "args": [], "key": hex }))
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
        let (_root, dirs) = aside("twice");
        let first = Listener::bind_in(&dirs, "t").expect("bind");
        let second = Listener::bind_in(&dirs, "t");
        assert!(
            second.is_err(),
            "the second must not steal the first's socket"
        );
        drop(first);
    }

    /// Every file left in a test's socket directories.
    fn left(dirs: &[PathBuf]) -> Vec<PathBuf> {
        dirs.iter()
            .flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten().flatten())
            .map(|entry| entry.path())
            .collect()
    }

    #[test]
    fn a_socket_left_behind_by_something_dead_is_replaced() {
        let (_root, dirs) = aside("stale");
        let path = dirs[0].join("api@s.sock");
        drop(UnixListener::bind(&path).expect("leave a corpse behind"));
        let listener = Listener::bind_in(&dirs, "s").expect("a corpse is not a live peer");
        drop(listener);
        assert_eq!(
            left(&dirs),
            Vec::<PathBuf>::new(),
            "sockets and locks go too"
        );
    }

    #[test]
    fn a_file_that_is_not_a_socket_is_not_cleared_to_make_room() {
        let (_root, dirs) = aside("squat");
        let path = dirs[0].join("api@q.sock");
        std::fs::write(&path, "not a socket").expect("leave something behind");
        assert!(Listener::bind_in(&dirs, "q").is_err());
        assert!(path.exists(), "it was not ours to remove");
        assert_eq!(
            left(&dirs),
            vec![path],
            "and the refusal leaves no lock behind"
        );
    }

    #[test]
    fn binding_in_no_directory_is_refused() {
        let refused = Listener::bind_in(&[], "none").err().map(|why| why.kind());
        assert_eq!(refused, Some(std::io::ErrorKind::InvalidInput));
    }

    #[test]
    fn a_listener_takes_every_socket_it_bound_with_it() {
        let (_root, dirs) = aside("gone");
        {
            let listener = Listener::bind_in(&dirs, "g").expect("bind");
            let paths: Vec<PathBuf> = listener.paths().into_iter().map(Path::to_owned).collect();
            assert_eq!(paths.len(), 2, "{paths:?}");
            for path in &paths {
                assert!(path.exists(), "{} was never bound", path.display());
            }
        }
        assert_eq!(
            left(&dirs),
            Vec::<PathBuf>::new(),
            "a socket or lock left behind is one the next instance must disprove"
        );
    }

    #[test]
    fn a_published_key_sits_beside_each_socket_for_its_owner_alone_and_goes_with_it() {
        use std::os::unix::fs::PermissionsExt;
        let (_root, dirs) = aside("published");
        let stale = dirs[0].join("api@k.sock.key");
        std::fs::write(&stale, "left by a dead instance\n").expect("a stale key");
        {
            let mut listener = Listener::bind_in(&dirs, "k").expect("bind");
            let key = crate::Key::mint().expect("mint");
            listener.publish(&key).expect("publish");
            for path in listener.paths() {
                let mut named = path.as_os_str().to_owned();
                named.push(".key");
                let file = PathBuf::from(named);
                let said = std::fs::read_to_string(&file).expect("published");
                assert_eq!(said, format!("{}\n", key.hex()));
                let mode = std::fs::metadata(&file).expect("stat").permissions().mode();
                assert_eq!(mode & 0o777, 0o600, "{}", file.display());
            }
        }
        assert_eq!(
            left(&dirs),
            Vec::<PathBuf>::new(),
            "a key file outlived the listener"
        );
    }

    #[test]
    fn a_lock_file_that_is_no_longer_ours_is_left_alone() {
        let (_root, dirs) = aside("relock");
        let listener = Listener::bind_in(&dirs, "r").expect("bind");
        let lock = crate::stale::lock_path(listener.path());
        std::fs::remove_file(&lock).expect("taken away");
        std::fs::write(&lock, "somebody else's").expect("replaced");
        drop(listener);
        assert!(
            lock.exists(),
            "a lock file this did not hold is not this one's to remove"
        );
    }

    #[test]
    fn the_old_name_being_taken_refuses_the_instance_outright() {
        // The fork this window exists to prevent: a daemon on one name and a fresh one on the
        // other, both answering, each holding a store the other's callers never see.
        let (_root, dirs) = aside("taken");
        let old = dirs[1].join("api@k.sock");
        let squatter = UnixListener::bind(&old).expect("bind the old name");

        let refused = Listener::bind_in(&dirs, "k");
        assert!(
            refused.is_err(),
            "the new name was free and the old one was live"
        );
        assert_eq!(
            left(&dirs),
            vec![old],
            "nothing of the refused instance is left"
        );
        drop(squatter);
    }

    #[test]
    fn both_names_reach_the_same_answering_thread() {
        let (_root, dirs) = aside("both");
        let listener = Listener::bind_in(&dirs, "o").expect("bind");
        let paths: Vec<PathBuf> = listener.paths().into_iter().map(Path::to_owned).collect();
        let counted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let tally = std::sync::Arc::clone(&counted);

        let (guard, hex) = (crate::key::guard_for_tests(), crate::key::for_tests().1);
        let serving = std::thread::spawn(move || {
            let _ = listener.serve(guard, move |_peer: &Peer, _request: Request| {
                let n = tally.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
                Reply::one(serde_json::json!(n))
            });
        });

        for path in &paths {
            let mut stream = UnixStream::connect(path).expect("connect");
            let call = serde_json::json!({ "call": "verbs", "args": [], "key": hex });
            let asked = serde_json::to_vec(&call).expect("e");
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
        drop(serving);
    }
}

#[cfg(test)]
mod crowd;

#[cfg(test)]
mod framing;

#[cfg(test)]
mod panics;
