use super::{Correlation, Id, Lineage};
use rustix::process::{self, Pid, Uid};
use std::{cell::Cell, io::Write, sync::OnceLock};

const MOST: usize = 256;
static SOURCE: OnceLock<Source> = OnceLock::new();

struct Source {
    parent: Pid,
    uid: Uid,
    root: Lineage,
}

thread_local! {
    static CURRENT: Cell<Option<Lineage>> = const { Cell::new(None) };
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: u8,
    run_id: Id,
    agent_id: Id,
    turn_id: Option<Id>,
    ask_id: Option<Id>,
}

struct Count(usize);

impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > MOST {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Source {
    fn accept(&self, value: serde_json::Value, pid: Pid, uid: Uid) -> Option<Lineage> {
        if pid != self.parent || uid != self.uid {
            return None;
        }
        serde_json::to_writer(Count(0), &value).ok()?;
        let request: Request = serde_json::from_value(value).ok()?;
        if request.schema != 1
            || Some(request.run_id) != self.root.run_id
            || Some(request.agent_id) != self.root.agent_id
        {
            return None;
        }
        Some(Lineage {
            turn_id: request.turn_id,
            ask_id: request.ask_id,
            ..self.root
        })
    }
}

pub(super) fn install(parent: Pid, root: Lineage) {
    if root.run_id.is_some()
        && root.agent_id.is_some()
        && root.turn_id.is_none()
        && root.ask_id.is_none()
    {
        let _ = SOURCE.set(Source {
            parent,
            uid: process::geteuid(),
            root,
        });
    }
}

pub fn revision() -> Option<u8> {
    SOURCE.get().map(|_| 1)
}

struct Scope(Option<Lineage>);

impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.set(self.0);
    }
}

fn scoped<T>(context: Option<Lineage>, work: impl FnOnce() -> T) -> T {
    let _scope = Scope(CURRENT.replace(context));
    work()
}

/// Scope metadata admitted by the socket key guard to one serialized request.
pub fn with_peer<T>(
    peer: &balthasar_ipc::Peer,
    value: Option<serde_json::Value>,
    work: impl FnOnce() -> T,
) -> T {
    let context = value.and_then(|value| {
        let source = SOURCE.get()?;
        if process::getppid() != Some(source.parent) {
            return None;
        }
        source.accept(value, Pid::from_raw(peer.pid)?, Uid::from_raw(peer.uid))
    });
    scoped(context, work)
}

pub(crate) fn capture(correlation: Correlation) -> Correlation {
    CURRENT
        .get()
        .map_or(correlation, |lineage| lineage.apply(correlation))
}

#[cfg(test)]
mod tests;
