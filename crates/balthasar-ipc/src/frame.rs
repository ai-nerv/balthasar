//! The wire, settled before either end ships.
//!
//! ```text
//!   frame   4 bytes big-endian length, then the body
//!   request {"call":"recall","args":["build command",{"limit":5}]}
//!   reply   {"ok":true,"family":1,"n":1,"result":[…]}
//!   refusal {"ok":false,"family":1,"n":0,"result":[],"error":"…","fault":"refused"}
//!   failure {"ok":false,"family":1,"n":0,"result":[],"error":"…","fault":"failed"}
//! ```
//!
//! `result` is a list of return values and `n` says how many: a client that unpacks reads a
//! bare-value server as having returned nothing at all.
//!
//! JSON is the encoding on argv, on a newline-delimited pipe and on this socket alike. An
//! unanswered message is tagged `event`, both directions; `scripts/gate-wire.sh` refuses any other.

use std::io::{Read, Write};

/// The most a single frame may carry, so a claimed length cannot make us allocate ahead of it.
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

/// What went wrong on the wire.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("that is not a request: {0}")]
    Malformed(String),
    #[error("a frame of {0} bytes is over the {MAX_FRAME} limit")]
    TooLarge(usize),
    #[error("the peer closed the connection")]
    Closed,
    /// A read timeout between frames: a caller sitting quietly, not one that has gone.
    #[error("nothing said yet")]
    Idle,
}

/// One call, as it arrives.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Request {
    pub call: String,
    #[serde(default)]
    pub args: Vec<serde_json::Value>,
}

/// Which kind of "no" an answer is.
///
/// A refusal costs the caller a feature; a failure costs it the turn that just happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
    Refused,
    Failed,
}

/// Which revision of the family wire this speaks.
///
/// Carried on the reply rather than negotiated, and bumped when an older reader would misread.
pub const FAMILY: u16 = 1;

/// Which revision of the registrar surface a plugin file is written against. See EXTENDING.md.
pub const SURFACE: u16 = 1;

/// One answer, as it goes back. `n` and `result` are always there, refusals included; every field
/// defaults on the way in, so a reply from before one of them existed still parses.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default)]
    pub family: u16,
    /// Only on `verbs`: a fact about the program rather than about the reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<u16>,
    #[serde(default)]
    pub n: usize,
    #[serde(default)]
    pub result: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Which kind of no. Absent means refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault: Option<Fault>,
}

impl Reply {
    /// One value came back.
    #[must_use]
    pub fn one(value: serde_json::Value) -> Self {
        Self::of(vec![value])
    }

    /// Several values came back, each its own row.
    #[must_use]
    pub fn rows(values: Vec<serde_json::Value>) -> Self {
        Self::of(values)
    }

    /// Nothing came back, and that is fine.
    #[must_use]
    pub fn none() -> Self {
        Self::of(Vec::new())
    }

    fn of(values: Vec<serde_json::Value>) -> Self {
        Self {
            ok: true,
            family: FAMILY,
            surface: None,
            n: values.len(),
            result: values,
            error: None,
            fault: None,
        }
    }

    /// The call was refused: an ordinary answer, so the connection stays open.
    #[must_use]
    pub fn refused(why: impl Into<String>) -> Self {
        Self::no(why, Fault::Refused)
    }

    /// What was handed over was not recorded.
    #[must_use]
    pub fn failed(why: impl Into<String>) -> Self {
        Self::no(why, Fault::Failed)
    }

