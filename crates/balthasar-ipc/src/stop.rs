//! Ending a serve on a signal, without a signal handler.
//!
//! `Drop` does not run on `SIGTERM`, and `SIGTERM` is exactly what the tie sends: a balthasar
//! started with `--tied` asks the kernel to signal it when the process that started it dies, and
//! the kernel's default action for that signal is to end this one where it stands. Nothing
//! unlinks the socket. The runtime directory fills with names the next instance has to disprove
//! one at a time, and a forked child's probe meeting one of them is what unlinked a live
//! daemon's socket. The tie chose `SIGTERM` over `SIGKILL` on the written grounds that it
//! "leaves room to unlink a socket and checkpoint on the way out". This is that room.
//!
//! **Blocked and waited for, rather than handled.** A handler body has to be async-signal-safe —
//! no allocation, no locking, nothing in `std` that might do either — which rules out doing the
//! unlink and the SQLite close inside it, and leaves a flag for somebody else to poll. Blocking
//! the two signals instead means the kernel holds them pending until a thread asks for one; the
//! thread that asks is an ordinary thread making an ordinary blocking call, and everything it
//! does afterwards is ordinary safe Rust.
//!
//! `libc` rather than the `rustix` the rest of this crate calls. rustix does have these two
//! syscalls — `kernel_sigprocmask` and `kernel_sigwait` — but only inside `rustix::runtime`, a
//! `doc(hidden)` module whose own documentation says it is for implementing a libc, that its API
//! "is not considered stable", and that using it for anything else "is likely to create serious
//! problems". `libc` is already compiled into this binary (rand reaches it through getrandom),
//! so naming it directly costs a line in a manifest and nothing at all in the build.
#![allow(unsafe_code)]

/// The signals that mean stop, and what to call them out loud.
///
/// `SIGINT` beside `SIGTERM` because Ctrl-C at a terminal is the same intention arriving by
/// another road, and somebody who stops a `balthasar serve` by hand should not have to sweep up
/// after it either.
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

/// Block them here, so that every thread started from this one inherits the block.
///
/// A blocked signal is not a lost one: the kernel holds it pending against the process until
/// somebody collects it, which is what [`awaited`] is for. The whole process has to be covered,
/// because a process-directed signal is delivered to any one thread that will take it — so this
/// is called before the first thread is spawned, and before the socket file exists, rather than
/// after either.
///
/// **A whole process's disposition, so the process asks for it.** This used to be called from
/// `Listener::bind`, which put it in the path of six tests that bind a socket and never serve
/// on it: their binaries came out with `SIGINT` blocked on a test thread, and once one of them
/// also served, a `sigwait` thread was left standing that a process-directed Ctrl-C could be
/// delivered to instead of to the default action. A library that changes how its caller dies is
/// doing more than it was asked. The binary calls this; the library only ever asks whether it
/// was called.
///
/// Says whether the mask took. If it did not, no waiter is started: a `sigwait` racing the
/// default action would be worse than the plain death it replaces, because it would sometimes
/// appear to work.
pub fn hold() -> bool {
    let set = stopping();
    // SAFETY: `set` is initialised above and outlives the call. Both signals in it are ordinary
    // ones — glibc reserves only its first few real-time signals — and nothing else in this
    // process is waiting on `SIGTERM` or `SIGINT` by any other means.
    unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut()) == 0 }
}

/// Whether this thread already has them blocked, and so whether waiting for one can work.
///
/// Asked of the kernel rather than remembered in a flag, because the two are answers to
/// different questions: a flag would say "somebody called [`hold`] in this process", and what
/// [`crate::Listener::serve`] needs to know is whether the signal will still be pending when a
/// thread it is about to start asks for one. Only a thread that inherited the mask can promise
/// that, and the thread that inherits is the one spawned from here.
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
    // Every one of them, not any: a waiter started for a set that is only half blocked would
    // collect the blocked half and let the other half kill the process mid-answer.
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
        // On a thread of its own, and joined, so the mask leaves with it. A mask is per-thread
        // and `--test-threads=1` runs tests on the main thread, so blocking in the test body
        // would leave the rest of that run unable to hear Ctrl-C.
        std::thread::spawn(|| {
            // Aimed at this thread rather than at the process, and that is not a convenience: a
            // process-directed signal goes to whichever thread will take it, and the other
            // threads in a test binary have never called `hold`. One of them would take it and
            // the default action would end the run. `pthread_kill` can only be delivered here,
            // where it is blocked, so it waits until it is asked for.
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
        // The block used to happen inside `Listener::bind`, which put it in the path of six
        // tests that bind and never serve. Their binaries came out with the stop signals blocked
        // on a test thread, and one that went on to serve left a `sigwait` thread standing that
        // a process-directed Ctrl-C could be delivered to instead of to the default action — so
        // interrupting `cargo test` sometimes did nothing. A library decides nothing about how
        // its caller dies.
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
