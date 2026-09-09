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

/// How long a read waits before looking up.
///
/// Not a deadline on the caller. A wait that expires is asked again — see [`Listener::serve`] —
/// so this only decides how often a connection with nothing on it comes up for air, which is
/// what keeps the wait interruptible rather than what limits it.
const QUIET: std::time::Duration = std::time::Duration::from_secs(30);

/// How many callers may be connected at once.
///
/// Bounded because this socket sits in the runtime directory, reachable by anything running as
/// this user, and a thread per connection with no ceiling is a file-descriptor exhaustion away
/// from taking the store down with it. Eight is melchior's number, and for the same reason: a
/// harness, its scribe, and the handful of children one turn forks all fit, while a program that
/// connects in a loop does not get to keep going.
const CALLERS: usize = 8;

/// How many callers are connected right now.
type Crowd = std::sync::Arc<std::sync::atomic::AtomicUsize>;

/// One connection's place in the crowd, given back when its thread ends.
///
/// A counter rather than a real semaphore because the standard library has none, and because the
/// only question asked of it is whether there is room at the moment of the accept.
struct Seat(Crowd);

impl Seat {
    /// Take a place, or nothing when they are all taken.
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
    /// Each connection may carry many calls, and a caller holds one for as long as it is
    /// working: a harness is asked several times per turn, unlike a control socket that is
    /// polled occasionally, so the handle is held rather than reconnected.
    ///
    /// **Quiet is not gone.** A caller that has said nothing for a while is the ordinary state
    /// of a harness between turns — the pause while somebody reads the last answer and decides
    /// what to ask next is minutes, not seconds. There used to be a thirty-second read timeout
    /// here, so that a peer which connected and said nothing could not hold the only slot; but
    /// it could not tell a caller that is *idle* from one that is *gone*, and so it hung up on
    /// the ordinary one. The caller found its handle broken on the next thing it tried to
    /// record, which for a harness is the turn that just finished.
    ///
    /// The timeout is still set, and is what keeps the wait interruptible; what changed is that
    /// expiring means *ask again*. A peer that has really gone closes its end, and that arrives
    /// as end-of-file — [`frame::WireError::Closed`] — which is what ends a connection now.
    ///
    /// **Accepting is what runs at once; answering is still one at a time.** Holding a
    /// connection is correct and is what a harness does for a whole session, but this loop used
    /// to answer one connection to its end before returning to the accept, so a second caller
    /// was never accepted at all. That is the failure a forked child hit: its `verbs` probe went
    /// unanswered, it read the silence as a dead socket and *unlinked* it, and the parent's live
    /// balthasar was left with no name. Fan out three children and the first two are nameless.
    ///
    /// So a connection gets a thread of its own to be read on, and every call those threads read
    /// is handed to **this** thread, which answers them one at a time. Serialising the answers is
    /// wanted rather than tolerated — the stores are SQLite and one writer at a time is the
    /// honest arrangement — and what a caller gets back is its turn, not the whole daemon.
    ///
    /// One thread rather than a lock because of what `answer` holds. The daemon's closure owns a
    /// Lua engine and an embedder behind `Rc` and `Box<dyn Embed>`, neither of which may cross a
    /// thread at all, so a `Mutex` around it would need a `Send` bound the caller cannot satisfy.
    /// Keeping it where it was built asks nothing of it, and gives the same one-at-a-time.
    ///
    /// # Errors
    /// When the listener cannot be handed to the thread that accepts on it.
    pub fn serve(&self, mut answer: impl FnMut(&Peer, Request) -> Reply) -> std::io::Result<()> {
        // A duplicate rather than a borrow, so that accepting needs nothing of this call's
        // lifetime: a panic out of `answer` below then unwinds straight out, as it did when this
        // was one loop, instead of stopping to join a thread that never ends.
        let listening = self.listener.try_clone()?;
        let (asked, asking) = std::sync::mpsc::channel::<Asked>();
        std::thread::spawn(move || accept(&listening, &asked));

        // The accept thread keeps its end of this for as long as it runs, so the wait here is
        // what makes `serve` the call that does not return.
        while let Ok(Asked {
            peer,
            request,
            answered,
        }) = asking.recv()
        {
            let _ = answered.send(answer(&peer, request));
        }
        Ok(())
    }
}

