use super::super::MAX_EVENT_BYTES;
use rustix::{
    fd::OwnedFd,
    net::{self, SendFlags, sockopt},
};

const MAX_BUFFER: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Loss {
    pub full: u64,
    pub unavailable: u64,
    pub invalid: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    Queued,
    Full,
    Unavailable,
    Invalid,
}

pub struct Sender {
    socket: OwnedFd,
    loss: Loss,
}

impl Sender {
    pub(super) fn adopt(socket: OwnedFd) -> std::io::Result<Self> {
        use rustix::{
            fs::{OFlags, fcntl_getfl},
            io::{FdFlags, fcntl_getfd},
        };
        if !fcntl_getfd(&socket)?.contains(FdFlags::CLOEXEC)
            || !fcntl_getfl(&socket)?.contains(OFlags::NONBLOCK)
            || sockopt::socket_recv_buffer_size(&socket)? > MAX_BUFFER
            || sockopt::socket_send_buffer_size(&socket)? > MAX_BUFFER
        {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        Ok(Self {
            socket,
            loss: Loss::default(),
        })
    }

    pub fn send(&mut self, bytes: &[u8]) -> Sent {
        let (counter, result) = if bytes.is_empty() || bytes.len() > MAX_EVENT_BYTES {
            (&mut self.loss.invalid, Sent::Invalid)
        } else {
            match net::send(
                &self.socket,
                bytes,
                SendFlags::DONTWAIT | SendFlags::NOSIGNAL,
            ) {
                Ok(n) if n == bytes.len() => return Sent::Queued,
                Err(rustix::io::Errno::AGAIN) => (&mut self.loss.full, Sent::Full),
                _ => (&mut self.loss.unavailable, Sent::Unavailable),
            }
        };
        *counter = counter.saturating_add(1);
        result
    }

    pub fn loss(&self) -> Loss {
        self.loss
    }
}
