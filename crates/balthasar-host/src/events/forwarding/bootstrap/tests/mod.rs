#![allow(clippy::expect_used, clippy::indexing_slicing)]
use super::*;
use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketFlags};
use std::{fs::File, io::IoSlice};

const GOLDEN: &[u8] = br#"{"schema":2,"kind":"native_event_bootstrap","program":"balthasar","producer":"00000000000000000000000000000002","lineage":{"run_id":null,"agent_id":null,"turn_id":null,"ask_id":null}}"#;

fn packet(bytes: &[u8], fds: &[rustix::fd::BorrowedFd<'_>]) -> OwnedFd {
    let (parent, child) = net::socketpair(
        net::AddressFamily::UNIX,
        SocketType::SEQPACKET,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )
    .expect("bootstrap pair");
    let mut storage = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(4))];
    let mut control = SendAncillaryBuffer::new(&mut storage);
    if !fds.is_empty() {
        assert!(control.push(SendAncillaryMessage::ScmRights(fds)));
    }
    net::sendmsg(
        &parent,
        &[IoSlice::new(bytes)],
        &mut control,
        SendFlags::DONTWAIT | SendFlags::NOSIGNAL,
    )
    .expect("packet");
    child
}

#[test]
fn independent_golden_and_descriptor_contract() {
    let (read, write) = std::os::unix::net::UnixDatagram::pair().expect("event pair");
    write.set_nonblocking(true).expect("nonblocking");
    sockopt::set_socket_recv_buffer_size(&write, 16384).expect("receive buffer");
    sockopt::set_socket_send_buffer_size(&write, 16384).expect("send buffer");
    let input = File::open("/dev/null").expect("input");
    let socket = packet(GOLDEN, &[write.as_fd(), input.as_fd()]);
    let mut incoming = receive_from(socket, Owner::Balthasar, process::getpid()).expect("accepted");
    assert_eq!(
        String::from(incoming.sender.identity()),
        "00000000000000000000000000000002"
    );
    assert_eq!(incoming.sender.send(b"{}"), super::super::Sent::Queued);
    assert_eq!(read.recv(&mut [0; 2]).expect("event"), 2);
    assert_eq!(
        rustix::fs::fstat(&input).expect("input stat").st_ino,
        rustix::fs::fstat(&incoming.input)
            .expect("restored stat")
            .st_ino
    );
}

#[test]
fn wrong_schema_owner_kind_and_extra_fields_are_refused() {
    for (key, value) in [
        ("schema", serde_json::json!(1)),
        ("schema", serde_json::json!(3)),
        (
            "lineage",
            serde_json::json!({"run_id":"00000000000000000000000000000001"}),
        ),
        (
            "lineage",
            serde_json::json!({"turn_id":"00000000000000000000000000000003"}),
        ),
        (
            "lineage",
            serde_json::json!({"ask_id":"00000000000000000000000000000004"}),
        ),
        ("lineage", serde_json::json!({"prompt":"private"})),
        ("lineage", serde_json::Value::Null),
        ("program", serde_json::json!("casper")),
        ("kind", serde_json::json!("other")),
        ("producer", serde_json::json!("secret")),
        ("extra", serde_json::json!(1)),
    ] {
        let mut frame: serde_json::Value = serde_json::from_slice(GOLDEN).expect("golden");
        frame[key] = value;
        let socket = packet(&serde_json::to_vec(&frame).expect("json"), &[]);
        assert!(matches!(
            receive_from(socket, Owner::Balthasar, process::getpid()),
            Err(Error::Frame)
        ));
    }
    let socket = packet(&[b' '; MAX_BOOTSTRAP_BYTES + 1], &[]);
    assert!(matches!(
        receive_from(socket, Owner::Balthasar, process::getpid()),
        Err(Error::Frame)
    ));
}

#[test]
fn wrong_parent_and_missing_or_excess_rights_are_refused() {
    let input = File::open("/dev/null").expect("input");
    let socket = packet(GOLDEN, &[]);
    assert!(matches!(
        receive_from(
            socket,
            Owner::Balthasar,
            process::getppid().expect("parent")
        ),
        Err(Error::Peer)
    ));
    for count in [0, 1, 3, 4] {
        let fds = vec![input.as_fd(); count];
        let socket = packet(GOLDEN, &fds);
        assert!(matches!(
            receive_from(socket, Owner::Balthasar, process::getpid()),
            Err(Error::Ancillary | Error::Frame)
        ));
    }
}

#[test]
fn blocking_event_and_packet_input_are_refused() {
    let (_read, write) = std::os::unix::net::UnixDatagram::pair().expect("events");
    let input = File::open("/dev/null").expect("input");
    let socket = packet(GOLDEN, &[write.as_fd(), input.as_fd()]);
    assert!(matches!(
        receive_from(socket, Owner::Balthasar, process::getpid()),
        Err(Error::Descriptor)
    ));
    write.set_nonblocking(true).expect("nonblocking");
    let socket = packet(GOLDEN, &[write.as_fd(), write.as_fd()]);
    assert!(matches!(
        receive_from(socket, Owner::Balthasar, process::getpid()),
        Err(Error::Descriptor)
    ));
}

#[test]
fn rejected_rights_are_closed_in_an_isolated_process() {
    use std::{
        io::Read,
        os::unix::net::UnixStream,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    const CASE: &str =
        "events::forwarding::bootstrap::tests::rejected_rights_are_closed_in_an_isolated_process";
    const KEY: &str = "BALTHASAR_TEST_BOOTSTRAP_ISOLATED";
    if std::env::var(KEY).ok().as_deref() != Some(CASE) {
        let mut child = Command::new(std::env::current_exe().expect("test binary"))
            .args(["--exact", CASE, "--nocapture"])
            .env(KEY, CASE)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("isolated child");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().expect("wait child") {
                assert!(status.success(), "isolated rights test: {status}");
                return;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("isolated rights deadline");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let golden = std::str::from_utf8(GOLDEN).expect("golden");
    for (bytes, count) in [
        (
            golden.replace("\"schema\":2", "\"schema\":2,\"schema\":2"),
            2,
        ),
        (golden.replace("balthasar", "casper"), 2),
        ("x".repeat(MAX_BOOTSTRAP_BYTES + 1), 2),
        (golden.to_owned(), 3),
        (golden.to_owned(), 4),
    ] {
        let (input, mut peer) = UnixStream::pair().expect("input pair");
        peer.set_nonblocking(true).expect("sentinel");
        let fds = vec![input.as_fd(); count];
        let socket = packet(bytes.as_bytes(), &fds);
        drop(fds);
        drop(input);
        assert!(receive_from(&socket, Owner::Balthasar, process::getpid()).is_err());
        assert_eq!(peer.read(&mut [0]).expect("received rights must close"), 0);
    }
}
