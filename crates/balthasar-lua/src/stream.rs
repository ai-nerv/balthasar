//! The socket primitive the client libraries need.
//!
//! The stub carries framing and encoding in plain Lua but cannot open a socket, so the host lends
//! it one. This is also what lets balthasar dial out: a sibling's client runs unchanged in this VM.
//!
//! ```lua
//! local h = balthasar.stream.connect(path, timeout_ms)
//! h:send(bytes)   h:recv(n)   h:close()
//! ```

use luna::{Callback, CallbackReturn, Context, Table, Value};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

/// The most a single `recv` will be asked for. A peer that says a frame is enormous must not make
/// us allocate for it before a byte of it has arrived.
const MAX_RECV: usize = 16 * 1024 * 1024;

/// How long to wait for a peer that accepted and then said nothing. A stale socket left by a
/// killed process accepts and never answers.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// A connected socket, shared between a handle's methods.
type Handle = Rc<RefCell<Option<UnixStream>>>;

/// Whether `path` is a socket a config may dial.
///
/// One of the family's own, by directory: the path's parent, canonicalised, is under one of
/// [`balthasar_model::runtime::family_dirs`], and the path itself is a socket owned by this uid.
/// Answers the canonical path to connect to, so a parent swapped after the check is not followed.
fn dialable(path: &Path) -> Option<PathBuf> {
    let roots = balthasar_model::runtime::family_dirs();
    dialable_under(path, &roots, balthasar_model::runtime::uid())
}

/// The same rule, against roots and a uid the caller names.
fn dialable_under(path: &Path, roots: &[PathBuf], uid: u32) -> Option<PathBuf> {
    let name = path.file_name()?;
    let parent = path.parent()?.canonicalize().ok()?;
    // The base each family dir sits in is canonicalised, the family dir itself is not: one that
    // is a symlink to somewhere else is judged by where it goes.
    let inside = roots.iter().any(|root| {
        let (Some(base), Some(leaf)) = (root.parent(), root.file_name()) else {
            return false;
        };
        let base = base.canonicalize().unwrap_or_else(|_| base.to_owned());
        parent.starts_with(base.join(leaf))
    });
    if !inside {
        return None;
    }
    let target = parent.join(name);
    let meta = std::fs::symlink_metadata(&target).ok()?;
    (meta.file_type().is_socket() && meta.uid() == uid).then_some(target)
}

/// Build the `stream` table.
pub fn table<'gc>(ctx: Context<'gc>) -> Table<'gc> {
    let stream = Table::new(&ctx);

    let connect = Callback::from_fn(&ctx, |ctx, _exec, mut stack| {
        let (path, timeout): (Value, Value) = stack.consume(ctx)?;
        let Value::String(path) = path else {
            stack.replace(ctx, (Value::Nil, "connect needs a path"));
            return Ok(CallbackReturn::Return);
        };
        let path = String::from_utf8_lossy(path.as_bytes()).into_owned();

        // Refused as an ordinary answer: raising would kill a config that probed for an absent
        // sibling.
        let Some(target) = dialable(Path::new(&path)) else {
            stack.replace(
                ctx,
                (
                    Value::Nil,
                    "only a family program's own socket is this VM's to open",
                ),
            );
            return Ok(CallbackReturn::Return);
        };

        let timeout = match timeout {
            Value::Integer(ms) if ms > 0 => Duration::from_millis(ms as u64),
            Value::Number(ms) if ms > 0.0 => Duration::from_millis(ms as u64),
            _ => DEFAULT_TIMEOUT,
        };

        // A socket answered by another uid is closed unused.
        let ours = |socket: &UnixStream| {
            rustix::net::sockopt::socket_peercred(socket)
                .is_ok_and(|said| said.uid.as_raw() == balthasar_model::runtime::uid())
        };
        match UnixStream::connect(&target) {
            Ok(socket) if !ours(&socket) => {
                stack.replace(ctx, (Value::Nil, "that socket is answered by another user"));
            }
            Ok(socket) => {
                // This VM has no way to interrupt a blocking read, so a handle whose deadline
                // never took would hang the whole harness.
                let deadlines = socket
                    .set_read_timeout(Some(timeout))
                    .and_then(|()| socket.set_write_timeout(Some(timeout)));
                if let Err(why) = deadlines {
                    let message = luna::String::from_slice(&ctx, why.to_string().as_bytes());
                    stack.replace(ctx, (Value::Nil, message));
                    return Ok(CallbackReturn::Return);
                }
                let held: Handle = Rc::new(RefCell::new(Some(socket)));
                stack.replace(ctx, handle_table(ctx, held));
            }
            Err(why) => {
                let message = luna::String::from_slice(&ctx, why.to_string().as_bytes());
                stack.replace(ctx, (Value::Nil, message));
            }
        }
        Ok(CallbackReturn::Return)
    });
    stream.set(ctx, "connect", connect).ok();
    stream
}

