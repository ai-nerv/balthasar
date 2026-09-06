//! The milestone's own acceptance test: the shipped stub, loaded into a real Lua VM, talking
//! to a real socket.
//!
//! Not balthasar calling itself. The stub goes into a VM as *source*, is handed nothing but the
//! socket primitive, and has to find, connect, frame, encode, decode and unpack on its own —
//! which is every layer a sibling would exercise, in the order a sibling would exercise them.

use balthasar_host::{Answering, Door};
use balthasar_ipc::{Listener, Peer, Request};
use balthasar_lua::{CLIENT, Engine};
use balthasar_model::{ScopeId, floor};
use balthasar_store::Store;

const NOW: balthasar_model::Timestamp = 1_756_000_000;

/// Serve a seeded store on a socket, for as long as the handle lives.
fn serving(name: &str, seed: &[&str]) -> (std::path::PathBuf, std::thread::JoinHandle<()>) {
    let instance = format!("stub-{name}-{}", std::process::id());
    let listener = Listener::bind(&instance).expect("bind");
    let path = listener.path().to_owned();

    let mut store = Store::ephemeral().expect("store");
    for text in seed {
        let mut at = Answering {
            store: &mut store,
            scrollback: None,
            scratch: None,
            scope: ScopeId::new("/w/thing"),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        };
        balthasar_host::answer(
            &mut at,
            &Door::Owner,
            &Request {
                call: "remember".into(),
                args: vec![serde_json::json!(text)],
            },
        );
    }

    let thread = std::thread::spawn(move || {
        let _ = listener.serve(|peer: &Peer, request: Request| {
            let mut at = Answering {
                store: &mut store,
                scrollback: None,
                scratch: None,
                scope: ScopeId::new("/w/thing"),
                now: NOW,
                inject_floor: floor::INJECT,
                live_floor: floor::LIVE,
                capture: false,
            };
            balthasar_host::answer(&mut at, &Door::Socket(peer.clone()), &request)
        });
    });
    (path, thread)
}

/// Run a script with the stub loaded and connected, and take what it left in `balthasar.answer`.
fn through_the_stub(path: &std::path::Path, script: &str) -> String {
    let mut engine = Engine::new();
    let source = format!(
        r#"
        local chunk = assert(load({CLIENT:?}, "balthasar.lua"))
        -- Given nothing but the socket primitive, exactly as a sibling would hand it over.
        local client = chunk(balthasar.stream)
        local mem, why = client.connect({{ path = {:?} }})
        if not mem then balthasar.answer = "could not connect: " .. tostring(why) return end
        balthasar.answer = tostring({script})
        mem:close()
        "#,
        path.to_string_lossy()
    );
    engine.run(&source, "stub.lua").expect("the stub must run");
    engine.harvest();
    engine
        .config()
        .string("answer")
        .expect("an answer")
        .to_owned()
}

#[test]
fn the_stub_connects_and_is_answered() {
    let (path, _thread) = serving("connect", &["we deploy with fly"]);
    assert_eq!(through_the_stub(&path, "mem.verbs() ~= nil"), "true");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_stub_recalls_and_unpacks_what_came_back() {
    // The reply shape, end to end. A client that unpacked a bare-value server would read this
    // as having returned nothing at all, and the bug would present as an empty memory.
    let (path, _thread) = serving("recall", &["we deploy with fly"]);
    let answer = through_the_stub(&path, "mem.recall(\"deploy\")[1].text");
    assert_eq!(answer, "we deploy with fly");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_stub_gets_which_project_and_which_run() {
    let (path, _thread) = serving("origin", &["we deploy with fly"]);
    let answer = through_the_stub(&path, "mem.recall(\"deploy\")[1].project");
    assert_eq!(answer, "/w/thing");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn one_stub_connection_carries_several_calls() {
    // The deviation from the sibling that reconnects per call, proven rather than asserted.
    let (path, _thread) = serving("many", &["a thing"]);
    let answer = through_the_stub(
        &path,
        "(mem.verbs() and mem.status() and mem.sessions()) ~= nil",
    );
    assert_eq!(answer, "true");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_stub_reports_a_refusal_rather_than_raising() {
    // A refused verb has to arrive as a value the caller can branch on. A client that raised
    // would take down whatever was using it over a question it was entitled to ask.
    let (path, _thread) = serving("refuse", &[]);
    let answer = through_the_stub(
        &path,
        "select(2, mem:call(\"prompt\", \"rm -rf /\")) ~= nil",
    );
    assert_eq!(answer, "true");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_stub_says_so_when_nothing_is_listening() {
    // The ordinary state of a machine where no daemon was started, and it must read as that
    // rather than as a crash.
    let mut engine = Engine::new();
    let source = format!(
        r#"
        local chunk = assert(load({CLIENT:?}, "balthasar.lua"))
        local client = chunk(balthasar.stream)
        local mem, why = client.connect({{ path = "/no/such/socket" }})
        balthasar.answer = tostring(mem == nil and type(why) == "string")
        "#
    );
    engine.run(&source, "stub.lua").expect("run");
    engine.harvest();
    assert_eq!(engine.config().string("answer"), Some("true"));
}

#[test]
fn the_library_that_speaks_this_surface_comes_back_over_the_wire() {
    // **A consumer keeping its own copy is a consumer whose copy goes stale**, and one did:
    // magi's copy of this file predated a fix, so every session on that machine silently had no
    // memory tools and nothing anywhere said why. `balthasar lua-api` prints the same source,
    // which is enough for a host that can shell out and useless to a sandboxed VM that cannot.
    let (path, _thread) = serving("client", &[]);
    let source = through_the_stub(&path, r#"select(1, mem:call("client"))"#);
    assert!(
        source.contains("balthasar's client library"),
        "it is the file this crate ships: {source:.120}"
    );
    let _ = std::fs::remove_file(&path);
}
