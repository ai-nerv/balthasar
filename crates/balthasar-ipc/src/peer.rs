//! Who is on the other end, taken from the kernel and never from a number the peer sent.

use std::os::unix::net::UnixStream;

/// A connected caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub pid: i32,
    pub uid: u32,
    pub program: Option<String>,
}

impl Peer {
    /// Who is at the other end of this socket.
    ///
    /// `None` when the kernel will not say, which is treated as the most restricted caller.
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

    /// Whether this is the same user balthasar is running as.
    #[must_use]
    pub fn is_owner(&self) -> bool {
        self.uid == rustix::process::getuid().as_raw()
    }

    /// How a witness records it: `harness[pid 4021]`, or `pid 4021` when the program is unknown.
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
/// Best effort: a peer that has exited between connecting and being asked leaves nothing to read.
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
