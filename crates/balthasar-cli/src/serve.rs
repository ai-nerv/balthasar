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

    /// Receive the parent's private event descriptor and restore native stdin.
    #[arg(long, hide = true)]
    event_bootstrap: bool,
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
    let _events = if args.event_bootstrap {
        let sender = balthasar_host::events::forwarding::receive_stdin()
            .map_err(|_| anyhow::anyhow!("invalid native event bootstrap"))?;
        balthasar_host::events::forwarding::Forwarder::activate(sender).ok()
    } else {
        None
    };
    // Before any socket exists: the key its coordinator wrote on stdin, else one of its own.
    let told = told_key()?;
    let minted = told.is_none();
    let key = match told {
        Some(key) => key,
        None => balthasar_ipc::Key::mint()?,
    };
    // Corpses in both directories, the old name's included.
    for dir in balthasar_ipc::socket_dirs() {
        balthasar_ipc::swept(&dir);
    }
    let mut listener = Listener::bind(&args.instance)?;
    if minted {
        listener.publish(&key)?;
        eprintln!(
            "{}",
            render::dim(&format!("key {}.key", listener.path().display()))
        );
    }
    // Written whether or not anybody connects: a caller that finds no socket spawns from this path.
    let descriptor = balthasar_ipc::tool_descriptor()?;

    eprintln!("{}", render::bold(&listener.path().display().to_string()));
    balthasar_model::noted!(
        "serve: {} listening on {}, scope {scope}",
        args.instance,
        listener.path().display()
    );
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

    let agent = agent_here();
    let guard = balthasar_ipc::Guard::new(key, needs_key)
        .with_request_context(balthasar_host::events::requests::revision());
    let served = listener.serve_observed_scoped::<balthasar_host::Ownership>(guard, |peer: &Peer, ownership, request: Request, context| {
        balthasar_host::events::requests::with_peer(peer, context, || {
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
                        retired: None,
                        ladder: crate::ladder::Ladder::new(
                            store_path.map(Path::to_path_buf),
                            scope.clone(),
                            which.clone(),
                        ),
                    })
                });
                match made {
                    Ok(ready) => {
                        balthasar_model::noted!("store: opened for {named}");
                        eprintln!("{}", render::dim(&format!("tool {named}")));
                        seat.insert(ready)
                    }
                    Err(why) => return Reply::refused(why.to_string()),
                }
            }
        };
        let now = now();
        let mut at = Answering {
            store: &mut held.store,
            scrollback: Some(&mut held.scrollback),
            scratch: Some(&mut held.scratch),
            scope: scope.clone(),
            agent: agent_of(peer, &agent),
            now,
            inject_floor: floors.inject,
            live_floor: floors.live,
            capture,
        };
        let reply =
            balthasar_host::answer_scoped_hooked(&mut at, &Door::Socket(peer.clone()), &request, loaded, ownership);
        retire(
            held,
            &request,
            &reply,
            loaded.settings().retained_since(now),
            now,
        );
        if request.call == "observe"
            && reply.ok
            && let Some(session) = request.args.first().and_then(|v| v.as_str())
        {
            held.ladder.heard(session, now);
        }
        held.ladder.tick(&request.call, loaded.settings().ladder, now);
        reply
        })
    });

    // The socket goes before the stores: closing a store checkpoints its WAL and fsyncs.
    drop(listener);
    if loaded.settings().ladder.is_some() {
        for held in opened.values_mut() {
            held.ladder.finish(now());
        }
    }
    served?;
    Ok(())
}

/// The key a coordinator wrote on standard input, or `None` when stdin is a terminal or ends
/// before a line begins.
fn told_key() -> anyhow::Result<Option<balthasar_ipc::Key>> {
    use std::io::IsTerminal;
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Ok(None);
    }
    balthasar_ipc::Key::read(&mut stdin.lock()).map_err(anyhow::Error::msg)
}

/// Whether a verb on the socket needs the key: one that changes the store or issues work does,
/// one that only reads does not, and a verb this does not have is refused by name either way.
fn needs_key(verb: &str) -> bool {
    balthasar_host::known(verb).is_some_and(|known| known.writes)
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
            balthasar_host::answer_hooked(&mut at, &Door::Owner, &request, loaded)
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
    /// The passes this store climbs on its own.
    ladder: crate::ladder::Ladder,
    /// When turn retention last ran on these stores.
    retired: Option<balthasar_model::Timestamp>,
}

/// Once a turn is recorded, and at most once a day, remove the turns of sessions quiet since
/// `before`. The session that just spoke is never one of them; no `before`, nothing is removed.
fn retire(
    held: &mut Opened,
    request: &Request,
    reply: &Reply,
    before: Option<balthasar_model::Timestamp>,
    now: balthasar_model::Timestamp,
) {
    let (Some(before), Some(live)) = (before, request.args.first().and_then(|v| v.as_str())) else {
        return;
    };
    let due = held
        .retired
        .is_none_or(|last| now.saturating_sub(last) >= balthasar_store::DAY);
    if request.call != "observe" || !reply.ok || !due {
        return;
    }
    held.retired = Some(now);
    let live = [balthasar_model::SessionId::new(live)];
    match balthasar_store::prune_turns(
        &held.scrollback,
        Some(&mut held.scratch),
        Some(&mut held.store),
        before,
        &live,
    ) {
        Ok(pruned) if pruned.sessions > 0 => balthasar_model::noted!(
            "serve: retention removed {} turns of {} sessions and {} scratch copies",
            pruned.turns,
            pruned.sessions,
            pruned.mirrors
        ),
        Ok(_) => {}
        Err(why) => balthasar_model::noted!("serve: retention failed: {why}"),
    }
}

