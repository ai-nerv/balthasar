#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_host::{Answering, Describing, Door, Ownership, answer, answer_scoped_hooked};
use balthasar_ipc::{ConnectionState, Peer, Reply, Request};
use balthasar_model::{AgentId, ScopeId, SessionId, floor, scratch::Scratch};
use balthasar_store::{Store, Transcript};
use serde_json::{Value, json};

struct Held {
    store: Store,
    transcript: Transcript,
}

impl Held {
    fn open(root: &Scratch) -> Self {
        Self {
            store: Store::ephemeral().expect("memory"),
            transcript: Transcript::open(&root.join("transcript.db")).expect("transcript"),
        }
    }

    fn ask(&mut self, state: Option<&mut Ownership>, call: &str, args: Vec<Value>) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: Some(&mut self.transcript),
            scratch: None,
            scope: ScopeId::new("/project"),
            agent: AgentId::main(),
            now: 100,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        };
        let request = Request {
            call: call.into(),
            args,
        };
        match state {
            Some(state) => answer_scoped_hooked(
                &mut at,
                &peer(),
                &request,
                &mut Describing(|_: &balthasar_store::Turn| None),
                state,
            ),
            None => answer(&mut at, &Door::Owner, &request),
        }
    }
}

fn peer() -> Door {
    Door::Socket(Peer {
        pid: 42,
        uid: 1000,
        program: Some("harness".into()),
        agent: None,
    })
}

fn control(key: &str, operation: &str) -> Vec<Value> {
    vec![json!(key), json!({ "revision": 1, "operation": operation })]
}

fn turn(key: &str, cursor: u64, text: &str) -> Vec<Value> {
    vec![
        json!(key),
        json!({ "cursor": cursor, "role": "user", "text": text, "at": 100 }),
    ]
}

#[test]
fn one_connection_reserves_before_replay_and_fences_legacy_and_other_connections() {
    let root = Scratch::new("writer", "host-fence");
    let mut a = Held::open(&root);
    let mut b = Held::open(&root);
    let (mut owner, mut competitor) = (Ownership::default(), Ownership::default());
    assert!(a.ask(None, "observe", turn("run", 0, "before")).ok);
    assert!(
        a.ask(Some(&mut owner), "writer", control("run", "reserve"))
            .ok
    );
    assert!(owner.retains_connection());
    assert!(
        !b.ask(Some(&mut competitor), "writer", control("run", "reserve"))
            .ok
    );
    assert!(!competitor.retains_connection());
    let blocked = b.ask(None, "observe", turn("run", 1, "legacy"));
    assert!(!blocked.ok);
    assert_eq!(blocked.fault, Some(balthasar_ipc::Fault::Failed));
    assert!(!b.ask(None, "amend", turn("run", 0, "changed")).ok);
    assert!(
        !b.ask(Some(&mut competitor), "observe", turn("run", 1, "other"))
            .ok
    );
    for verb in [
        "model",
        "layout",
        "applied",
        "overflowed",
        "jobs",
        "job_done",
    ] {
        let refused = b.ask(None, verb, vec![json!("run"), json!({})]);
        assert!(!refused.ok);
        assert!(
            refused
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("writer"),
            "{verb}: {refused:?}"
        );
    }
    let replay = a.ask(Some(&mut owner), "replay", vec![json!("run")]);
    assert!(replay.ok);
    assert_eq!(replay.result.len(), 1);
    assert_eq!(replay.result[0]["text"], "before");
    assert!(
        a.ask(Some(&mut owner), "observe", turn("run", 1, "owned"))
            .ok
    );
    assert!(b.ask(None, "observe", turn("independent", 0, "free")).ok);
    assert!(
        a.ask(Some(&mut owner), "writer", control("run", "reserve"))
            .ok
    );
    assert!(
        a.ask(Some(&mut owner), "writer", control("run", "release"))
            .ok
    );
    assert!(!owner.retains_connection());
    assert!(
        b.ask(Some(&mut competitor), "writer", control("run", "reserve"))
            .ok
    );
    drop(competitor);
    assert!(
        a.ask(None, "observe", turn("run", 2, "after disconnect"))
            .ok
    );
}

#[test]
fn failed_control_or_wrong_store_never_releases_the_previous_writer() {
    let root = Scratch::new("writer", "host-preserve");
    let other_root = Scratch::new("writer", "host-other-store");
    let mut held = Held::open(&root);
    let mut other = Held::open(&other_root);
    let mut owner = Ownership::default();
    assert!(
        held.ask(Some(&mut owner), "writer", control("run", "reserve"))
            .ok
    );
    for operation in ["reserve", "release", "unknown"] {
        assert!(
            !held
                .ask(Some(&mut owner), "writer", control("wrong", operation))
                .ok
        );
    }
    for operation in ["reserve", "release"] {
        assert!(
            !other
                .ask(Some(&mut owner), "writer", control("run", operation))
                .ok
        );
    }
    assert!(
        !other
            .ask(Some(&mut owner), "observe", turn("run", 0, "wrong store"))
            .ok
    );
    assert!(
        !held
            .ask(Some(&mut owner), "observe", turn("wrong", 0, "wrong key"))
            .ok
    );
    assert!(!held.ask(None, "observe", turn("run", 0, "blocked")).ok);
    assert!(owner.retains_connection());
    assert!(
        held.ask(Some(&mut owner), "observe", turn("run", 0, "safe"))
            .ok
    );
}

