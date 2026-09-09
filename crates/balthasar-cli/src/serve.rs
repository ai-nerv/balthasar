//! `balthasar serve`, `balthasar api` and `balthasar lua-api` — the three ways in from outside.
//!
//! Both doors reach the same dispatcher, which reaches the same functions the CLI calls. That
//! is the arrangement that keeps a socket answer and a terminal answer from describing a
//! memory differently.

use crate::{Which, now, open, render, runs_under};
use balthasar_host::{Answering, Door};
use balthasar_ipc::{Listener, Peer, Reply, Request};
use balthasar_model::ScopeId;
use clap::Parser;
use std::path::Path;

/// Listen for other programs.
#[derive(Debug, Parser)]
pub struct ServeArgs {
    /// A name, when more than one balthasar should be reachable at once.
    #[arg(long, default_value = "default")]
    instance: String,

    /// End this when the process with this id ends, and let the kernel be what enforces it.
    ///
    /// The caller names itself rather than being inferred, because "who started me" is a
    /// question with no reliable answer once the answer has changed: an orphan has already been
    /// handed to whatever reaps on this machine, and that is init on some and the user's own
    /// session manager on others. A pid to compare against is the same test on both.
    ///
    /// Absent by default: a balthasar started at a terminal or by a unit file is meant to
    /// outlive the thing that typed the command, and would otherwise leave with the shell.
    #[arg(long, value_name = "PID")]
    tied: Option<u32>,
}

/// Answer one question and exit.
#[derive(Debug, Parser)]
pub struct ApiArgs {
    /// The verb.
    verb: String,
    /// Its arguments, each as JSON.
    args: Vec<String>,
    /// Answer in CBOR rather than JSON.
    ///
    /// The same option `needs` and `configure` take, and the same one melchior's `ask` takes.
    /// This door answered in JSON alone, so a caller that had asked every other door in the
    /// family for CBOR had to keep a JSON parser for this one.
    #[arg(long)]
    cbor: bool,
}

/// Ask the kernel to end this process when whoever started it ends.
///
/// `PR_SET_PDEATHSIG`, and it has to be the kernel because of the case that matters: nothing
/// runs in a process that is killed outright, so a caller cannot be relied on to take its own
/// children with it. A cleanup on the way out covers the exits that have a way out. This covers
/// the rest — a panic, a `kill -9`, an OOM — which are exactly the ones that leave a memory
/// layer running with nobody to answer.
///
/// The signal only watches from the moment it is set, so a caller that died in the window
/// between the spawn and this call is a death nothing was ever sent for — and without the check
/// below this would serve forever, watching a parent that had already gone. Asking whether the
/// caller is still our parent settles that, and settles the other gap too: the signal arrives
/// when the *thread* that spawned this exits rather than the whole process.
///
/// Against the pid the caller gave rather than against whatever `getppid` said a moment ago,
/// which was the version that did not work: an orphan is reparented before it gets here, so the
/// value read first and the value read second are the same reaper, and the comparison passed
/// while the caller was already dead.
fn tie_to_caller(caller: u32) -> anyhow::Result<()> {
    rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::TERM))?;
    let ours = rustix::process::getppid().is_some_and(|parent| {
        u32::try_from(parent.as_raw_nonzero().get()).is_ok_and(|pid| pid == caller)
    });
    if !ours {
        std::process::exit(0);
    }
    Ok(())
}

