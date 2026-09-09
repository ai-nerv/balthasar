//! `balthasar serve`, `balthasar api` and `balthasar lua-api` — the three ways in from outside.

use crate::render::How;
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
    #[command(flatten)]
    how: How,
}

/// Hand over the client library.
#[derive(Debug, Parser)]
pub struct ClientArgs {
    #[command(flatten)]
    how: How,
}

/// Ask the kernel to end this process when whoever started it ends.
///
/// `PR_SET_PDEATHSIG` only watches from the moment it is set, and arrives when the *thread* that
/// spawned this exits rather than the whole process, so the parent is checked as well. Against
/// the pid the caller gave rather than `getppid`, which reads the reaper an orphan has already
/// been handed to and so compares equal while the caller is dead.
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
    // Before the tie, and so before any socket exists: `PR_SET_PDEATHSIG` is `SIGTERM`, and a
    // parent dying before the block is in place would end this process where it stands.
    if !balthasar_ipc::hold_stop_signals() {
        eprintln!(
            "{}",
            render::dim("could not hold SIGTERM; a stop will not be tidy")
        );
    }
    if let Some(caller) = args.tied {
        tie_to_caller(caller)?;
    }
    // Here rather than in `bind`: the sweep dials every socket in the runtime directory.
    balthasar_ipc::swept(&balthasar_ipc::socket_dir());
    let listener = Listener::bind(&args.instance)?;
    // Written whether or not anybody connects: a caller that finds no socket spawns from this path.
    let descriptor = balthasar_ipc::tool_descriptor()?;

    eprintln!("{}", render::bold(&listener.path().display().to_string()));
    eprintln!("{}", render::dim(&format!("scope {scope}")));
    eprintln!(
        "{}",
        render::dim(&format!("descriptor {}", descriptor.display()))
    );

    // One store per tool, opened when that tool first speaks.
    let mut opened: std::collections::HashMap<balthasar_store::Tool, Opened> =
        std::collections::HashMap::new();
    let fallback = tool.tool.clone();
    // Read once at startup rather than per request, so a mid-session change cannot halve a ledger.
    let capture = loaded.settings().ledger().capture;

    let served = listener.serve(|peer: &Peer, request: Request| {
        let named = named_by_kernel(peer).unwrap_or_else(|| fallback.clone());
        let held = match opened.entry(named.clone()) {
            std::collections::hash_map::Entry::Occupied(seat) => seat.into_mut(),
            std::collections::hash_map::Entry::Vacant(seat) => {
                // The kernel named it, so this counts as named.
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

    // The socket goes before the stores: closing a store checkpoints its WAL and fsyncs.
    drop(listener);
    served?;
    Ok(())
}

/// Answer one question on standard output and exit successfully.
///
/// The reply is the wire shape, not what the human CLI prints. A refused verb is
/// `{"ok":false,…}` with exit status zero: "exited 1" cannot be told from a missing binary.
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
            // A bare word is a string.
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
            // One-shot is the owner's own door: this process, with no socket in between.
            balthasar_host::answer_with(&mut at, &Door::Owner, &request, |entry| loaded.mask(entry))
        }
        Err(why) => Reply::refused(why.to_string()),
    };

    let mut out = std::io::stdout().lock();
    crate::coordinated::emit(&mut out, args.how, &reply);
    Ok(())
}

/// Hand over the client library.
///
/// Bare, it is the source, because that is what a person redirecting it into a file wants. Asked
/// in an encoding, it is framed with the source as the one value in `result`.
pub fn lua_api(args: &ClientArgs) {
    if args.how.framed() {
        let mut out = std::io::stdout().lock();
        crate::coordinated::emit(
            &mut out,
            args.how,
            &Reply::one(serde_json::json!(balthasar_lua::CLIENT)),
        );
        return;
    }
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
/// Read from the peer's own environment rather than taken from a call, which a caller could vary
/// per call. A peer that names none falls back to this process's own, and only then to
/// [`AgentId::main`]: `/proc/<pid>/environ` is the block the kernel wrote at `exec`, which
/// `setenv` never touches, so a harness that learns its name later hands it down at the spawn.
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

/// Which agent this process itself is, and the fallback for a peer that named none.
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
/// A peer the kernel will not name falls back to whatever the daemon was started as.
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

#[cfg(test)]
mod agents {
    use super::*;

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
        let body = environ(&["MY_BALTHASAR_AGENT=wrong"]);
        assert_eq!(named_in(&body), None);
    }
}