/// One connection's methods.
fn handle_table<'gc>(ctx: Context<'gc>, held: Handle) -> Table<'gc> {
    let handle = Table::new(&ctx);

    let sending = Rc::clone(&held);
    let send = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        // `h:send(bytes)` passes the handle as the first argument, so it is taken and dropped.
        let (_this, bytes): (Value, Value) = stack.consume(ctx)?;
        let Value::String(bytes) = bytes else {
            stack.replace(ctx, (Value::Nil, "send needs a string"));
            return Ok(CallbackReturn::Return);
        };
        let mut borrowed = sending.borrow_mut();
        let Some(socket) = borrowed.as_mut() else {
            stack.replace(ctx, (Value::Nil, "this connection is closed"));
            return Ok(CallbackReturn::Return);
        };
        match socket
            .write_all(bytes.as_bytes())
            .and_then(|()| socket.flush())
        {
            Ok(()) => stack.replace(ctx, true),
            Err(why) => {
                let message = luna::String::from_slice(&ctx, why.to_string().as_bytes());
                stack.replace(ctx, (Value::Nil, message));
            }
        }
        Ok(CallbackReturn::Return)
    });
    handle.set(ctx, "send", send).ok();

    let receiving = Rc::clone(&held);
    let recv = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        let (_this, wanted): (Value, Value) = stack.consume(ctx)?;
        let wanted = match wanted {
            Value::Integer(n) if n > 0 => (n as usize).min(MAX_RECV),
            Value::Number(n) if n > 0.0 => (n as usize).min(MAX_RECV),
            _ => {
                stack.replace(ctx, (Value::Nil, "recv needs a count"));
                return Ok(CallbackReturn::Return);
            }
        };
        let mut borrowed = receiving.borrow_mut();
        let Some(socket) = borrowed.as_mut() else {
            stack.replace(ctx, (Value::Nil, "this connection is closed"));
            return Ok(CallbackReturn::Return);
        };
        let mut buffer = vec![0_u8; wanted];
        match socket.read(&mut buffer) {
            // A short read is ordinary and the client asks again; zero means the peer went away.
            Ok(n) => {
                buffer.truncate(n);
                stack.replace(ctx, luna::String::from_slice(&ctx, &buffer));
            }
            Err(why) => {
                let message = luna::String::from_slice(&ctx, why.to_string().as_bytes());
                stack.replace(ctx, (Value::Nil, message));
            }
        }
        Ok(CallbackReturn::Return)
    });
    handle.set(ctx, "recv", recv).ok();

    let closing = Rc::clone(&held);
    let close = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        let _this: Value = stack.consume(ctx)?;
        closing.borrow_mut().take();
        stack.replace(ctx, true);
        Ok(CallbackReturn::Return)
    });
    handle.set(ctx, "close", close).ok();

    handle
}

#[cfg(test)]
mod dialling {
    use super::dialable_under;
    use balthasar_model::runtime::{family_dirs_in, uid};
    use balthasar_model::scratch::Scratch;
    use std::os::unix::net::UnixListener;

    /// A runtime directory of the test's own, with a family socket and a bus beside it.
    struct Runtime {
        root: Scratch,
        _held: Vec<UnixListener>,
    }

    impl Runtime {
        fn new(name: &str) -> Self {
            let root = Scratch::new("balthasar-dial", name);
            std::fs::create_dir_all(root.join("run/magi")).expect("mkdir");
            let held = vec![
                UnixListener::bind(root.join("run/magi/host.sock")).expect("bind"),
                UnixListener::bind(root.join("run/bus")).expect("bind"),
            ];
            Self { root, _held: held }
        }