/// Start listening.
pub fn serve(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &ServeArgs,
    floors: balthasar_lua::Floors,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    // Before the tie, and so before any socket exists. `PR_SET_PDEATHSIG` is `SIGTERM`, and a
    // parent that dies in the window between asking for the signal and blocking it would end
    // this process where it stands — which is survivable only because there is nothing on disk
    // yet to leave behind. Once the socket is bound there is, and by then the block is in place.
    if !balthasar_ipc::hold_stop_signals() {
        eprintln!(
            "{}",
            render::dim("could not hold SIGTERM; a stop will not be tidy")
        );
    }
    if let Some(caller) = args.tied {
        tie_to_caller(caller)?;
    }
    let listener = Listener::bind(&args.instance)?;
    // Written whether or not anybody connects: a caller that finds no socket falls back to
    // spawning, and it can only do that if we left it an absolute path to spawn.
    let descriptor = balthasar_ipc::tool_descriptor()?;

    eprintln!("{}", render::bold(&listener.path().display().to_string()));
    eprintln!("{}", render::dim(&format!("scope {scope}")));
    eprintln!(
        "{}",
        render::dim(&format!("descriptor {}", descriptor.display()))
    );

    // One store per tool, opened when that tool first speaks. A harness and a shell reaching
    // the same daemon are two memories, and neither is opened on the chance that it might be.
    let mut opened: std::collections::HashMap<balthasar_store::Tool, Opened> =
        std::collections::HashMap::new();
    let fallback = tool.tool.clone();
    // Read once at startup rather than per request: a configuration that changed mid-session
    // would make half a run's ledger and half not, which is worse than either.
    let capture = loaded.settings().ledger().capture;

    let served = listener.serve(|peer: &Peer, request: Request| {
        let named = named_by_kernel(peer).unwrap_or_else(|| fallback.clone());
        let held = match opened.entry(named.clone()) {
            std::collections::hash_map::Entry::Occupied(seat) => seat.into_mut(),
            std::collections::hash_map::Entry::Vacant(seat) => {
                // The kernel named it, so this counts as named: a peer reads its own memory
                // rather than every tool's, which is what a program asking for context wants.
                let which = Which {
                    tool: named.clone(),
                    named: true,
                };
                let made = open(store_path, scope, &which).and_then(|store| {
                    crate::scrollback(store_path, scope, &which).map(|scrollback| Opened {
                        store,
                        scrollback,
                        scratch: balthasar_store::Scratchpad::at(runs_under(
                            store_path, scope, &which,
                        )),
                    })
                });
                match made {
                    Ok(ready) => {
                        eprintln!("{}", render::dim(&format!("tool {named}")));
                        seat.insert(ready)
                    }
                    Err(why) => return Reply::refused(why.to_string()),
                }
            }
        };
        let mut at = Answering {
            store: &mut held.store,
            scrollback: Some(&mut held.scrollback),
            scratch: Some(&mut held.scratch),
            scope: scope.clone(),
            agent: agent_of(peer),
            now: now(),
            inject_floor: floors.inject,
            live_floor: floors.live,
            capture,
        };
        balthasar_host::answer_with(&mut at, &Door::Socket(peer.clone()), &request, |entry| {
            loaded.mask(entry)
        })
    });

    // The socket goes before the stores do, and the order is deliberate rather than incidental:
    // closing a store checkpoints its WAL and fsyncs, and a caller that connects during that is
    // better off finding nothing and spawning its own than waiting on a daemon that has already
    // stopped answering. `opened` drops on the way out of this function, after this line.
    drop(listener);
    served?;
    Ok(())
}

/// Answer one question on standard output and exit successfully.
///
/// Two rules, both learned the hard way by the siblings. The reply is the **wire** shape rather
/// than what the human CLI prints, so a client needs one parser and not two. And a refused verb
/// is `{"ok":false,…}` with exit status zero, because a real error arriving as "exited 1" is
/// indistinguishable from the binary being missing.
pub fn api(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &ApiArgs,
    floors: balthasar_lua::Floors,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let parsed: Vec<serde_json::Value> = args
        .args
        .iter()
        .map(|raw| {
            // A bare word is a string. Every caller that shells out has to quote its JSON, and
            // making them quote `"recall"` twice is a papercut with no upside.
            serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.clone()))
        })
        .collect();

    let request = Request {
        call: args.verb.clone(),
        args: parsed,
    };

    let capture = loaded.settings().ledger().capture;
    let reply = match open(store_path, scope, tool).and_then(|store| {
        crate::scrollback(store_path, scope, tool).map(|scrollback| (store, scrollback))
    }) {
        Ok((mut store, mut scrollback)) => {
            let mut at = Answering {
                store: &mut store,
                scrollback: Some(&mut scrollback),
                scratch: None,
                scope: scope.clone(),
                agent: agent_here(),
                now: now(),
                inject_floor: floors.inject,
                live_floor: floors.live,
                capture,
            };
            // One-shot is the owner's own door: it is this process, started by whoever ran it,
            // with no socket in between and nobody else to attribute it to.
            balthasar_host::answer_with(&mut at, &Door::Owner, &request, |entry| loaded.mask(entry))
        }
        Err(why) => Reply::refused(why.to_string()),
    };

    let mut out = std::io::stdout().lock();
    crate::coordinated::emit(&mut out, args.cbor, &serde_json::to_value(&reply)?);
    Ok(())
}

/// Print the client library, for a program that needs to embed it.
pub fn lua_api() {
    print!("{}", balthasar_lua::CLIENT);
}

/// One tool's memory, held open for as long as the daemon is.
struct Opened {
    store: balthasar_store::Store,
    scrollback: balthasar_store::Transcript,
    scratch: balthasar_store::Scratchpad,
}

/// What a harness names its agent in, in the environment of the process that connects.
const AGENT: &str = "BALTHASAR_AGENT";