    fn no(why: impl Into<String>, fault: Fault) -> Self {
        Self {
            ok: false,
            family: FAMILY,
            surface: None,
            n: 0,
            result: Vec::new(),
            error: Some(why.into()),
            fault: Some(fault),
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
///
/// [`WireError::Idle`] when a read timeout expired with nothing of a frame received; an expiry
/// part-way through has already dropped bytes the next read would take for a header.
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

/// Whether an error is a read timeout rather than a broken stream.
///
/// `SO_RCVTIMEO` reports `WouldBlock` on some platforms and `TimedOut` on others.
fn expired(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// Fill a buffer, however many reads that takes.
///
/// Taking one short read for the whole message desynchronises on the first frame that splits.
fn exactly(from: &mut impl Read, into: &mut [u8]) -> Result<(), WireError> {
    let mut have = 0;
    while have < into.len() {
        match from.read(&mut into[have..]) {
            Ok(0) => return Err(WireError::Closed),
            Ok(n) => have += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            // Only at the start: the bytes already taken cannot be put back.
            Err(e) if expired(&e) && have == 0 => return Err(WireError::Idle),
            Err(e) => return Err(WireError::Io(e)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that times out instead of answering, after handing over `first`.
    struct Slow {
        first: Vec<u8>,
        at: usize,
    }

    impl Read for Slow {
        fn read(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
            if self.at >= self.first.len() {
                return Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, "quiet"));
            }
            let n = (self.first.len() - self.at).min(into.len());
            into[..n].copy_from_slice(&self.first[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }

    #[test]
    fn a_peer_that_has_said_nothing_is_idle_rather_than_gone() {
        let mut quiet = Slow {
            first: Vec::new(),
            at: 0,
        };
        assert!(matches!(recv(&mut quiet), Err(WireError::Idle)));
    }

    #[test]
    fn a_wait_that_expired_mid_frame_is_not_resumable() {
        let mut torn = Slow {
            first: vec![0, 0],
            at: 0,
        };
        assert!(matches!(recv(&mut torn), Err(WireError::Io(_))));
    }

    #[test]
    fn a_peer_that_closed_is_gone_rather_than_idle() {
        let mut closed: &[u8] = &[];
        assert!(matches!(recv(&mut closed), Err(WireError::Closed)));
    }

    #[test]
    fn a_frame_survives_the_round_trip() {
        let mut buffer = Vec::new();
        send(&mut buffer, b"hello").expect("send");
        assert_eq!(recv(&mut buffer.as_slice()).expect("recv"), b"hello");
    }

    #[test]
    fn a_frame_split_across_reads_is_reassembled() {
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
        let text = serde_json::to_string(&Reply::one(serde_json::json!(42))).expect("encode");
        assert!(text.contains(r#""n":1"#), "{text}");
        assert!(text.contains(r#""result":[42]"#), "{text}");
    }

    #[test]
    fn a_refusal_is_an_answer_rather_than_a_failure() {
        let text = serde_json::to_string(&Reply::refused("no")).expect("encode");
        assert!(text.contains(r#""ok":false"#), "{text}");
        assert!(text.contains(r#""fault":"refused""#), "{text}");
    }

    #[test]
    fn every_reply_carries_a_list_and_a_count_even_when_it_is_a_no() {
        for reply in [Reply::refused("no"), Reply::failed("broke"), Reply::none()] {
            let text = serde_json::to_string(&reply).expect("encode");
            assert!(text.contains(r#""n":0"#), "{text}");
            assert!(text.contains(r#""result":[]"#), "{text}");
        }
    }

    #[test]
    fn surface_rides_only_on_the_reply_that_was_given_one() {
        let plain = serde_json::to_string(&Reply::none()).expect("encode");
        assert!(!plain.contains("surface"), "{plain}");
        let mut listed = Reply::rows(vec![serde_json::json!("verbs")]);
        listed.surface = Some(SURFACE);
        let text = serde_json::to_string(&listed).expect("encode");
        assert!(text.contains(r#""surface":1"#), "{text}");
        assert!(text.contains(r#""n":1"#), "{text}");
    }

    #[test]
    fn a_reply_from_before_these_fields_still_parses() {
        let old: Reply = serde_json::from_str(r#"{"ok":false,"error":"no"}"#).expect("decode");
        assert_eq!(old.family, 0);
        assert_eq!(old.n, 0);
        assert!(old.result.is_empty());
    }

    #[test]
    fn a_request_with_no_arguments_is_a_request() {
        let parsed: Request = serde_json::from_str(r#"{"call":"verbs"}"#).expect("decode");
        assert_eq!(parsed.call, "verbs");
        assert!(parsed.args.is_empty());
    }
}
