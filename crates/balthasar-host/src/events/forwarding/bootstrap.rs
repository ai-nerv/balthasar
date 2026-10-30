use super::{Sender, transport};
use crate::events::Lineage;
use crate::events::{Id, Program as Owner};
use rustix::{
    fd::{AsFd, OwnedFd},
    net::{self, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SocketType, sockopt},
    process::{self, Pid},
};
use serde::{Deserialize, Serialize};
use std::io::IoSliceMut;

pub const MAX_BOOTSTRAP_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Descriptor,
    Peer,
    Frame,
    Ancillary,
    Transport,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    NativeEventBootstrap,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    schema: u8,
    kind: Kind,
    program: Owner,
    producer: Id,
    lineage: Lineage,
}

/// Receive from the native parent and restore stdin before readers or children start.
pub fn receive_stdin() -> Result<Sender, Error> {
    let parent = process::getppid().ok_or(Error::Peer)?;
    let incoming = receive_from(std::io::stdin(), Owner::Balthasar, parent)?;
    rustix::stdio::dup2_stdin(&incoming.input).map_err(|_| Error::Descriptor)?;
    Ok(incoming.sender)
}

struct Incoming {
    sender: Sender,
    input: OwnedFd,
}

fn receive_from(socket: impl AsFd, owner: Owner, parent: Pid) -> Result<Incoming, Error> {
    private_socket(&socket, SocketType::SEQPACKET)?;
    let peer = sockopt::socket_peercred(&socket).map_err(|_| Error::Peer)?;
    if peer.pid != parent || peer.uid != process::geteuid() {
        return Err(Error::Peer);
    }
    let mut bytes = [0; MAX_BOOTSTRAP_BYTES];
    let mut storage = [std::mem::MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(2))];
    let mut control = RecvAncillaryBuffer::new(&mut storage);
    let reply = net::recvmsg(
        &socket,
        &mut [IoSliceMut::new(&mut bytes)],
        &mut control,
        RecvFlags::DONTWAIT | RecvFlags::CMSG_CLOEXEC,
    )
    .map_err(|_| Error::Transport)?;
    if !reply
        .flags
        .difference(net::ReturnFlags::CMSG_CLOEXEC)
        .is_empty()
        || reply.bytes == 0
    {
        return Err(Error::Frame);
    }
    let frame: Frame = serde_json::from_slice(bytes.get(..reply.bytes).ok_or(Error::Frame)?)
        .map_err(|_| Error::Frame)?;
    if frame.schema != 2 || frame.program != owner || !frame.lineage.valid() {
        return Err(Error::Frame);
    }
    let [event, input] = descriptors(&mut control)?;
    private_socket(&event, SocketType::DGRAM)?;
    let event_peer = sockopt::socket_peercred(&event).map_err(|_| Error::Peer)?;
    if event_peer.uid != process::geteuid() {
        return Err(Error::Peer);
    }
    check_input(&input)?;
    let sender = transport::Sender::adopt(event).map_err(|_| Error::Descriptor)?;
    Ok(Incoming {
        sender: Sender::new(sender, owner, frame.producer)
            .with_lineage(frame.lineage)
            .with_parent(parent),
        input,
    })
}

fn descriptors(control: &mut RecvAncillaryBuffer<'_>) -> Result<[OwnedFd; 2], Error> {
    let mut found = [None, None];
    let mut count = 0;
    for message in control.drain() {
        let RecvAncillaryMessage::ScmRights(descriptors) = message else {
            return Err(Error::Ancillary);
        };
        for fd in descriptors {
            let slot = found.get_mut(count).ok_or(Error::Ancillary)?;
            *slot = Some(fd);
            count += 1;
        }
    }
    let [event, input] = found;
    Ok([
        event.ok_or(Error::Ancillary)?,
        input.ok_or(Error::Ancillary)?,
    ])
}

fn private_socket(fd: impl AsFd, kind: SocketType) -> Result<(), Error> {
    if sockopt::socket_type(&fd).map_err(|_| Error::Descriptor)? != kind
        || sockopt::socket_domain(&fd).map_err(|_| Error::Descriptor)? != net::AddressFamily::UNIX
    {
        return Err(Error::Descriptor);
    }
    let duplicate = fd
        .as_fd()
        .try_clone_to_owned()
        .map_err(|_| Error::Descriptor)?;
    let socket = std::os::unix::net::UnixDatagram::from(duplicate);
    if !socket
        .local_addr()
        .map_err(|_| Error::Descriptor)?
        .is_unnamed()
        || !socket
            .peer_addr()
            .map_err(|_| Error::Descriptor)?
            .is_unnamed()
    {
        return Err(Error::Descriptor);
    }
    Ok(())
}

fn check_input(input: impl AsFd) -> Result<(), Error> {
    let flags = rustix::fs::fcntl_getfl(&input).map_err(|_| Error::Descriptor)?;
    if flags.contains(rustix::fs::OFlags::WRONLY) {
        return Err(Error::Descriptor);
    }
    match sockopt::socket_type(input) {
        Ok(SocketType::STREAM) | Err(rustix::io::Errno::NOTSOCK) => Ok(()),
        _ => Err(Error::Descriptor),
    }
}

#[cfg(test)]
mod tests;