/// Which agent inside a run a connection belongs to.
///
/// Read from the peer's own environment rather than taken from a call, which is the same shape
/// as `named_by_kernel` above and exists for the same reason: a dimension a caller can vary per
/// call is not isolation, it is a parameter, and a subagent that could name a sibling's agent
/// would read a sibling's scratch by asking for it. What can be varied here is the peer's own
/// name for itself, which is no more than it already gets to decide by being that process.
///
/// **A peer that names none falls back to this process's own**, and only then to
/// [`AgentId::main`]. `/proc/<pid>/environ` is the block the kernel wrote at `exec` and nothing
/// else: `setenv` allocates on the heap and never touches it, so the one process that cannot put
/// its name there is the one that learned it after starting — which is exactly the harness that
/// spawned this balthasar, since it is told its name by a layer it starts itself. It hands the
/// name down at the spawn instead, where this process's own environment is the only place it can
/// arrive.
///
/// Safe because it is one balthasar per harness: the fallback is the agent of whoever convened
/// this one. A subagent spawned with its own name has it in its own initial block, so the peer
/// read finds it and the fallback is never reached — which is why the order is peer first.
///
/// Naming none at all is one directory per run, exactly the arrangement that existed before there
/// were agents, so a harness that has never heard of them is unchanged.
fn agent_of(peer: &Peer) -> balthasar_model::AgentId {
    std::fs::read(format!("/proc/{}/environ", peer.pid))
        .ok()
        .and_then(|body| named_in(&body))
        .unwrap_or_else(agent_here)
}

/// What `BALTHASAR_AGENT` says in one `/proc/<pid>/environ` block, if it says anything.
fn named_in(body: &[u8]) -> Option<balthasar_model::AgentId> {
    let prefix = format!("{AGENT}=");
    body.split(|byte| *byte == 0)
        .filter_map(|entry| std::str::from_utf8(entry).ok())
        .find_map(|entry| entry.strip_prefix(&prefix))
        .map(str::trim)
        .filter(|named| !named.is_empty())
        .map(balthasar_model::AgentId::new)
}

/// Which agent this process itself is: the door that has no peer, and the fallback for one whose
/// peer named none.
///
/// `balthasar api` is one process answering one question for whoever ran it, so the environment
/// to read is our own. `serve` reaches here for the harness that spawned it — see [`agent_of`].
fn agent_here() -> balthasar_model::AgentId {
    std::env::var(AGENT)
        .ok()
        .map(|named| named.trim().to_owned())
        .filter(|named| !named.is_empty())
        .map_or_else(
            balthasar_model::AgentId::main,
            balthasar_model::AgentId::new,
        )
}

/// Which tool a connection belongs to, as the kernel names it.
///
/// This is the whole reason a tool dimension is safe: the caller is not asked, so it cannot
/// answer wrongly. A peer the kernel will not name, or whose name nothing survives, falls back
/// to whatever the daemon was started as rather than being filed under a guess.
fn named_by_kernel(peer: &Peer) -> Option<balthasar_store::Tool> {
    peer.program
        .as_deref()
        .and_then(|program| {
            Path::new(program)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .and_then(|name| balthasar_store::Tool::from_program(&name))
}

/// The peer's own name comes first, and the fallback exists at all.
#[cfg(test)]
mod agents {
    use super::*;

    /// One `/proc/<pid>/environ` block: NUL-separated, and no trailing separator to rely on.
    fn environ(entries: &[&str]) -> Vec<u8> {
        entries.join("\0").into_bytes()
    }

    #[test]
    fn a_peer_that_names_itself_is_read_from_its_own_block() {
        let body = environ(&["PATH=/usr/bin", "BALTHASAR_AGENT=zeta-pi", "HOME=/home/x"]);
        assert_eq!(
            named_in(&body).map(|id| id.to_string()),
            Some("zeta-pi".to_owned())
        );
    }

    #[test]
    fn a_peer_that_names_nothing_is_none_rather_than_main() {
        // The distinction the fallback turns on. Answering `main` here would be this function
        // deciding the question, and the caller would have nothing left to fall back *to*.
        for body in [
            environ(&["PATH=/usr/bin"]),
            environ(&["BALTHASAR_AGENT="]),
            environ(&["BALTHASAR_AGENT=   "]),
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

    #[test]
    fn a_name_is_not_matched_on_a_variable_that_merely_ends_in_it() {
        // `strip_prefix` on an entry, not a search through the whole block: a harness setting
        // `MY_BALTHASAR_AGENT` would otherwise name every agent of the run.
        let body = environ(&["MY_BALTHASAR_AGENT=wrong"]);
        assert_eq!(named_in(&body), None);
    }
}
