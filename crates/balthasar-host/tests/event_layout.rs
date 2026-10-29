#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_host::{Answering, Door, answer, events::*};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{
    AgentId, MemoryId, ScopeId, SessionId, WitnessKind, floor, scratch::Scratch,
};
use balthasar_store::{State, Store, Transcript};
use serde_json::{Value, json};
use std::{sync::mpsc, time::Duration};

const SESSION: &str = "private-session-canary";
const SCOPE: &str = "/private/path-canary";
const MODEL: &str = "private-model-canary";
const NOW: i64 = 1_756_000_000;

struct Harness {
    store: Store,
    transcript: Transcript,
    memories: Vec<MemoryId>,
    root: Scratch,
}

impl Harness {
    fn new() -> Self {
        let root = Scratch::new("event-layout", "native");
        let mut harness = Self {
            store: Store::open(&root.join("memory.db")).expect("memory"),
            transcript: Transcript::open(&root.join("transcript.db")).expect("transcript"),
            memories: Vec::new(),
            root,
        };
        for cursor in 0..2 {
            let text = format!("private-answer-canary {cursor}");
            assert!(
                harness
                    .ask(
                        "observe",
                        json!({"cursor":cursor,"role":"user","kind":"user","text":text})
                    )
                    .ok
            );
            harness.memories.push(
                harness
                    .store
                    .scratch_for(&ScopeId::new(SCOPE), &SessionId::new(SESSION), &text)
                    .expect("scratch")
                    .expect("kept"),
            );
        }
        harness
    }

    fn ask(&mut self, call: &str, second: Value) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: Some(&mut self.transcript),
            scratch: None,
            scope: ScopeId::new(SCOPE),
            agent: AgentId::main(),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        };
        answer(
            &mut at,
            &Door::Owner,
            &Request {
                call: call.into(),
                args: vec![json!(SESSION), second],
            },
        )
    }

    fn propose(&self) -> String {
        self.transcript.propose(
            &SessionId::new(SESSION), MODEL, 100,
            &json!({"slots":[{"kind":"summary","covers":[0,1],"text":"private-answer-canary"}]}),
            &json!({}), NOW,
        ).expect("proposal")
    }

    fn apply(&mut self, id: &str) -> Reply {
        self.ask("applied", json!({"id":id,"usage":{"input":120}}))
    }

    fn applied(&self, id: &str) -> bool {
        self.transcript
            .proposal(id)
            .expect("proposal read")
            .expect("proposal exists")
            .applied
            .is_some()
    }

    fn pending(&self) -> usize {
        self.transcript
            .layout_effects(
                &SessionId::new(SESSION),
                &AgentId::main(),
                &ScopeId::new(SCOPE),
            )
            .expect("pending effects")
            .len()
    }

    fn witnesses(&self) -> Vec<usize> {
        self.memories
            .iter()
            .map(|id| {
                self.store
                    .witnesses_of(id)
                    .expect("witnesses")
                    .iter()
                    .filter(|w| w.kind == WitnessKind::Distillation)
                    .count()
            })
            .collect()
    }

    fn fault(&self, sql: &str) -> rusqlite::Connection {
        let connection =
            rusqlite::Connection::open(self.root.join("transcript.db")).expect("fault connection");
        connection.execute_batch(sql).expect("owned fault");
        connection
    }

    fn reopen(&mut self) {
        self.store = Store::open(&self.root.join("memory.db")).expect("reopen memory");
        self.transcript =
            Transcript::open(&self.root.join("transcript.db")).expect("reopen transcript");
    }
}

fn observed(receiver: &Receiver, expected: &[LayoutOutcome]) {
    for outcome in expected {
        let bytes = receiver.try_recv().expect("actual layout event");
        let text = std::str::from_utf8(&bytes).expect("UTF8");
        for canary in [
            SESSION,
            SCOPE,
            MODEL,
            "private-answer-canary",
            "private-error-canary",
            "L-",
        ] {
            assert!(!text.contains(canary), "leaked {canary}");
        }
        let event = decode(&bytes).expect("strict event");
        assert_eq!(event.correlation, Correlation::default());
        assert_eq!(
            event.fact,
            Fact::Layout {
                outcome: *outcome,
                duration_ms: None
            }
        );
    }
    assert!(receiver.try_recv().is_err(), "unexpected extra event");
}

fn decisions_and_refusals(receiver: &Receiver) {
    let mut harness = Harness::new();
    let reply = harness.ask(
        "layout",
        json!({"query":"private-answer-canary","helpers":[]}),
    );
    assert!(reply.ok, "{:?}", reply.error);
    let laid = reply.result.first().expect("decision");
    let id = laid["id"].as_str().expect("layout ID");
    assert!(!harness.applied(id));
    observed(receiver, &[LayoutOutcome::Decided]);
    assert!(!harness.ask("applied", json!({})).ok);
    observed(receiver, &[LayoutOutcome::ApplicationFailed]);
    assert!(!harness.apply("missing-layout-canary").ok);
    observed(receiver, &[LayoutOutcome::ApplicationFailed]);
    assert!(
        !harness
            .ask("overflowed", json!({"id":"missing-layout-canary"}))
            .ok
    );
    observed(receiver, &[LayoutOutcome::Failed]);
    let reply = harness.ask("overflowed", json!({"id":id,"said":"private-error-canary"}));
    assert!(reply.ok, "{:?}", reply.error);
    observed(receiver, &[LayoutOutcome::Decided]);
    let id = harness.propose();
    assert!(harness.apply(&id).ok);
    assert!(harness.applied(&id));
    assert_eq!(harness.pending(), 0);
    assert_eq!(harness.witnesses(), [1, 1]);
    observed(
        receiver,
        &[LayoutOutcome::Applied, LayoutOutcome::Recovered],
    );
    assert!(harness.apply(&id).ok);
    observed(receiver, &[LayoutOutcome::IgnoredRepeated]);
    assert_eq!(harness.witnesses(), [1, 1]);
}