/// Take callers, one thread each, and pass what they say to whoever is answering.
///
/// Identity is settled here, per connection, before a byte is read: the kernel's word about the
/// peer travels with every call it makes rather than being asked for again, and a caller has no
/// say in it.
fn accept(listening: &UnixListener, asked: &std::sync::mpsc::Sender<Asked>) {
    let crowd: Crowd = Crowd::default();

    for incoming in listening.incoming() {
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
        // Refused rather than queued, and refused *out loud*: this socket has a client that
        // answers silence by deleting it, so even a turned-away probe must leave knowing that
        // somebody is listening here.
        let Some(seat) = Seat::take(&crowd) else {
            let _ = refuse(&mut stream, "balthasar is holding all the callers it can");
            continue;
        };

        let _ = stream.set_read_timeout(Some(QUIET));
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
/// A refusal rather than a dropped connection when there is nobody left to ask, for the reason
/// the whole wire is written that way: "no answer" and "the far end has gone" look identical to
/// a client, and one of them is worth retrying.
fn answering(asked: &std::sync::mpsc::Sender<Asked>, peer: &Peer, request: Request) -> Reply {
    let (answered, hearing) = std::sync::mpsc::channel();
    let sent = asked.send(Asked {
        peer: peer.clone(),
        request,
        answered,
    });
    match sent.ok().and_then(|()| hearing.recv().ok()) {
        Some(reply) => reply,
        None => Reply::refused("balthasar has stopped answering"),
    }
}

/// Answer one connection's calls until it closes.
///
/// Split from the accept loop so that what happens *on* a connection can be tested without one:
/// [`Listener::serve`] never returns by design, so a test that drove it through a real socket
/// could set up the exchange and then had nothing to wait on.
///
/// Shared rather than owned, because several of these run at once now and what they share must
/// not: `answer` arrives as a way to *ask* the one thread that answers, taken per call rather
/// than per connection, so that holding a connection holds nothing.
fn converse(stream: &mut UnixStream, peer: &Peer, answer: &impl Fn(&Peer, Request) -> Reply) {
    loop {
        let body = match frame::recv(stream) {
            Ok(body) => body,
            // Nothing said, and nothing half-said: the peer is still there and has simply not
            // asked for anything yet. Only a wait that began at a frame boundary is resumable --
            // one that expired mid-frame has lost bytes the next read would misread as a header.
            Err(frame::WireError::Idle) => continue,
            Err(_) => break,
        };
        // Answered in the encoding it was asked in. Read from the body's first byte rather than
        // negotiated, so a caller that has never heard of CBOR is unaffected and one that has
        // needs to say nothing in advance.
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
    fn a_connection_is_answered_in_whatever_it_was_asked_in() {
        // Both encodings through the real connection loop, over a real socket. The property is
        // not that CBOR encodes -- `encoding` settles that -- but that a server nobody told
        // anything answers a CBOR caller in CBOR and a JSON caller in JSON, on one connection
        // type, with no flag and no handshake between them.
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

            // Closing our end is what ends the conversation, which is what lets this join.
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

/// A caller that holds its connection must not be the only one that can have it.
#[cfg(test)]
mod crowd {
    use super::*;
    use std::io::{Read, Write};

    /// Say one thing on an open connection and read the answer back.
    ///
    /// By hand rather than through a client, because what is being tested is the socket and a
    /// client that reconnected on its own would hide exactly the case that matters.
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
        // A harness connects once and holds the handle for the whole session, saying nothing
        // between turns. A serve loop that accepts one connection at a time is then never back
        // at the accept, so the second caller -- the probe a forked child sends before it
        // trusts the socket -- is never answered at all. The child reads that as a dead socket
        // and unlinks it, which leaves the parent's live daemon with no name.
        let instance = format!("crowd-{}", std::process::id());
        let listener = Listener::bind(&instance).expect("bind");
        let path = listener.path().to_owned();
        let _serving = std::thread::spawn(move || {
            let _ = listener.serve(|_peer: &Peer, request: Request| {
                Reply::one(serde_json::json!(format!("heard {}", request.call)))
            });
        });

        let mut scribe = UnixStream::connect(&path).expect("the scribe connects");
        // Asked and answered before the second one knocks, so the connection is provably the
        // one the server is sitting on rather than one still in the backlog.
        assert_eq!(
            say(&mut scribe, "remember")["result"][0],
            serde_json::json!("heard remember")
        );

        let mut probe = UnixStream::connect(&path).expect("the child's probe connects");
        // A deadline rather than a wait: the failure this is written for is a probe that is
        // never answered, and a test that hung on it would say nothing on the way past.
        probe
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("a deadline");
        let answered = say(&mut probe, "verbs");
        assert_eq!(answered["ok"], serde_json::json!(true));
        assert_eq!(answered["result"][0], serde_json::json!("heard verbs"));

        // The scribe is still on its own connection, and still answered.
        assert_eq!(
            say(&mut scribe, "keep")["result"][0],
            serde_json::json!("heard keep")
        );

        drop(probe);
        drop(scribe);
        // The accept loop never returns, so the thread outlives the test and its `Drop` never
        // runs. Unlinked here rather than left in the runtime directory to be disproved later.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_caller_over_the_ceiling_is_told_so_rather_than_left_hanging() {
        // The bound has to be visible from the wire, because the only client that matters here
        // treats silence as death. A refusal is a frame; a dropped connection is the thing that
        // gets the socket unlinked.
        let instance = format!("ceiling-{}", std::process::id());
        let listener = Listener::bind(&instance).expect("bind");
        let path = listener.path().to_owned();
        let _serving = std::thread::spawn(move || {
            let _ = listener.serve(|_peer: &Peer, request: Request| {
                Reply::one(serde_json::json!(format!("heard {}", request.call)))
            });
        });

        // Held, and each one answered first so that its seat is provably taken rather than
        // still sitting in the backlog.
        let mut held: Vec<UnixStream> = Vec::new();
        for _ in 0..CALLERS {
            let mut caller = UnixStream::connect(&path).expect("connect");
            assert_eq!(
                say(&mut caller, "verbs")["result"][0],
                serde_json::json!("heard verbs")
            );
            held.push(caller);
        }

        let mut over = UnixStream::connect(&path).expect("connect");
        over.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("a deadline");
        let mut header = [0_u8; 4];
        over.read_exact(&mut header)
            .expect("a refusal, not silence");
        let mut body = vec![0_u8; u32::from_be_bytes(header) as usize];
        over.read_exact(&mut body).expect("the refusal's body");
        let told: serde_json::Value = serde_json::from_slice(&body).expect("decode");
        assert_eq!(told["ok"], serde_json::json!(false));

        // A seat given back is a seat somebody else may take. Tried until it is rather than
        // once: the seat comes back when the reading thread notices the close, which is a
        // moment after the close and not at it.
        drop(held.pop());
        let mut taken = None;
        for _ in 0..100 {
            let mut next = UnixStream::connect(&path).expect("connect");
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
        let _ = std::fs::remove_file(&path);
    }
}
