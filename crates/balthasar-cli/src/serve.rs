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
    // Here rather than in `bind`: the sweep dials every socket in the runtime directory. Both
    // directories, because a corpse under the old name is a name the next instance has to disprove.
    for dir in balthasar_ipc::socket_dirs() {
        balthasar_ipc::swept(&dir);
    }
    let listener = Listener::bind(&args.instance)?;
    // Written whether or not anybody connects: a caller that finds no socket spawns from this path.
    let descriptor = balthasar_ipc::tool_descriptor()?;

    eprintln!("{}", render::bold(&listener.path().display().to_string()));
    // Said out loud rather than assumed: an unrebuilt caller reaches this only through the second name.
    for also in listener.paths().into_iter().skip(1) {
        eprintln!("{}", render::dim(&format!("also {}", also.display())));
    }
    if listener.paths().len() < 2 {
        balthasar_model::noted!(
            "serve: {} could not be bound as well; a caller that has not been rebuilt will not \
             find this one",
            balthasar_ipc::legacy_socket_path(&args.instance).display()
        );
    }
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
        if !has_room(opened.len(), opened.contains_key(&named), named == fallback) {
            return Reply::refused(format!(
                "this balthasar already holds stores for {TOOLS} tools and will not open one for \
                 '{named}' — start a balthasar of its own for it"
            ));
        }
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
///
/// The role's name, because the harness on the other end talks to a `memory` rather than to this
/// program — see `ROLES.md`.
const AGENT: &str = "MAGI_MEMORY_AGENT";

/// What that variable used to be called. Still read, for one release.
const AGENT_WAS: &str = "BALTHASAR_AGENT";

/// Say once that a caller is still naming its agent the old way.
fn the_old_name_was_used() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        balthasar_model::noted!(
            "serve: a caller named its agent in ${AGENT_WAS}; ${AGENT} is what replaces it"
        );
    });
}

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

/// What names the agent in one `/proc/<pid>/environ` block, if anything does.
///
/// The role's variable first, then the program's. A harness that sets both means the new one.
fn named_in(body: &[u8]) -> Option<balthasar_model::AgentId> {
    let said = |variable: &str| {
        let prefix = format!("{variable}=");
        body.split(|byte| *byte == 0)
            .filter_map(|entry| std::str::from_utf8(entry).ok())
            .find_map(|entry| entry.strip_prefix(&prefix))
            .map(str::trim)
            .filter(|named| !named.is_empty())
            .map(balthasar_model::AgentId::new)
    };
    said(AGENT).or_else(|| {
        let older = said(AGENT_WAS);
        if older.is_some() {
            the_old_name_was_used();
        }
        older
    })
}

/// Which agent this process itself is, and the fallback for a peer that named none.
fn agent_here() -> balthasar_model::AgentId {
    let said = |variable: &str| {
        std::env::var(variable)
            .ok()
            .map(|named| named.trim().to_owned())
            .filter(|named| !named.is_empty())
    };
    said(AGENT)
        .or_else(|| {
            let older = said(AGENT_WAS);
            if older.is_some() {
                the_old_name_was_used();
            }
            older
        })
        .map_or_else(
            balthasar_model::AgentId::main,
            balthasar_model::AgentId::new,
        )
}

/// How many tools one daemon opens a store for.
///
/// Nothing authenticates the kernel-given name that picks one, so without a ceiling any program
/// reaching the socket mints directories and holds descriptors until one of the two runs out.
const TOOLS: usize = 8;

/// Whether a daemon already holding `open` tools will open one more.
///
/// `own` keeps a seat, so a crowd of peers cannot lock the owner out of its own memory.
fn has_room(open: usize, held: bool, own: bool) -> bool {
    held || open < if own { TOOLS } else { TOOLS - 1 }
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
        let body = environ(&["PATH=/usr/bin", "MAGI_MEMORY_AGENT=zeta-pi", "HOME=/home/x"]);
        assert_eq!(
            named_in(&body).map(|id| id.to_string()),
            Some("zeta-pi".to_owned())
        );
    }

    #[test]
    fn a_peer_still_naming_its_agent_the_old_way_is_read_all_the_same() {
        // A harness that has not been rebuilt sets only this, and its turns must not land under
        // `main` while it thinks they are landing under an agent.
        let body = environ(&["PATH=/usr/bin", "BALTHASAR_AGENT=zeta-pi"]);
        assert_eq!(
            named_in(&body).map(|id| id.to_string()),
            Some("zeta-pi".to_owned())
        );
    }

    #[test]
    fn a_peer_that_sets_both_means_the_one_that_replaced_the_other() {
        let body = environ(&["MAGI_MEMORY_AGENT=new", "BALTHASAR_AGENT=old"]);
        assert_eq!(
            named_in(&body).map(|id| id.to_string()),
            Some("new".to_owned())
        );
    }

    #[test]
    fn a_peer_that_names_nothing_is_none_rather_than_main() {
        for body in [
            environ(&["PATH=/usr/bin"]),
            environ(&["MAGI_MEMORY_AGENT="]),
            environ(&["MAGI_MEMORY_AGENT=   "]),
            environ(&["BALTHASAR_AGENT="]),
            environ(&["MAGI_MEMORY_AGENT=", "BALTHASAR_AGENT="]),
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
        for body in [
            environ(&["MY_MAGI_MEMORY_AGENT=wrong"]),
            environ(&["MY_BALTHASAR_AGENT=wrong"]),
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

#[cfg(test)]
mod tools {
    use super::{TOOLS, has_room};

    #[test]
    fn several_real_tools_share_one_daemon() {
        // The case the ceiling must not break: a family of programs and a harness or two.
        for open in 0..TOOLS - 1 {
            assert!(has_room(open, false, false), "{open} tools in");
        }
    }

    #[test]
    fn an_unknown_peer_stops_minting_stores_at_the_ceiling() {
        // Every new name is a directory on disk and descriptors held for the daemon's lifetime.
        assert!(!has_room(TOOLS - 1, false, false));
        assert!(!has_room(TOOLS, false, false));
    }

    #[test]
    fn a_tool_already_open_costs_nothing_to_answer_again() {
        assert!(has_room(TOOLS, true, false), "its store is already open");
    }

    #[test]
    fn a_crowd_of_peers_cannot_lock_the_owner_out_of_its_own_memory() {
        assert!(has_room(TOOLS - 1, false, true));
        assert!(!has_room(TOOLS - 1, false, false));
    }
}