fn rollback(receiver: &Receiver) {
    let mut harness = Harness::new();
    let id = harness.propose();
    let faults = harness.fault("CREATE TRIGGER fail_enqueue BEFORE INSERT ON layout_effect WHEN NEW.cursor = 1 BEGIN SELECT RAISE(ABORT, 'private-error-canary'); END;");
    assert!(!harness.apply(&id).ok);
    harness.reopen();
    assert!(!harness.applied(&id));
    assert_eq!(harness.pending(), 0);
    assert_eq!(harness.witnesses(), [0, 0]);
    assert!(
        harness
            .transcript
            .replay(&SessionId::new(SESSION))
            .expect("rows")
            .iter()
            .all(|row| row.state == State::Live)
    );
    assert!(
        harness
            .transcript
            .measured(MODEL)
            .expect("factor")
            .is_none()
    );
    observed(receiver, &[LayoutOutcome::ApplicationFailed]);
    faults
        .execute_batch("DROP TRIGGER fail_enqueue")
        .expect("remove fault");
    assert!(harness.apply(&id).ok);
    observed(
        receiver,
        &[LayoutOutcome::Applied, LayoutOutcome::Recovered],
    );
}

fn partial_recovery(receiver: &Receiver, retry_application: bool) {
    let mut harness = Harness::new();
    let id = harness.propose();
    let faults = harness.fault("CREATE TRIGGER fail_ack BEFORE UPDATE OF done ON layout_effect WHEN NEW.cursor = 1 BEGIN SELECT RAISE(ABORT, 'private-error-canary'); END;");
    assert!(!harness.apply(&id).ok);
    harness.reopen();
    assert!(harness.applied(&id));
    assert_eq!(harness.pending(), 1);
    assert_eq!(harness.witnesses(), [1, 1]);
    assert!(
        harness
            .transcript
            .replay(&SessionId::new(SESSION))
            .expect("rows")
            .iter()
            .all(|row| row.state == State::Summarised)
    );
    observed(
        receiver,
        &[LayoutOutcome::Applied, LayoutOutcome::RecoveryFailed],
    );
    faults
        .execute_batch("DROP TRIGGER fail_ack")
        .expect("remove fault");
    if retry_application {
        assert!(harness.apply(&id).ok);
        observed(
            receiver,
            &[LayoutOutcome::IgnoredRepeated, LayoutOutcome::Recovered],
        );
    } else {
        assert!(harness.ask("layout", json!({"helpers":[]})).ok);
        observed(
            receiver,
            &[LayoutOutcome::Recovered, LayoutOutcome::Decided],
        );
    }
    assert_eq!(harness.pending(), 0);
    assert_eq!(harness.witnesses(), [1, 1]);
    assert!(harness.apply(&id).ok);
    observed(receiver, &[LayoutOutcome::IgnoredRepeated]);
}

fn completed() {
    let mut harness = Harness::new();
    let id = harness.propose();
    assert!(harness.apply(&id).ok);
    assert!(harness.applied(&id));
    assert_eq!(harness.pending(), 0);
    assert_eq!(harness.witnesses(), [1, 1]);
}

#[test]
fn actual_layout_events_distinguish_commit_repetition_and_recovery() {
    completed();
    assert_eq!(loss(), Loss::default());
    let receiver = enable_local().expect("explicit isolated activation");
    decisions_and_refusals(&receiver);
    rollback(&receiver);
    partial_recovery(&receiver, true);
    partial_recovery(&receiver, false);
    for _ in 0..QUEUED_EVENTS {
        assert_eq!(
            emit(|| (
                Correlation::default(),
                Fact::Memory {
                    outcome: MemoryOutcome::Read
                }
            )),
            Emitted::Queued
        );
    }
    let (done, completion) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        completed();
        done.send(()).expect("completed");
    });
    completion
        .recv_timeout(Duration::from_secs(5))
        .expect("application must complete while consumer is stalled");
    worker.join().expect("joined");
    assert_eq!(loss().full, 2);
    for _ in 0..QUEUED_EVENTS {
        assert_eq!(
            decode(&receiver.try_recv().expect("filler"))
                .expect("schema")
                .fact,
            Fact::Memory {
                outcome: MemoryOutcome::Read
            }
        );
    }
    completed();
    observed(
        &receiver,
        &[LayoutOutcome::Applied, LayoutOutcome::Recovered],
    );
    drop(receiver);
    completed();
    assert_eq!(
        loss(),
        Loss {
            full: 2,
            closed: 2,
            ..Loss::default()
        }
    );
}
