//! Who is on the other end.
//!
//! Taken from the kernel, never from a number the peer sent. A write ceiling that a caller can
//! talk its way past is not a ceiling, and identity is the one thing on this socket that must
//! not be self-reported.

use std::os::unix::net::UnixStream;

/// A connected caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    /// Its process id, as the kernel reports it.
    pub pid: i32,
    /// Its user id.
    pub uid: u32,
    /// What it is running, when that can be read.
    pub program: Option<String>,
}

impl Peer {
    /// Who is at the other end of this socket.
    ///
    /// `None` when the kernel will not say, which is treated as the most restricted caller
    /// rather than the least: an unidentifiable peer gets what an unidentifiable peer gets.
    #[must_use]
    pub fn of(stream: &UnixStream) -> Option<Self> {
        let credentials = rustix::net::sockopt::socket_peercred(stream).ok()?;
        let pid = credentials.pid.as_raw_nonzero().get();
        let uid = credentials.uid.as_raw();
        Some(Self {
            pid,
            uid,
            program: program_of(pid),
        })
    }

    /// Whether this is the same user memo is running as.
    ///
    /// The only identity check that matters here. A memory store is one person's, and a peer
    /// belonging to somebody else has no business in it whatever it claims to be.
    #[must_use]
    pub fn is_owner(&self) -> bool {
        self.uid == rustix::process::getuid().as_raw()
    }

    /// How a witness records it.
    ///
    /// `harness[pid 4021]` — the program the kernel says is running, and its process id. Enough
    /// for `memo why` to answer "which process believes this".
    #[must_use]
    pub fn named(&self) -> String {
        match &self.program {
            Some(program) => format!("{program}[pid {}]", self.pid),
            None => format!("pid {}", self.pid),
        }
    }
}

/// What a process is running, from `/proc`.
///
/// Best effort: a peer that has exited between connecting and being asked about leaves nothing
/// to read, and that is not a reason to refuse the call it already made.
fn program_of(pid: i32) -> Option<String> {
    let path = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    Some(path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peer_is_identified_from_the_kernel() {
        let (here, there) = UnixStream::pair().expect("a pair");
        let peer = Peer::of(&here).expect("the kernel says");
        assert_eq!(peer.pid, std::process::id() as i32);
        assert!(peer.is_owner(), "we are talking to ourselves");
        drop(there);
    }

    #[test]
    fn a_peer_names_itself_the_way_a_witness_records_it() {
        let (here, there) = UnixStream::pair().expect("a pair");
        let peer = Peer::of(&here).expect("the kernel says");
        let named = peer.named();
        assert!(named.contains(&peer.pid.to_string()), "{named}");
        drop(there);
    }

    #[test]
    fn a_process_that_is_not_there_has_no_program() {
        assert_eq!(program_of(-1), None);
    }
}
