//! The socket primitive the client libraries need.
//!
//! Layer one of three: the stub carries framing and encoding in plain Lua, but it cannot open a
//! socket, so the host lends it one. A host native like any other — deliberately *not* a VM
//! feature, so a VM that cannot load C modules needs no change to join the family.
//!
//! ```lua
//! local h = memo.stream.connect(path, timeout_ms)
//! h:send(bytes)   h:recv(n)   h:close()
//! ```
//!
//! This is also what lets memo dial *out*: a sibling's own client runs unchanged in this VM,
//! given this table.

use luna::{Callback, CallbackReturn, Context, Table, Value};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::time::Duration;

/// The most a single `recv` will be asked for.
///
/// A peer that says a frame is enormous must not make us allocate for it before a byte of it
/// has arrived. The client asks in pieces anyway; this bounds a hostile answer.
const MAX_RECV: usize = 16 * 1024 * 1024;

/// How long to wait for a peer that accepted and then said nothing.
///
/// A default rather than forever: a stale socket left by a killed process accepts and never
/// answers, which is indistinguishable from a hang without one.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// A connected socket, shared between a handle's methods.
type Handle = Rc<RefCell<Option<UnixStream>>>;

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

        let timeout = match timeout {
            Value::Integer(ms) if ms > 0 => Duration::from_millis(ms as u64),
            Value::Number(ms) if ms > 0.0 => Duration::from_millis(ms as u64),
            _ => DEFAULT_TIMEOUT,
        };

        match UnixStream::connect(&path) {
            Ok(socket) => {
                let _ = socket.set_read_timeout(Some(timeout));
                let _ = socket.set_write_timeout(Some(timeout));
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
            // A stream delivers what it likes: a short read is ordinary, and the client asks
            // again. Zero means the peer went away, which the client reads as a close.
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
mod tests {
    use crate::Engine;

    #[test]
    fn connecting_to_nothing_answers_nil_and_a_reason() {
        // A client has to be able to tell "nothing is listening" from a crash, because that is
        // the ordinary state of a machine where no daemon was started.
        let mut engine = Engine::new();
        engine
            .run(
                r#"
                local handle, why = memo.stream.connect("/no/such/socket")
                memo.answered = (handle == nil) and (type(why) == "string")
                "#,
                "probe.lua",
            )
            .expect("run");
        engine.harvest();
        assert_eq!(engine.config().boolean("answered"), Some(true));
    }

    #[test]
    fn the_primitive_is_named_where_a_siblings_client_looks_for_it() {
        // A copied stub finds the transport itself when nothing is passed, and it looks under
        // the family's globals. Getting this name wrong sends discovery to `io.popen`.
        let mut engine = Engine::new();
        engine
            .run(
                "memo.found = type(memo.stream) == \"table\" and type(memo.stream.connect) == \"function\"",
                "probe.lua",
            )
            .expect("run");
        engine.harvest();
        assert_eq!(engine.config().boolean("found"), Some(true));
    }
}
