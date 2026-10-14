//! `handle:call(request, { deadline_ms = N })`: one family call, framed, sent, read and decoded
//! here rather than in Lua.

use luna::{Context, Value};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// The family's one frame cap: 16 MiB, on the call written and on the reply read.
pub(super) const CAP: usize = 16 * 1024 * 1024;

/// How long the call may take: `opts.deadline_ms` when it is a positive number, else `fallback`.
pub(super) fn patience<'gc>(ctx: Context<'gc>, opts: Value<'gc>, fallback: Duration) -> Duration {
    let Value::Table(opts) = opts else {
        return fallback;
    };
    match opts.get_value(ctx, "deadline_ms") {
        Value::Integer(ms) if ms > 0 => Duration::from_millis(ms.unsigned_abs()),
        Value::Number(ms) if ms > 0.0 && ms.is_finite() => Duration::from_secs_f64(ms / 1000.0),
        _ => fallback,
    }
}

/// `request` as one framed JSON call, or why it is not sent: a value that cannot be written down,
/// or a body over [`CAP`].
pub(super) fn framed<'gc>(ctx: Context<'gc>, request: Value<'gc>) -> Result<Vec<u8>, String> {
    if !matches!(request, Value::Table(_)) {
        return Err("call needs a request table".to_owned());
    }
    let json = crate::convert::to_json(ctx, request, "the request").map_err(|why| {
        format!("this request cannot be written down as JSON ({why}); it was not sent")
    })?;
    let body = serde_json::to_vec(&json).map_err(|why| format!("{why}; it was not sent"))?;
    let length = u32::try_from(body.len())
        .ok()
        .filter(|n| *n as usize <= CAP)
        .ok_or_else(|| {
            format!(
                "this call is {} bytes, over the {CAP}-byte frame cap; it was not sent",
                body.len()
            )
        })?;
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Write `frame` and read one reply by `deadline`, decoded. Any failure leaves the connection out
/// of step, and the caller closes it.
pub(super) fn exchange(
    socket: &mut UnixStream,
    frame: &[u8],
    deadline: Instant,
) -> Result<serde_json::Value, String> {
    write_by(socket, frame, deadline).map_err(|why| format!("sending the call: {why}"))?;
    let mut head = [0_u8; 4];
    read_by(socket, &mut head, deadline).map_err(|why| format!("awaiting the reply: {why}"))?;
    let length = u32::from_be_bytes(head) as usize;
    if length == 0 || length > CAP {
        return Err(format!(
            "a reply of {length} bytes; the frame cap is {CAP} and zero is malformed"
        ));
    }
    let mut body = vec![0_u8; length];
    read_by(socket, &mut body, deadline).map_err(|why| format!("reading the reply: {why}"))?;
    serde_json::from_slice(&body).map_err(|why| format!("a reply that will not decode: {why}"))
}

/// What is left of `deadline`, or a timeout once it has passed.
fn left(deadline: Instant) -> std::io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| std::io::Error::new(ErrorKind::TimedOut, "the deadline passed"))
}

/// All of `bytes`, each write bounded by what is left of `deadline`.
fn write_by(socket: &mut UnixStream, bytes: &[u8], deadline: Instant) -> std::io::Result<()> {
    let mut sent = 0;
    while let Some(rest) = bytes.get(sent..).filter(|rest| !rest.is_empty()) {
        socket.set_write_timeout(Some(left(deadline)?))?;
        match socket.write(rest) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(n) => sent += n,
            Err(why) if why.kind() == ErrorKind::Interrupted => {}
            Err(why) => return Err(why),
        }
    }
    Ok(())
}

/// Exactly `into.len()` bytes, each read bounded by what is left of `deadline`. A peer that hangs
/// up part way is a short read.
fn read_by(socket: &mut UnixStream, into: &mut [u8], deadline: Instant) -> std::io::Result<()> {
    let mut have = 0;
    while let Some(rest) = into.get_mut(have..).filter(|rest| !rest.is_empty()) {
        socket.set_read_timeout(Some(left(deadline)?))?;
        match socket.read(rest) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    format!("the peer closed after {have} bytes"),
                ));
            }
            Ok(n) => have += n,
            Err(why) if why.kind() == ErrorKind::Interrupted => {}
            Err(why) => return Err(why),
        }
    }
    Ok(())
}
