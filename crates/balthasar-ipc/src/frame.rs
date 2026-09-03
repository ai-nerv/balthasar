//! The wire, settled before either end ships.
//!
//! ```text
//!   frame   4 bytes big-endian length, then the body
//!   request {"call":"recall","args":["build command",{"limit":5}]}
//!   reply   {"ok":true,"n":1,"result":[[…]]}
//!   refusal {"ok":false,"error":"…","fault":"refused"}
//!   failure {"ok":false,"error":"…","fault":"failed"}
//! ```
//!
//! `result` is a **list** of return values and `n` says how many, so one call answers with what
//! the function did. This is the shape the family already uses, and two tools disagreeing about
//! it fail *silently*: a client that unpacks reads a bare-value server as having returned
//! nothing at all, so the bug presents as an empty memory rather than as an error.

use std::io::{Read, Write};

/// The most a single frame may carry.
///
/// A peer that says a frame is enormous must not make us allocate for it before a byte of it
/// has arrived.
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

/// What went wrong on the wire.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    /// The socket did.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// The peer sent something that is not a request.
    #[error("that is not a request: {0}")]
    Malformed(String),
    /// The peer claimed a frame larger than anything is allowed to be.
    #[error("a frame of {0} bytes is over the {MAX_FRAME} limit")]
    TooLarge(usize),
    /// The peer went away.
    #[error("the peer closed the connection")]
    Closed,
}

/// One call, as it arrives.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Request {
    /// Which verb.
    pub call: String,
    /// Its arguments, in order.
    #[serde(default)]
    pub args: Vec<serde_json::Value>,
}

/// Which kind of "no" an answer is.
///
/// A caller that keeps its only transcript here has to tell two failures apart, and cannot do it
/// by reading the message. A refusal costs it a feature; a failed write costs it the turn that
/// just happened, and continuing past one writes the next turn on top of a hole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
    /// A verb balthasar will not do. The caller carries on; this is not fatal.
    Refused,
    /// Something the caller handed over was not recorded. The caller must stop.
    Failed,
}

/// One answer, as it goes back.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Reply {
    /// Whether the call was answered.
    pub ok: bool,
    /// How many values came back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<usize>,
    /// The values themselves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Vec<serde_json::Value>>,
    /// Why not, when `ok` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Which kind of "no", when `ok` is false.
    ///
    /// Machine-readable on purpose. The third state a caller needs — nothing listening at all —
    /// is not in here because it cannot be: it is what a failed dial looks like, and a balthasar
    /// that is not running cannot say so.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fault: Option<Fault>,
}

impl Reply {
    /// One value came back.
    #[must_use]
    pub fn one(value: serde_json::Value) -> Self {
        Self {
            ok: true,
            n: Some(1),
            result: Some(vec![value]),
            error: None,
            fault: None,
        }
    }

    /// Nothing came back, and that is fine.
    #[must_use]
    pub fn none() -> Self {
        Self {
            ok: true,
            n: Some(0),
            result: Some(Vec::new()),
            error: None,
            fault: None,
        }
    }

    /// The call was refused.
    ///
    /// A refusal is an ordinary answer, not a transport failure: the connection stays open and
    /// the exit status of a one-shot stays zero, because a client that had to tell "the tool
    /// said no" from "the tool is missing" would need two parsers.
    #[must_use]
    pub fn refused(why: impl Into<String>) -> Self {
        Self {
            ok: false,
            n: None,
            result: None,
            error: Some(why.into()),
            fault: Some(Fault::Refused),
        }
    }

    /// What was handed over was not recorded.
    ///
    /// Distinct from a refusal because the consequences are: a caller that treats this as "no,
    /// and carry on" continues a session on top of a turn that is not there. Anything that took
    /// custody of a caller's only copy and then could not keep it answers with this.
    #[must_use]
    pub fn failed(why: impl Into<String>) -> Self {
        Self {
            ok: false,
            n: None,
            result: None,
            error: Some(why.into()),
            fault: Some(Fault::Failed),
        }
    }
}

