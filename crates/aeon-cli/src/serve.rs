//! `aeon serve`, `aeon api` and `aeon lua-api` — the three ways in from outside.
//!
//! Both doors reach the same dispatcher, which reaches the same functions the CLI calls. That
//! is the arrangement that keeps a socket answer and a terminal answer from describing a
//! memory differently.

use crate::{now, open, render};
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

    let mut store = open(store_path, scope)?;
    listener.serve(|peer: &Peer, request: Request| {
        let mut at = Answering {
            store: &mut store,
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

    let reply = match open(store_path, scope) {
        Ok(mut store) => {
            let mut at = Answering {
                store: &mut store,
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
