//! Ending a serve on a signal, without a signal handler.
//!
//! `Drop` does not run on `SIGTERM`, which is what `--tied` asks the kernel to send. Blocked and
//! collected in `sigwait` rather than handled: a handler body must be async-signal-safe, and the
//! socket unlink and the SQLite close are not. `libc` rather than `rustix`, whose
//! `kernel_sigprocmask` and `kernel_sigwait` are `doc(hidden)` and documented as unstable.
#![allow(unsafe_code)]

/// The signals that mean stop, and what to call them out loud.
const STOPPING: [(libc::c_int, &str); 2] = [(libc::SIGTERM, "SIGTERM"), (libc::SIGINT, "SIGINT")];

/// The two of them as a set, built the way libc insists a set is built.
fn stopping() -> libc::sigset_t {
    let mut set = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
    // SAFETY: `sigemptyset` writes a whole `sigset_t` through the pointer, which is what
    // initialises it; each `sigaddset` after that writes into a set that now exists.
    unsafe {
        libc::sigemptyset(set.as_mut_ptr());
        for (signal, _) in STOPPING {
            libc::sigaddset(set.as_mut_ptr(), signal);
        }
        set.assume_init()
    }
}

/// Block them, so every thread started from this one inherits the block. Says whether it took.
///
/// Call before the first thread is spawned: a process-directed signal goes to any thread that
/// will take it. When it did not take, start no waiter — a `sigwait` racing the default action
/// would sometimes appear to work. The binary calls this, never a library.
pub fn hold() -> bool {
    let set = stopping();
    // SAFETY: `set` is initialised above and outlives the call. Both signals in it are ordinary
    // ones — glibc reserves only its first few real-time signals — and nothing else in this
    // process is waiting on `SIGTERM` or `SIGINT` by any other means.
    unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut()) == 0 }
}

/// Whether this thread already has them blocked, and so whether waiting for one can work.
///
/// Asked of the kernel, not a flag: only a thread spawned from one that has them blocked can
/// promise the signal is still pending when it asks.
pub(crate) fn holding() -> bool {
    let mut current = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
    // SAFETY: with a null `set`, `pthread_sigmask` only reads this thread's mask out through the
    // third pointer, which is a local that outlives the call and is initialised by the write.
    let read = unsafe {
        libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), current.as_mut_ptr()) == 0
    };
    if !read {
        return false;
    }
    let current = unsafe { current.assume_init() };
    // Every one of them, not any: a waiter started for a half-blocked set lets the other half
    // kill the process mid-answer.
    STOPPING.iter().all(|(signal, _)| {
        // SAFETY: `current` was written whole by the call above.
        unsafe { libc::sigismember(&raw const current, *signal) == 1 }
    })
}

/// Wait until one of them arrives, and say which.
pub(crate) fn awaited() -> Option<&'static str> {
    let set = stopping();
    let mut arrived: libc::c_int = 0;
    // SAFETY: both pointers are to locals that outlive the call, and `sigwait` writes a signal
    // number through the second one only when it returns zero.
    let waited = unsafe { libc::sigwait(&raw const set, &raw mut arrived) };
    if waited != 0 {
        return None;
    }
    STOPPING
        .iter()
        .find(|(signal, _)| *signal == arrived)
        .map(|(_, named)| *named)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blocked_signal_is_waited_for_rather_than_lost() {
        // On a thread of its own, and joined, so the per-thread mask leaves with it rather than
        // deafening the rest of a `--test-threads=1` run.
        std::thread::spawn(|| {
            // `pthread_kill` rather than a process-directed signal: the other threads in this
            // binary never called `hold`, and one of them would take it and end the run.
            assert!(!holding(), "a fresh thread inherits nothing");
            assert!(hold(), "the mask has to take before anything is raised");
            assert!(holding(), "and the kernel has to agree that it took");
            // SAFETY: the target is this thread, which is running, and `SIGTERM` is blocked in
            // it by the call above, so the signal becomes pending rather than being acted on.
            let sent = unsafe { libc::pthread_kill(libc::pthread_self(), libc::SIGTERM) };
            assert_eq!(sent, 0, "the signal was not sent");
            assert_eq!(awaited(), Some("SIGTERM"));
        })
        .join()
        .expect("the waiting thread");
    }

    #[test]
    fn binding_a_socket_does_not_change_how_this_process_dies() {
        // The block used to happen inside `Listener::bind`, which left six tests that only bind
        // with the stop signals blocked, and interrupting `cargo test` sometimes did nothing.
        let listener =
            crate::Listener::bind(&format!("mask-{}", std::process::id())).expect("bind");
        for (signal, named) in STOPPING {
            assert!(!blocked(signal), "binding blocked {named}");
        }
        drop(listener);
    }

    /// Whether one signal is blocked in this thread right now.
    #[cfg(test)]
    fn blocked(signal: libc::c_int) -> bool {
        let mut current = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
        // SAFETY: with a null `set`, `pthread_sigmask` only reads this thread's mask out through
        // the third pointer, into a local that outlives the call.
        let read = unsafe {
            libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), current.as_mut_ptr()) == 0
        };
        // SAFETY: written whole by the call above, when it succeeded.
        read && unsafe { libc::sigismember(current.as_ptr(), signal) == 1 }
    }
}
