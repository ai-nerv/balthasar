//! What a harness installs, and what happens to it when balthasar is not there.
//!
//! Every call has to answer `nil` and let the harness carry on exactly as it did before.

use balthasar_lua::{CLIENT, Engine};
use std::path::PathBuf;

/// The drop-in a harness loads, as it ships.
fn shipped() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/harness/balthasar.lua");
    std::fs::read_to_string(&path).expect("balthasar ships a harness drop-in")
}

/// Run a script with the drop-in loaded, and take what it left.
fn through(script: &str) -> String {
    let mut engine = Engine::new();
    let source = format!(
        r#"
        local harness = assert(load({harness:?}, "harness.lua"))()
        balthasar.answer = tostring({script})
        "#,
        harness = shipped()
    );
    engine
        .run(&source, "probe.lua")
        .expect("the drop-in must load");
    engine.harvest();
    engine
        .config()
        .string("answer")
        .expect("an answer")
        .to_owned()
}

#[test]
fn the_drop_in_loads() {
    assert_eq!(through("harness._NAME"), "balthasar.harness");
}

#[test]
fn nothing_is_live_before_anything_connects() {
    assert_eq!(through("harness.live()"), "false");
}

#[test]
fn observing_with_no_balthasar_is_not_an_error() {
    // Fire and forget. What is lost is one turn's observation, and the harness still has its
    // own transcript to backfill from later.
    assert_eq!(through("harness.observe('s', {})"), "false");
}

#[test]
fn planning_with_no_balthasar_answers_nothing() {
    // Not an error to handle, an absence to carry on through.
    assert_eq!(through("harness.plan('s', {}) == nil"), "true");
}

#[test]
fn every_call_survives_balthasar_being_absent() {
    for call in [
        "harness.plan('s', {})",
        "harness.recall('anything')",
        "harness.remember('a thing')",
    ] {
        assert_eq!(through(&format!("{call} == nil")), "true", "{call} raised");
    }
}

#[test]
fn opening_without_the_client_says_what_is_missing() {
    // The usual cause is that `balthasar configs` has not been run, and saying so is what makes
    // that fixable rather than mysterious.
    let why = through("select(2, harness.open({}))");
    assert!(why.contains("client library"), "{why}");
}

#[test]
fn a_failed_connection_is_not_retried_every_turn() {
    // A harness asks several times per turn; a connect attempt on each would be a syscall storm.
    let answer = through(&format!(
        "(function() \
               harness.open({{ source = {CLIENT:?}, where = {{ path = '/no/such/socket' }} }}) \
               local _, why = harness.open({{ source = {CLIENT:?} }}) \
               return why \
             end)()"
    ));
    assert!(answer.contains("earlier this session"), "{answer}");
}

#[test]
fn closing_when_nothing_is_open_is_harmless() {
    assert_eq!(
        through("(function() harness.close() return harness.live() end)()"),
        "false"
    );
}

/// A socket whose file goes when the test ends, however the test ends.
struct Bound(PathBuf);