#[test]
fn capability_is_connection_local_and_one_shot_cannot_claim_ownership() {
    let root = Scratch::new("writer", "host-capability");
    let mut held = Held::open(&root);
    let (mut a, mut b) = (Ownership::default(), Ownership::default());
    let status = || vec![json!("run"), json!({ "revision": 1 })];
    assert_eq!(
        held.ask(None, "writer_status", status()).result[0]["supported"],
        false
    );
    assert!(!held.ask(None, "writer", control("run", "reserve")).ok);
    let ready = held.ask(Some(&mut a), "writer_status", status());
    assert_eq!(ready.result[0]["supported"], true);
    assert_eq!(ready.result[0]["owned"], false);
    assert!(
        held.ask(Some(&mut a), "writer", control("run", "reserve"))
            .ok
    );
    assert_eq!(
        held.ask(Some(&mut a), "writer_status", status()).result[0]["owned"],
        true
    );
    assert_eq!(
        held.ask(Some(&mut b), "writer_status", status()).result[0]["owned"],
        false
    );
    drop(a);
    held.transcript = Transcript::ephemeral().expect("ephemeral");
    assert_eq!(
        held.ask(Some(&mut b), "writer_status", status()).result[0]["supported"],
        false
    );
    assert!(
        !held
            .ask(Some(&mut b), "writer", control("run", "reserve"))
            .ok
    );
    assert!(
        held.ask(None, "observe", turn("run", 0, "ephemeral unchanged"))
            .ok
    );
}

#[test]
fn malformed_control_cannot_mutate_ownership() {
    let root = Scratch::new("writer", "host-malformed");
    let mut held = Held::open(&root);
    let mut state = Ownership::default();
    for options in [
        json!({}),
        json!({"revision":2,"operation":"reserve"}),
        json!({"revision":1,"operation":"reserve","extra":true}),
        json!({"revision":1,"operation":3}),
        json!({"revision":1}),
    ] {
        assert!(
            !held
                .ask(Some(&mut state), "writer", vec![json!("run"), options])
                .ok
        );
        assert!(!state.retains_connection());
    }
    for key in [String::new(), "x".repeat(1025)] {
        assert!(
            !held
                .ask(Some(&mut state), "writer", control(&key, "reserve"))
                .ok
        );
    }
    assert!(
        !held
            .ask(Some(&mut state), "writer_status", control("run", "reserve"))
            .ok
    );
    assert!(!held.ask(Some(&mut state), "writer", vec![json!("run")]).ok);
}

#[test]
fn retention_skips_a_reserved_transcript_and_purge_refuses_the_whole_run() {
    let root = Scratch::new("writer", "host-retention");
    let mut held = Held::open(&root);
    let mut state = Ownership::default();
    for key in ["run", "child", "quiet"] {
        let mut args = turn(key, 0, key);
        if key == "child" {
            args[1]["run"] = json!("run");
        }
        assert!(held.ask(None, "observe", args).ok);
    }
    assert!(
        held.ask(Some(&mut state), "writer", control("child", "reserve"))
            .ok
    );
    let run = SessionId::new("run");
    let child = SessionId::new("child");
    assert!(balthasar_store::purge_run(&held.transcript, &run).is_err());
    assert_eq!(held.transcript.count(&run).expect("run retained"), 1);
    assert_eq!(held.transcript.count(&child).expect("child retained"), 1);
    let memories = held.store.owned_by(&run).expect("memories");
    assert!(!memories.is_empty());
    let refused = held.ask(
        None,
        "forget",
        vec![json!("run"), json!({"session":true,"purge":true})],
    );
    assert!(!refused.ok);
    assert_eq!(
        held.store.owned_by(&run).expect("unchanged memories"),
        memories
    );
    let pruned =
        balthasar_store::prune_turns(&held.transcript, None, None, 200, &[]).expect("retention");
    assert_eq!(pruned.sessions, 2);
    assert_eq!(held.transcript.count(&child).expect("reserved child"), 1);
    assert_eq!(held.transcript.count(&run).expect("unreserved parent"), 0);
    drop(state);
    let pruned = balthasar_store::prune_turns(&held.transcript, None, None, 200, &[])
        .expect("released retention");
    assert_eq!(pruned.sessions, 1);
    assert_eq!(held.transcript.count(&child).expect("removed child"), 0);
}

#[test]
fn replaced_lock_identity_fails_closed_for_reads_writes_and_release() {
    use sha2::Digest;
    let root = Scratch::new("writer", "host-replaced");
    let mut held = Held::open(&root);
    let mut state = Ownership::default();
    assert!(
        held.ask(Some(&mut state), "writer", control("run", "reserve"))
            .ok
    );
    let locks = root.join("transcript.db.writers");
    let file = locks.join(format!("{:x}.lock", sha2::Sha256::digest(b"run")));
    std::fs::rename(&file, root.join("retained.lock")).expect("replace lock path");
    for verb in ["observe", "replay", "resume", "scroll"] {
        assert!(
            !held
                .ask(Some(&mut state), verb, turn("run", 0, "refused"))
                .ok
        );
    }
    assert!(
        !held
            .ask(Some(&mut state), "writer", control("run", "release"))
            .ok
    );
    assert!(state.retains_connection());
    assert_eq!(
        held.transcript
            .count(&SessionId::new("run"))
            .expect("no mutation"),
        0
    );
}
