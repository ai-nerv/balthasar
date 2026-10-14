//! Who is on the other end, taken from the kernel and never from a number the peer sent.

use std::os::unix::net::UnixStream;

/// Where a peer's environment names its agent, the role's own variable first. Attribution only.
pub const AGENT_VARS: &[&str] = &["MAGI_MEMORY_AGENT", "BALTHASAR_AGENT"];

/// A connected caller, read once when the connection is accepted and pinned to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub pid: i32,
    pub uid: u32,
    pub program: Option<String>,
    /// The agent the peer's environment named, with the variable that named it.
    pub agent: Option<(String, String)>,
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
            agent: std::fs::read(format!("/proc/{pid}/environ"))
                .ok()
                .and_then(|body| named_in(&body)),
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
    Some(program_name(&path))
}

/// The program a `/proc/<pid>/exe` link names. A binary replaced while it runs reads as
/// `name (deleted)`, and is still the program it was.
#[must_use]
pub fn program_name(path: &std::path::Path) -> String {
    let name = path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    name.strip_suffix(" (deleted)")
        .map_or(name.clone(), str::to_owned)
}

/// The agent one `/proc/<pid>/environ` block names, and the variable it used.
#[must_use]
pub fn named_in(body: &[u8]) -> Option<(String, String)> {
    AGENT_VARS.iter().find_map(|variable| {
        let prefix = format!("{variable}=");
        body.split(|byte| *byte == 0)
            .filter_map(|entry| std::str::from_utf8(entry).ok())
            .find_map(|entry| entry.strip_prefix(&prefix))
            .map(str::trim)
            .filter(|named| !named.is_empty())
            .map(|named| ((*variable).to_owned(), named.to_owned()))
    })
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

    #[test]
    fn a_binary_replaced_while_it_runs_is_still_the_same_program() {
        let path = std::path::Path::new("/home/you/.local/bin/harness (deleted)");
        assert_eq!(program_name(path), "harness");
        assert_eq!(
            program_name(std::path::Path::new("/usr/bin/harness")),
            "harness"
        );
    }

    fn environ(entries: &[&str]) -> Vec<u8> {
        entries.join("\0").into_bytes()
    }

    #[test]
    fn a_peer_that_names_itself_is_read_from_its_own_block() {
        let body = environ(&["PATH=/usr/bin", "MAGI_MEMORY_AGENT=zeta-pi", "HOME=/home/x"]);
        assert_eq!(named_in(&body).map(|(_, n)| n), Some("zeta-pi".to_owned()));
    }

    #[test]
    fn a_peer_still_naming_its_agent_the_old_way_is_read_all_the_same() {
        let body = environ(&["PATH=/usr/bin", "BALTHASAR_AGENT=zeta-pi"]);
        assert_eq!(
            named_in(&body),
            Some(("BALTHASAR_AGENT".to_owned(), "zeta-pi".to_owned()))
        );
    }

    #[test]
    fn a_peer_that_sets_both_means_the_one_that_replaced_the_other() {
        let body = environ(&["MAGI_MEMORY_AGENT=new", "BALTHASAR_AGENT=old"]);
        assert_eq!(named_in(&body).map(|(_, n)| n), Some("new".to_owned()));
    }

    #[test]
    fn a_peer_that_names_nothing_is_none_rather_than_main() {
        for body in [
            environ(&["PATH=/usr/bin"]),
            environ(&["MAGI_MEMORY_AGENT="]),
            environ(&["MAGI_MEMORY_AGENT=   "]),
            environ(&["BALTHASAR_AGENT="]),
            environ(&["MY_MAGI_MEMORY_AGENT=wrong"]),
            Vec::new(),
        ] {
            assert_eq!(
                named_in(&body),
                None,
                "{:?}",
                String::from_utf8_lossy(&body)
            );
        }
    }
}
