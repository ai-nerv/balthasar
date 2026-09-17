use balthasar_host::{Answering, Door, answer};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{AgentId, ScopeId, SessionId, floor, scratch::Scratch};
use balthasar_store::{Scratchpad, Store, Transcript};
use serde_json::{Value, json};

struct Held {
    store: Store,
    transcript: Transcript,
    scratch: Scratchpad,
    _dir: Scratch,
}

impl Held {
    fn new() -> Self {
        let dir = Scratch::new("binding", "runs");
        Self {
            store: Store::ephemeral().expect("store"),
            transcript: Transcript::open(&dir.join("transcript.db")).expect("transcript"),
            scratch: Scratchpad::at(dir.join("runs")),
            _dir: dir,
        }
    }

    fn ask(&mut self, agent: &str, call: &str, args: Vec<Value>) -> Reply {
        answer(
            &mut Answering {
                store: &mut self.store,
                scrollback: Some(&mut self.transcript),
                scratch: Some(&mut self.scratch),
                scope: ScopeId::new("/project"),
                agent: AgentId::new(agent),
                now: 1_756_000_000,
                inject_floor: floor::INJECT,
                live_floor: floor::LIVE,
                capture: false,
            },
            &Door::Owner,
            &Request {
                call: call.into(),
                args,
            },
        )
    }

    fn observe(&mut self, agent: &str, transcript: &str, run: &str) -> Reply {
        self.ask(
            agent,
            "observe",
            vec![
                json!(transcript),
                json!({
                    "cursor": 0, "role": "user", "text": agent, "run": run,
                }),
            ],
        )
    }
}

#[test]
fn independent_transcripts_share_run_directories_not_agent_scratch() {
    let mut held = Held::new();
    for (agent, transcript) in [("parent", "run"), ("child", "opaque-child-key")] {
        assert!(held.observe(agent, transcript, "run").ok);
        let path = held
            .scratch
            .path_of(&SessionId::new("run"), &AgentId::new(agent));
        assert!(
            path.is_file(),
            "missing shared-run scratch: {}",
            path.display()
        );
        let rows = held.ask(agent, "replay", vec![json!(transcript)]);
        assert_eq!(rows.result[0]["text"], agent);
        let store = held
            .scratch
            .of(&SessionId::new("run"), &AgentId::new(agent))
            .expect("agent scratch");
        assert_eq!(
            store
                .owned_by(&SessionId::new("run"))
                .expect("owned memories")
                .len(),
            1
        );
    }
    assert_eq!(held.scratch.runs().len(), 2);
}

#[test]
fn layout_recovery_stays_with_the_bound_run_and_original_agent() {
    let mut held = Held::new();
    let run = SessionId::new("run");
    let transcript = SessionId::new("opaque-child-key");
    let agent = AgentId::new("child");
    let scope = ScopeId::new("/project");
    assert!(held.observe("child", transcript.as_str(), run.as_str()).ok);
    assert!(held.observe("parent", "parent-transcript", run.as_str()).ok);
    let source = held
        .scratch
        .of(&run, &agent)
        .expect("child scratch")
        .scratch_for(&scope, &run, "child")
        .expect("source")
        .expect("kept");
    let id = held
        .transcript
        .propose(
            &transcript,
            "test",
            10,
            &json!({"slots":[{"kind":"summary","covers":[0,0]}]}),
            &json!({}),
            1,
        )
        .expect("layout");
    let faults =
        rusqlite::Connection::open(held.scratch.path_of(&run, &agent)).expect("child faults");
    faults.execute_batch("CREATE TRIGGER fail_child BEFORE INSERT ON witness BEGIN SELECT RAISE(ABORT, 'child witness failed'); END;").expect("fault");
    let args = vec![json!(transcript.as_str()), json!({"id":id})];
    assert!(!held.ask("child", "applied", args.clone()).ok);
    assert_eq!(
        held.transcript
            .layout_effects(&transcript, &agent, &scope)
            .expect("pending")
            .len(),
        1
    );
    assert!(held.ask("parent", "applied", args.clone()).ok);
    assert_eq!(
        held.transcript
            .layout_effects(&transcript, &agent, &scope)
            .expect("still pending")
            .len(),
        1
    );
    let home = held.scratch.home().to_owned();
    held.scratch = Scratchpad::at(home);
    held.transcript = Transcript::open(held.transcript.path()).expect("reopen");
    faults
        .execute_batch("DROP TRIGGER fail_child")
        .expect("remove fault");
    assert!(held.ask("child", "applied", args).ok);
    let witnesses = held
        .scratch
        .of(&run, &agent)
        .expect("child scratch")
        .witnesses_of(&source)
        .expect("witnesses");
    assert_eq!(witnesses.len(), 1);
    assert_eq!(witnesses[0].session, run);
    assert!(
        held.transcript
            .layout_effects(&transcript, &agent, &scope)
            .expect("completed")
            .is_empty()
    );
    let parent = held
        .scratch
        .of(&run, &AgentId::new("parent"))
        .expect("parent scratch");
    let parent_source = parent
        .scratch_for(&scope, &run, "parent")
        .expect("source")
        .expect("kept");
    assert!(
        parent
            .witnesses_of(&parent_source)
            .expect("parent evidence")
            .is_empty()
    );
}

