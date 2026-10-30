//! Per-connection state owned by the serialized dispatcher.

use super::{Asked, Heard, Listener, Peer, Reply, Request, accept, answering, stop};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

/// Dispatcher state that can retain an idle connection while it owns a resource.
pub trait ConnectionState: Default {
    fn retains_connection(&self) -> bool {
        false
    }
}

impl ConnectionState for () {}

struct Owned<S> {
    value: S,
    retained: Arc<AtomicBool>,
}

impl<S> Drop for Owned<S> {
    fn drop(&mut self) {
        self.retained.store(false, Ordering::Release);
    }
}

pub(super) struct Connection {
    id: u64,
    asked: mpsc::Sender<Heard>,
    used: Cell<bool>,
    retained: Arc<AtomicBool>,
}

impl Connection {
    pub(super) fn new(asked: mpsc::Sender<Heard>, identities: &AtomicU64) -> Option<Self> {
        let id = identities
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .ok()?;
        Some(Self {
            id,
            asked,
            used: Cell::new(false),
            retained: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(super) fn answer(
        &self,
        peer: &Peer,
        request: Request,
        context: Option<serde_json::Value>,
    ) -> Reply {
        self.used.set(true);
        answering(&self.asked, self.id, &self.retained, peer, request, context)
    }

    pub(super) fn converse(
        &self,
        stream: &mut std::os::unix::net::UnixStream,
        peer: &Peer,
        guard: &crate::Guard,
        patience: super::Patience,
    ) {
        super::converse_retained(
            stream,
            peer,
            guard,
            &|peer, request, context| self.answer(peer, request, context),
            patience,
            &self.retained,
        );
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if self.used.get() {
            let _ = self.asked.send(Heard::Closed(self.id));
        }
    }
}

impl Listener {
    /// Serve with isolated connection state, dropped on disconnect or dispatcher shutdown.
    pub fn serve_scoped<S: ConnectionState>(
        &self,
        guard: crate::Guard,
        mut answer: impl FnMut(&Peer, &mut S, Request) -> Reply,
    ) -> std::io::Result<()> {
        self.serve_observed_scoped(guard, |peer, state, request, _| {
            answer(peer, state, request)
        })
    }

    /// Dispatch authenticated optional metadata separately from ordinary request arguments.
    pub fn serve_observed_scoped<S: ConnectionState>(
        &self,
        guard: crate::Guard,
        answer: impl FnMut(&Peer, &mut S, Request, Option<serde_json::Value>) -> Reply,
    ) -> std::io::Result<()> {
        let guard = Arc::new(guard);
        let listening = self
            .bound
            .iter()
            .map(|claimed| claimed.listener.try_clone())
            .collect::<Result<Vec<_>, _>>()?;
        let (asked, asking) = mpsc::channel::<Heard>();
        if stop::holding() {
            let stopping = asked.clone();
            std::thread::spawn(move || {
                if let Some(named) = stop::awaited() {
                    let _ = stopping.send(Heard::Stop(named));
                }
            });
        }
        let identities = Arc::new(AtomicU64::new(1));
        for one in listening {
            let (asked, guard, identities) =
                (asked.clone(), Arc::clone(&guard), Arc::clone(&identities));
            std::thread::spawn(move || accept(&one, &asked, &guard, &identities));
        }
        drop(asked);
        dispatch_observed(asking, answer);
        Ok(())
    }
}

#[cfg(test)]
fn dispatch<S: ConnectionState>(
    asking: mpsc::Receiver<Heard>,
    mut answer: impl FnMut(&Peer, &mut S, Request) -> Reply,
) {
    dispatch_observed(asking, |peer, state, request, _| {
        answer(peer, state, request)
    });
}

fn dispatch_observed<S: ConnectionState>(
    asking: mpsc::Receiver<Heard>,
    mut answer: impl FnMut(&Peer, &mut S, Request, Option<serde_json::Value>) -> Reply,
) {
    let mut sessions = BTreeMap::<u64, Owned<S>>::new();
    while let Ok(heard) = asking.recv() {
        match heard {
            Heard::Call(Asked {
                connection,
                retained,
                peer,
                request,
                context,
                answered,
            }) => {
                let call = request.call.clone();
                let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let owned = sessions.entry(connection).or_insert_with(|| Owned {
                        value: S::default(),
                        retained,
                    });
                    answer(
                        &peer,
                        &mut owned.value,
                        request,
                        context.map(|value| *value),
                    )
                }))
                .unwrap_or_else(|_| {
                    Reply::failed(format!("balthasar failed while answering `{call}`"))
                });
                if let Some(owned) = sessions.get(&connection) {
                    owned
                        .retained
                        .store(owned.value.retains_connection(), Ordering::Release);
                }
                let _ = answered.send(reply);
            }
            Heard::Closed(connection) => {
                sessions.remove(&connection);
            }
            Heard::Stop(named) => {
                eprintln!("balthasar: stopped on {named}");
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests;
