//! `aeon serve`, `aeon api` and `aeon lua-api` — the three ways in from outside.
//!
//! Both doors reach the same dispatcher, which reaches the same functions the CLI calls. That
//! is the arrangement that keeps a socket answer and a terminal answer from describing a
//! memory differently.

use crate::{Which, now, open, render, runs_under};
use aeon_host::{Answering, Door};
use aeon_ipc::{Listener, Peer, Reply, Request};
use aeon_model::ScopeId;
use clap::Parser;
use std::path::Path;

/// Listen for other programs.
#[derive(Debug, Parser)]
pub struct ServeArgs {
    /// A name, when more than one aeon should be reachable at once.
    #[arg(long, default_value = "default")]
    instance: String,
}

/// Answer one question and exit.
#[derive(Debug, Parser)]
pub struct ApiArgs {
    /// The verb.
    verb: String,
    /// Its arguments, each as JSON.
    args: Vec<String>,
}

/// Start listening.
pub fn serve(
    store_path: Option<&Path>,
    scope: &ScopeId,
    tool: &Which,
    args: &ServeArgs,
    floors: aeon_lua::Floors,
    loaded: &mut crate::loaded::Loaded,
) -> anyhow::Result<()> {
    let listener = Listener::bind(&args.instance)?;
    // Written whether or not anybody connects: a caller that finds no socket falls back to
    // spawning, and it can only do that if we left it an absolute path to spawn.
    let descriptor = aeon_ipc::tool_descriptor()?;

    eprintln!("{}", render::bold(&listener.path().display().to_string()));
    eprintln!("{}", render::dim(&format!("scope {scope}")));
    eprintln!(
        "{}",
        render::dim(&format!("descriptor {}", descriptor.display()))
    );

    // One store per tool, opened when that tool first speaks. A harness and a shell reaching
    // the same daemon are two memories, and neither is opened on the chance that it might be.
    let mut opened: std::collections::HashMap<aeon_store::Tool, Opened> =
        std::collections::HashMap::new();
    let fallback = tool.tool.clone();

    listener.serve(|peer: &Peer, request: Request| {
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
                        scratch: aeon_store::Scratchpad::at(runs_under(store_path, scope, &which)),
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
            now: now(),
            inject_floor: floors.inject,
            live_floor: floors.live,
        };
        aeon_host::answer_with(&mut at, &Door::Socket(peer.clone()), &request, |entry| {
            loaded.mask(entry)
        })
    })?;
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
    floors: aeon_lua::Floors,
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

    let reply = match open(store_path, scope, tool).and_then(|store| {
        crate::scrollback(store_path, scope, tool).map(|scrollback| (store, scrollback))
    }) {
        Ok((mut store, mut scrollback)) => {
            let mut at = Answering {
                store: &mut store,
                scrollback: Some(&mut scrollback),
                scratch: None,
                scope: scope.clone(),
                now: now(),
                inject_floor: floors.inject,
                live_floor: floors.live,
            };
            // One-shot is the owner's own door: it is this process, started by whoever ran it,
            // with no socket in between and nobody else to attribute it to.
            aeon_host::answer_with(&mut at, &Door::Owner, &request, |entry| loaded.mask(entry))
        }
        Err(why) => Reply::refused(why.to_string()),
    };

    println!("{}", serde_json::to_string(&reply)?);
    Ok(())
}

/// Print the client library, for a program that needs to embed it.
pub fn lua_api() {
    print!("{}", aeon_lua::CLIENT);
}

/// One tool's memory, held open for as long as the daemon is.
struct Opened {
    store: aeon_store::Store,
    scrollback: aeon_store::Transcript,
    scratch: aeon_store::Scratchpad,
}

/// Which tool a connection belongs to, as the kernel names it.
///
/// This is the whole reason a tool dimension is safe: the caller is not asked, so it cannot
/// answer wrongly. A peer the kernel will not name, or whose name nothing survives, falls back
/// to whatever the daemon was started as rather than being filed under a guess.
fn named_by_kernel(peer: &Peer) -> Option<aeon_store::Tool> {
    peer.program
        .as_deref()
        .and_then(|program| {
            Path::new(program)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .and_then(|name| aeon_store::Tool::from_program(&name))
}