/// Write one frame.
pub fn send(to: &mut impl Write, body: &[u8]) -> Result<(), WireError> {
    let n = body.len();
    if n > MAX_FRAME {
        return Err(WireError::TooLarge(n));
    }
    let header = u32::try_from(n).unwrap_or(u32::MAX).to_be_bytes();
    to.write_all(&header)?;
    to.write_all(body)?;
    to.flush()?;
    Ok(())
}

/// Read one frame.
pub fn recv(from: &mut impl Read) -> Result<Vec<u8>, WireError> {
    let mut header = [0_u8; 4];
    exactly(from, &mut header)?;
    let n = u32::from_be_bytes(header) as usize;
    if n > MAX_FRAME {
        return Err(WireError::TooLarge(n));
    }
    let mut body = vec![0_u8; n];
    exactly(from, &mut body)?;
    Ok(body)
}

/// Fill a buffer, however many reads that takes.
///
/// A stream delivers what it likes. One read answering fewer bytes than asked for is ordinary,
/// and treating it as the whole message desynchronises on the first frame large enough to be
/// split.
fn exactly(from: &mut impl Read, into: &mut [u8]) -> Result<(), WireError> {
    let mut have = 0;
    while have < into.len() {
        match from.read(&mut into[have..]) {
            Ok(0) => return Err(WireError::Closed),
            Ok(n) => have += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(WireError::Io(e)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_survives_the_round_trip() {
        let mut buffer = Vec::new();
        send(&mut buffer, b"hello").expect("send");
        assert_eq!(recv(&mut buffer.as_slice()).expect("recv"), b"hello");
    }

    #[test]
    fn a_frame_split_across_reads_is_reassembled() {
        // The bug this exists to prevent: a client that took one short read for the whole
        // message desynchronises on the first reply large enough to be split.
        struct Dribble(Vec<u8>, usize);
        impl Read for Dribble {
            fn read(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
                if self.1 >= self.0.len() {
                    return Ok(0);
                }
                into[0] = self.0[self.1];
                self.1 += 1;
                Ok(1)
            }
        }
        let mut buffer = Vec::new();
        send(&mut buffer, b"a longer body than one byte").expect("send");
        let mut one_at_a_time = Dribble(buffer, 0);
        assert_eq!(
            recv(&mut one_at_a_time).expect("recv"),
            b"a longer body than one byte"
        );
    }

    #[test]
    fn a_peer_that_went_away_says_so() {
        assert!(matches!(recv(&mut [].as_slice()), Err(WireError::Closed)));
    }

    #[test]
    fn a_frame_nothing_is_allowed_to_be_is_refused_before_it_is_allocated() {
        let header = (MAX_FRAME as u32 + 1).to_be_bytes();
        assert!(matches!(
            recv(&mut header.as_slice()),
            Err(WireError::TooLarge(_))
        ));
    }

    #[test]
    fn a_reply_carries_a_list_and_a_count() {
        // Two tools in one family disagreeing here fail silently: a client that unpacks reads
        // a bare-value server as having returned nothing.
        let text = serde_json::to_string(&Reply::one(serde_json::json!(42))).expect("encode");
        assert!(text.contains(r#""n":1"#), "{text}");
        assert!(text.contains(r#""result":[42]"#), "{text}");
    }

    #[test]
    fn a_refusal_is_an_answer_rather_than_a_failure() {
        let text = serde_json::to_string(&Reply::refused("no")).expect("encode");
        assert!(text.contains(r#""ok":false"#), "{text}");
        assert!(!text.contains("result"), "{text}");
    }

    #[test]
    fn a_request_with_no_arguments_is_a_request() {
        let parsed: Request = serde_json::from_str(r#"{"call":"verbs"}"#).expect("decode");
        assert_eq!(parsed.call, "verbs");
        assert!(parsed.args.is_empty());
    }
}