impl Drop for Bound {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A peer that leaves the first call unanswered and serves the next connection properly — what a
/// harness meets when its first question arrives while balthasar is still opening its store.
fn slow_once() -> Bound {
    use std::io::{Read, Write};

    let path = std::env::temp_dir().join(format!("bh-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");

    std::thread::spawn(move || {
        let row = r#"{"ok":true,"result":[{"text":"a thing"}],"n":1}"#;
        let framed = |body: &str| {
            let mut out = (body.len() as u32).to_be_bytes().to_vec();
            out.extend_from_slice(body.as_bytes());
            out
        };
        let frame = |socket: &mut std::os::unix::net::UnixStream| -> Option<()> {
            let mut head = [0_u8; 4];
            socket.read_exact(&mut head).ok()?;
            let mut body = vec![0_u8; u32::from_be_bytes(head) as usize];
            socket.read_exact(&mut body).ok()?;
            Some(())
        };

        // Held open rather than dropped: a closed socket would be an end-of-file, and the case
        // under test is the one where the peer is simply too slow.
        let (mut first, _) = listener.accept().expect("accept");
        frame(&mut first).expect("the handshake");
        let _ = first.write_all(&framed(row));
        frame(&mut first).expect("the call it never answers");

        let (mut again, _) = listener.accept().expect("accept");
        while frame(&mut again).is_some() {
            let _ = again.write_all(&framed(row));
        }
    });
    Bound(path)
}

#[test]
fn a_call_that_timed_out_costs_a_connect_and_not_the_session() {
    // The client drops a handle it could not finish reading on, so the harness that holds one
    // has to dial again — otherwise one slow first call silently ends memory for the whole
    // session, which is worse than the desynchronisation it replaces.
    let bound = slow_once();
    let answer = through(&format!(
        "(function() \
               harness.open({{ source = {CLIENT:?}, transport = balthasar.stream, \
                 where = {{ path = {path:?}, timeout_ms = 200 }} }}) \
               local first = harness.recall('one') \
               local second = harness.recall('two') \
               return tostring(first) .. '/' .. tostring(second and second[1].text) \
             end)()",
        path = bound.0.to_string_lossy()
    ));
    assert_eq!(answer, "nil/a thing", "{answer}");
}

/// A peer that answers every call by naming every call it has been asked.
///
/// A harness verb that never reaches the wire and one the far end refused both leave the harness
/// with the same nothing, so proving the first requires asking the far end what it heard.
fn echoing() -> Bound {
    use std::io::{Read, Write};

    let path = std::env::temp_dir().join(format!("bh-echo-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");

    std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("accept");
        let mut heard: Vec<String> = Vec::new();
        loop {
            let mut head = [0_u8; 4];
            if socket.read_exact(&mut head).is_err() {
                return;
            }
            let mut body = vec![0_u8; u32::from_be_bytes(head) as usize];
            if socket.read_exact(&mut body).is_err() {
                return;
            }
            let body = String::from_utf8_lossy(&body).into_owned();
            let called = body
                .split_once("\"call\":\"")
                .and_then(|(_, rest)| rest.split_once('"'))
                .map(|(name, _)| name.to_owned())
                .unwrap_or_default();
            heard.push(called);
            let row = format!(
                r#"{{"ok":true,"result":[{{"heard":"{}"}}],"n":1}}"#,
                heard.join(",")
            );
            let mut out = (row.len() as u32).to_be_bytes().to_vec();
            out.extend_from_slice(row.as_bytes());
            if socket.write_all(&out).is_err() {
                return;
            }
        }
    });
    Bound(path)
}

#[test]
fn observing_and_planning_reach_balthasar_rather_than_dying_in_the_client() {
    // The two verbs a harness exists to call. Bound to the client's own surface table, neither
    // was there, and every `observe` a harness made was swallowed by the `pcall` around it.
    let bound = echoing();
    let answer = through(&format!(
        "(function() \
               harness.open({{ source = {CLIENT:?}, transport = balthasar.stream, \
                 where = {{ path = {path:?}, timeout_ms = 200 }} }}) \
               local kept = harness.observe('s', {{ text = 'a turn' }}) \
               local plan = harness.plan('s', {{}}) \
               return tostring(kept) .. '/' .. tostring(plan and plan.heard) \
             end)()",
        path = bound.0.to_string_lossy()
    ));
    // The connect handshake asks `verbs` first, so what matters is the tail.
    assert!(
        answer.starts_with("true/"),
        "the turn was not recorded: {answer}"
    );
    assert!(answer.ends_with("observe,plan"), "{answer}");
}

/// A peer that answers the handshake and refuses everything after it.
fn refusing() -> Bound {
    use std::io::{Read, Write};

    let path = std::env::temp_dir().join(format!("bh-no-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");

    std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("accept");
        let mut first = true;
        loop {
            let mut head = [0_u8; 4];
            if socket.read_exact(&mut head).is_err() {
                return;
            }
            let mut body = vec![0_u8; u32::from_be_bytes(head) as usize];
            if socket.read_exact(&mut body).is_err() {
                return;
            }
            let row = if std::mem::take(&mut first) {
                r#"{"ok":true,"result":["verbs"],"n":1}"#.to_owned()
            } else {
                r#"{"ok":false,"error":"this balthasar keeps no scrollback","fault":"failed"}"#
                    .to_owned()
            };
            let mut out = (row.len() as u32).to_be_bytes().to_vec();
            out.extend_from_slice(row.as_bytes());
            if socket.write_all(&out).is_err() {
                return;
            }
        }
    });
    Bound(path)
}

#[test]
fn a_refused_observation_says_the_turn_was_not_recorded() {
    // `observe` answers with no values, so a refusal arrives looking exactly like a success. A
    // harness told `true` here would carry on and write the next turn over a hole.
    let bound = refusing();
    let answer = through(&format!(
        "(function() \
               harness.open({{ source = {CLIENT:?}, transport = balthasar.stream, \
                 where = {{ path = {path:?}, timeout_ms = 200 }} }}) \
               return harness.observe('s', {{ text = 'a turn' }}) \
             end)()",
        path = bound.0.to_string_lossy()
    ));
    assert_eq!(answer, "false", "{answer}");
}