/// What a harness names its agent in, in the environment it starts `serve` with.
///
/// The role's name, because the harness on the other end talks to a `memory` rather than to this
/// program — see `ROLES.md`.
const AGENT: &str = "MAGI_MEMORY_AGENT";

/// What that variable used to be called. Still read, for one release.
const AGENT_WAS: &str = "BALTHASAR_AGENT";

/// Say once that this process was named its agent the old way.
fn the_old_name_was_used() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        balthasar_model::noted!(
            "serve: started with its agent in ${AGENT_WAS}; ${AGENT} is what replaces it"
        );
    });
}

/// Which agent inside a run a connection belongs to: `served`, the one this `serve` was started
/// for. What the peer's environment named at accept is attribution, said once when it differs.
fn agent_of(peer: &Peer, served: &balthasar_model::AgentId) -> balthasar_model::AgentId {
    if let Some((_, named)) = &peer.agent
        && named != served.as_str()
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            balthasar_model::noted!(
                "serve: pid {} names agent {named}; its calls are recorded as {served}",
                peer.pid
            );
        });
    }
    served.clone()
}

/// Which agent this process itself was started for, read from its own environment.
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

    #[test]
    fn a_peer_naming_another_agent_is_recorded_as_the_one_serve_was_started_for() {
        let served = balthasar_model::AgentId::new("alpha-rho");
        let peer = Peer {
            pid: -1,
            uid: 0,
            program: None,
            agent: Some((AGENT.to_owned(), "intruder".to_owned())),
        };
        assert_eq!(agent_of(&peer, &served), served);
    }

    #[test]
    fn a_peer_that_named_nothing_is_the_one_serve_was_started_for() {
        let served = balthasar_model::AgentId::new("alpha-rho");
        let peer = Peer {
            pid: -1,
            uid: 0,
            program: None,
            agent: None,
        };
        assert_eq!(agent_of(&peer, &served), served);
    }
}

#[cfg(test)]
mod retiring {
    use super::*;
    use balthasar_model::{SessionId, Timestamp};
    use balthasar_store::DAY;

    const NOW: Timestamp = 1_800_000_000;
    const KEPT: Option<Timestamp> = Some(NOW - 30 * DAY);

    fn held(dir: &Path) -> Opened {
        Opened {
            store: balthasar_store::Store::ephemeral().expect("store"),
            scrollback: balthasar_store::Transcript::ephemeral().expect("scrollback"),
            scratch: balthasar_store::Scratchpad::at(dir),
            retired: None,
            ladder: crate::ladder::Ladder::new(
                Some(dir.join("memory.db")),
                balthasar_model::ScopeId::new("/w"),
                crate::Which {
                    tool: balthasar_store::Tool::default(),
                    named: false,
                },
            ),
        }
    }

    fn said(held: &Opened, session: &str) {
        let turn = balthasar_store::Turn {
            at: NOW - 40 * DAY,
            role: "user".into(),
            kind: "prose".into(),
            text: format!("{session} said this"),
            ..balthasar_store::Turn::default()
        };
        held.scrollback
            .insert(&SessionId::new(session), &turn)
            .expect("turn");
    }

    fn holds(held: &Opened, session: &str) -> u64 {
        held.scrollback
            .count(&SessionId::new(session))
            .expect("count")
    }

    fn call(verb: &str, session: &str) -> Request {
        Request {
            call: verb.to_owned(),
            args: vec![serde_json::json!(session)],
        }
    }

    #[test]
    fn a_recorded_turn_prunes_quiet_sessions_once_a_day_and_never_its_own() {
        let dir = balthasar_model::scratch::Scratch::new("balthasar-serve-retire", "daily");
        let mut held = held(&dir);
        said(&held, "quiet");
        said(&held, "speaking");
        retire(
            &mut held,
            &call("replay", "speaking"),
            &Reply::none(),
            KEPT,
            NOW,
        );
        assert_eq!(holds(&held, "quiet"), 1, "a read prunes nothing");

        retire(
            &mut held,
            &call("observe", "speaking"),
            &Reply::none(),
            KEPT,
            NOW,
        );
        assert_eq!(holds(&held, "quiet"), 0);
        assert_eq!(holds(&held, "speaking"), 1, "the session that spoke");

        said(&held, "later");
        let observed = call("observe", "speaking");
        retire(&mut held, &observed, &Reply::none(), KEPT, NOW + DAY / 2);
        assert_eq!(holds(&held, "later"), 1, "not twice in one day");
        retire(&mut held, &observed, &Reply::none(), KEPT, NOW + DAY);
        assert_eq!(holds(&held, "later"), 0);
    }

    #[test]
    fn with_no_retention_set_nothing_is_removed() {
        let dir = balthasar_model::scratch::Scratch::new("balthasar-serve-retire", "unset");
        let mut held = held(&dir);
        said(&held, "quiet");
        retire(
            &mut held,
            &call("observe", "speaking"),
            &Reply::none(),
            None,
            NOW,
        );
        assert_eq!(holds(&held, "quiet"), 1);
        assert_eq!(held.retired, None);
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