#[test]
fn transcript_run_binding_survives_restart_and_refuses_reassignment() {
    let mut held = Held::new();
    assert!(held.observe("child", "opaque-child-key", "run").ok);
    held.transcript = Transcript::open(held.transcript.path()).expect("reopen transcript");
    let resumed = held.ask("child", "resume", vec![json!("opaque-child-key")]);
    assert_eq!(resumed.result[0]["run"], "run");
    assert!(!held.observe("child", "opaque-child-key", "another-run").ok);
    let rows = held.ask("child", "replay", vec![json!("opaque-child-key")]);
    assert_eq!(rows.result.len(), 1);
    assert_eq!(held.scratch.runs().len(), 1);
}

#[test]
fn omitted_binding_on_amend_retains_the_run_and_cannot_redirect_it() {
    let mut held = Held::new();
    assert!(held.observe("child", "opaque-child-key", "run").ok);
    for run in [json!(42), json!(""), json!("different")] {
        let reply = held.ask(
            "child",
            "amend",
            vec![
                json!("opaque-child-key"),
                json!({
                    "cursor": 0, "role": "user", "text": "wrong", "run": run,
                }),
            ],
        );
        assert!(!reply.ok);
    }
    assert!(
        held.ask(
            "child",
            "amend",
            vec![
                json!("opaque-child-key"),
                json!({
                    "cursor": 0, "role": "user", "text": "revised",
                })
            ]
        )
        .ok
    );
    let rows = held.ask("child", "replay", vec![json!("opaque-child-key")]);
    assert_eq!(rows.result[0]["text"], "revised");
    assert_eq!(
        held.transcript
            .run_of(&SessionId::new("opaque-child-key"))
            .expect("run binding"),
        SessionId::new("run")
    );
}

#[test]
fn legacy_transcripts_keep_their_scratch_identity() {
    let mut held = Held::new();
    let session = SessionId::new("legacy");
    held.transcript
        .write(&session, &balthasar_store::Turn::default())
        .expect("legacy turn");
    assert!(held.observe("parent", "legacy", "legacy").ok);
    assert!(!held.observe("parent", "legacy", "new").ok);
    let reply = held.ask(
        "old-client",
        "observe",
        vec![
            json!("unbound"),
            json!({
                "cursor": 0, "text": "original protocol",
            }),
        ],
    );
    assert!(reply.ok);
    assert!(
        held.scratch
            .path_of(&SessionId::new("unbound"), &AgentId::new("old-client"))
            .is_file()
    );
}

#[test]
fn purging_a_run_removes_all_its_transcripts_but_not_another_run() {
    let mut held = Held::new();
    assert!(held.observe("parent", "run", "run").ok);
    assert!(held.observe("child", "opaque-child-key", "run").ok);
    assert!(held.observe("other", "elsewhere", "elsewhere").ok);
    let gone =
        balthasar_store::purge_run(&held.transcript, &SessionId::new("run")).expect("purge run");
    assert_eq!(gone, 2);
    for id in ["run", "opaque-child-key"] {
        assert!(
            held.transcript
                .replay(&SessionId::new(id))
                .expect("purged transcript")
                .is_empty()
        );
    }
    assert_eq!(
        held.transcript
            .replay(&SessionId::new("elsewhere"))
            .expect("other transcript")
            .len(),
        1
    );
    assert!(held.observe("child", "opaque-child-key", "new-run").ok);
}

#[test]
fn concurrent_agents_bind_without_read_to_write_lock_failures() {
    let held = Held::new();
    let connections: Vec<_> = (0..4)
        .map(|_| Transcript::open(held.transcript.path()).expect("connection"))
        .collect();
    let barrier = std::sync::Barrier::new(connections.len());
    std::thread::scope(|scope| {
        let workers: Vec<_> = connections
            .into_iter()
            .enumerate()
            .map(|(agent, transcript)| {
                let barrier = &barrier;
                scope.spawn(move || {
                    let mut failures = Vec::new();
                    for round in 0..20 {
                        barrier.wait();
                        let key = SessionId::new(format!("agent-{agent}-{round}"));
                        if let Err(why) = transcript.bind_run(&key, Some("shared")) {
                            failures.push(why.to_string());
                        }
                    }
                    failures
                })
            })
            .collect();
        let failures: Vec<_> = workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("worker"))
            .collect();
        assert!(failures.is_empty(), "binding failures: {failures:?}");
    });
}