        fn allowed(&self, path: &std::path::Path) -> bool {
            let roots = family_dirs_in(Some(self.root.join("run")), &self.root.join("tmp"), uid());
            dialable_under(path, &roots, uid()).is_some()
        }
    }

    #[test]
    fn a_family_socket_may_be_dialled() {
        let at = Runtime::new("family");
        assert!(at.allowed(&at.root.join("run/magi/host.sock")));
    }

    #[test]
    fn the_session_bus_beside_it_may_not() {
        let at = Runtime::new("bus");
        assert!(!at.allowed(&at.root.join("run/bus")));
    }

    #[test]
    fn climbing_out_of_a_family_dir_is_not_inside_it() {
        let at = Runtime::new("climb");
        assert!(!at.allowed(&at.root.join("run/magi/../bus")));
    }

    #[test]
    fn a_symlink_wearing_a_family_name_is_refused() {
        let at = Runtime::new("link");
        let link = at.root.join("run/magi/bus.sock");
        std::os::unix::fs::symlink(at.root.join("run/bus"), &link).expect("symlink");
        assert!(!at.allowed(&link));
    }

    #[test]
    fn a_family_dir_that_is_a_symlink_to_elsewhere_is_judged_by_where_it_goes() {
        let at = Runtime::new("dirlink");
        std::fs::create_dir_all(at.root.join("elsewhere")).expect("mkdir");
        let _bound = UnixListener::bind(at.root.join("elsewhere/x.sock")).expect("bind");
        std::os::unix::fs::symlink(at.root.join("elsewhere"), at.root.join("run/casper"))
            .expect("symlink");
        assert!(!at.allowed(&at.root.join("run/casper/x.sock")));
    }

    #[test]
    fn a_regular_file_is_not_a_socket() {
        let at = Runtime::new("file");
        std::fs::write(at.root.join("run/magi/plain"), "x").expect("write");
        assert!(!at.allowed(&at.root.join("run/magi/plain")));
    }

    #[test]
    fn a_socket_owned_by_another_uid_is_refused() {
        let at = Runtime::new("owner");
        let roots = family_dirs_in(Some(at.root.join("run")), &at.root.join("tmp"), uid());
        assert!(dialable_under(&at.root.join("run/magi/host.sock"), &roots, uid() + 1).is_none());
    }

    #[test]
    fn the_uid_suffixed_temporary_dir_counts_too() {
        let at = Runtime::new("tmp");
        let dir = at.root.join(format!("tmp/memory-{}", uid()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let _bound = UnixListener::bind(dir.join("api@default.sock")).expect("bind");
        assert!(at.allowed(&dir.join("api@default.sock")));
    }
}

#[cfg(test)]
mod tests {
    use crate::Engine;

    #[test]
    fn a_socket_outside_the_family_dirs_is_refused_from_lua() {
        let at = balthasar_model::scratch::Scratch::new("balthasar-dial", "lua");
        let bus = at.join("bus");
        let _bound = std::os::unix::net::UnixListener::bind(&bus).expect("bind");
        let mut engine = Engine::new();
        engine
            .run(
                &format!(
                    "local h, why = balthasar.stream.connect({:?})\n\
                     balthasar.answered = (h == nil) and why",
                    bus.display().to_string()
                ),
                "probe.lua",
            )
            .expect("run");
        engine.harvest();
        let said = engine.config().string("answered").unwrap_or("").to_owned();
        assert!(said.contains("family"), "{said}");
    }

    #[test]
    fn connecting_to_nothing_answers_nil_and_a_reason() {
        let mut engine = Engine::new();
        engine
            .run(
                r#"
                local handle, why = balthasar.stream.connect("/no/such/socket")
                balthasar.answered = (handle == nil) and (type(why) == "string")
                "#,
                "probe.lua",
            )
            .expect("run");
        engine.harvest();
        assert_eq!(engine.config().boolean("answered"), Some(true));
    }

    #[test]
    fn the_primitive_is_named_where_a_siblings_client_looks_for_it() {
        // A copied stub looks for the transport under the family's globals; a wrong name here
        // sends discovery to `io.popen`.
        let mut engine = Engine::new();
        engine
            .run(
                "balthasar.found = type(balthasar.stream) == \"table\" and type(balthasar.stream.connect) == \"function\"",
                "probe.lua",
            )
            .expect("run");
        engine.harvest();
        assert_eq!(engine.config().boolean("found"), Some(true));
    }
}
