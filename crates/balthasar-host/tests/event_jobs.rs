#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_host::{Answering, Door, Hooks, Keeping, answer_hooked, events::*};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{AgentId, ScopeId, SessionId, floor, scratch::Scratch};
use balthasar_store::{JobState, Store, Transcript};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Barrier, mpsc},
    time::Duration,
};

const SESSION: &str = "private-session-canary";
const TEXT: &str = "private-answer-canary";
const NOW: i64 = 1_756_000_000;

struct Plain;
impl Hooks for Plain {}

struct Before<F>(F);
impl<F: Fn()> Hooks for Before<F> {
    fn memory(&self) -> Keeping {
        (self.0)();
        Keeping::default()
    }
}

struct Harness {
    store: Store,
    transcript: Transcript,
}

impl Harness {
    fn new(transcript: Transcript) -> Self {
        Self {
            store: Store::ephemeral().expect("memory"),
            transcript,
        }
    }

    fn ephemeral() -> Self {
        Self::new(Transcript::ephemeral().expect("transcript"))
    }

    fn job(&self, blocking: bool) -> String {
        let id = self
            .transcript
            .queue_job(
                &SessionId::new(SESSION),
                "summarise",
                &json!({"blocking":blocking}),
                &json!({"from":0,"to":1}),
                NOW,
            )
            .expect("queued native job");
        self.transcript
            .issue_job(&id, NOW)
            .expect("native issued state");
        id
    }

    fn done(&mut self, arguments: Value, hooks: &mut dyn Hooks) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: Some(&mut self.transcript),
            scratch: None,
            scope: ScopeId::new("/private/path-canary"),
            agent: AgentId::main(),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        };
        answer_hooked(
            &mut at,
            &Door::Owner,
            &Request {
                call: "job_done".into(),
                args: vec![json!(SESSION), arguments],
            },
            hooks,
        )
    }

    fn state(&self, id: &str) -> JobState {
        self.transcript
            .job(id)
            .expect("job read")
            .expect("job exists")
            .state
    }
}

fn observed(receiver: &Receiver, expected: JobOutcome) {
    let bytes = receiver.try_recv().expect("one actual job event");
    let text = std::str::from_utf8(&bytes).expect("encoded UTF8");
    for canary in [
        SESSION,
        TEXT,
        "private-error-canary",
        "path-canary",
        "private-model-canary",
        "J-",
    ] {
        assert!(!text.contains(canary), "event leaked {canary}");
    }
    let event = decode(&bytes).expect("strict event schema");
    assert_eq!(event.correlation, Correlation::default());
    assert_eq!(
        event.fact,
        Fact::JobResult {
            outcome: expected,
            revision: None
        }
    );
    assert!(receiver.try_recv().is_err(), "one event per observed reply");
}

fn completion_and_refusals(receiver: &Receiver) {
    let mut harness = Harness::ephemeral();
    let id = harness.job(false);
    let reply = harness.done(
        json!({"id":id,"text":TEXT,"model":"private-model-canary"}),
        &mut Before(|| {
            assert!(
                receiver.try_recv().is_err(),
                "no event before the transaction"
            );
        }),
    );
    assert!(reply.ok);
    assert_eq!(harness.state(&id), JobState::Done);
    assert!(
        harness
            .transcript
            .summary(&SessionId::new(SESSION))
            .expect("summary")
            .is_some()
    );
    observed(receiver, JobOutcome::Accepted);
    assert!(harness.done(json!({"id":id,"text":TEXT}), &mut Plain).ok);
    observed(receiver, JobOutcome::IgnoredRepeated);
    assert!(
        !harness
            .done(json!({"id":"missing-job-canary","text":TEXT}), &mut Plain)
            .ok
    );
    observed(receiver, JobOutcome::Rejected);
    assert!(!harness.done(json!({"text":TEXT}), &mut Plain).ok);
    observed(receiver, JobOutcome::Rejected);

    let id = harness.job(false);
    harness
        .transcript
        .issue_job(&id, NOW)
        .expect("reissued lease");
    assert!(
        harness
            .done(json!({"id":id,"attempt":1,"text":TEXT}), &mut Plain)
            .ok
    );
    assert_eq!(harness.state(&id), JobState::Issued);
    observed(receiver, JobOutcome::IgnoredObsolete);
}

fn retries_and_terminal_failure(receiver: &Receiver) {
    let mut harness = Harness::ephemeral();
    let id = harness.job(false);
    assert!(
        harness
            .done(json!({"id":id,"failed":"private-error-canary"}), &mut Plain)
            .ok
    );
    assert_eq!(harness.state(&id), JobState::Queued);
    observed(receiver, JobOutcome::RetryQueued);
    assert!(harness.done(json!({"id":id,"text":TEXT}), &mut Plain).ok);
    assert_eq!(harness.state(&id), JobState::Queued);
    observed(receiver, JobOutcome::IgnoredObsolete);
    harness
        .transcript
        .issue_job(&id, NOW)
        .expect("retry issued");
    assert!(
        harness
            .done(json!({"id":id,"failed":"private-error-canary"}), &mut Plain)
            .ok
    );
    assert_eq!(harness.state(&id), JobState::Failed);
    observed(receiver, JobOutcome::Failed);
    let blocking = harness.job(true);
    assert!(
        harness
            .done(
                json!({"id":blocking,"failed":"private-error-canary"}),
                &mut Plain
            )
            .ok
    );
    assert_eq!(harness.state(&blocking), JobState::Failed);
    observed(receiver, JobOutcome::Failed);
}

fn atomic_recheck(receiver: &Receiver) {
    let root = Scratch::new("event-jobs", "recheck");
    let path = root.join("transcript.db");
    let mut harness = Harness::new(Transcript::open(&path).expect("first connection"));
    let other = Transcript::open(&path).expect("second connection");
    let id = harness.job(false);
    let reply = harness.done(
        json!({"id":id,"text":TEXT}),
        &mut Before(|| {
            assert!(receiver.try_recv().is_err());
            other
                .settle_job(&id, JobState::Done, None, NOW)
                .expect("concurrent completion");
        }),
    );
    assert!(reply.ok);
    assert_eq!(harness.state(&id), JobState::Done);
    assert!(
        harness
            .transcript
            .summary(&SessionId::new(SESSION))
            .expect("summary")
            .is_none()
    );
    observed(receiver, JobOutcome::IgnoredRepeated);
    let id = harness.job(false);
    let reply = harness.done(
        json!({"id":id,"text":TEXT}),
        &mut Before(|| {
            other.issue_job(&id, NOW).expect("concurrent new lease");
        }),
    );
    assert!(!reply.ok);
    assert_eq!(harness.state(&id), JobState::Issued);
    observed(receiver, JobOutcome::Rejected);
}

fn transaction_failure(receiver: &Receiver) {
    let root = Scratch::new("event-jobs", "rollback");
    let path = root.join("transcript.db");
    let mut harness = Harness::new(Transcript::open(&path).expect("transcript"));
    let fault = rusqlite::Connection::open(&path).expect("fault connection");
    fault
        .execute_batch(
            "CREATE TRIGGER refuse_settlement BEFORE UPDATE OF state ON job
        WHEN NEW.state = 'done' BEGIN SELECT RAISE(ABORT, 'private-error-canary'); END;",
        )
        .expect("transaction failure fixture");
    let id = harness.job(false);
    assert!(!harness.done(json!({"id":id,"text":TEXT}), &mut Plain).ok);
    assert_eq!(harness.state(&id), JobState::Issued);
    assert!(
        harness
            .transcript
            .summary(&SessionId::new(SESSION))
            .expect("rolled back summary")
            .is_none()
    );
    observed(receiver, JobOutcome::Rejected);
    fault
        .execute_batch("DROP TRIGGER refuse_settlement;")
        .expect("remove owned fault");
    assert!(harness.done(json!({"id":id,"text":TEXT}), &mut Plain).ok);
    assert_eq!(harness.state(&id), JobState::Done);
    observed(receiver, JobOutcome::Accepted);
}

fn stalled_receiver(receiver: &Receiver) {
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
    let barrier = Arc::new(Barrier::new(2));
    let (done, completion) = mpsc::sync_channel(1);
    let task = std::thread::spawn({
        let barrier = barrier.clone();
        move || {
            let mut harness = Harness::ephemeral();
            let id = harness.job(false);
            barrier.wait();
            assert!(harness.done(json!({"id":id,"text":TEXT}), &mut Plain).ok);
            assert_eq!(harness.state(&id), JobState::Done);
            assert!(
                harness
                    .transcript
                    .summary(&SessionId::new(SESSION))
                    .expect("persisted summary")
                    .is_some()
            );
            done.send(()).expect("useful work completed");
        }
    });
    barrier.wait();
    completion
        .recv_timeout(Duration::from_secs(5))
        .expect("job must complete with event consumer stalled");
    task.join().expect("worker joined");
    assert_eq!(loss().full, 1);
    for _ in 0..QUEUED_EVENTS {
        assert_eq!(
            decode(&receiver.try_recv().expect("queued filler"))
                .expect("schema")
                .fact,
            Fact::Memory {
                outcome: MemoryOutcome::Read
            }
        );
    }
    assert!(receiver.try_recv().is_err());
    let mut harness = Harness::ephemeral();
    let id = harness.job(false);
    assert!(harness.done(json!({"id":id,"text":TEXT}), &mut Plain).ok);
    observed(receiver, JobOutcome::Accepted);
}

#[test]
fn actual_job_events_follow_settlement_without_becoming_authority() {
    assert_eq!(emit(|| panic!("disabled preparation")), Emitted::Disabled);
    let mut disabled = Harness::ephemeral();
    let id = disabled.job(false);
    assert!(disabled.done(json!({"id":id,"text":TEXT}), &mut Plain).ok);
    assert_eq!(disabled.state(&id), JobState::Done);
    assert_eq!(loss(), Loss::default());
    let receiver = enable_local().expect("explicit isolated activation");
    assert!(matches!(enable_local(), Err(EnableError::AlreadyEnabled)));
    completion_and_refusals(&receiver);
    retries_and_terminal_failure(&receiver);
    atomic_recheck(&receiver);
    transaction_failure(&receiver);
    stalled_receiver(&receiver);
    drop(receiver);
    let mut closed = Harness::ephemeral();
    let id = closed.job(false);
    assert!(closed.done(json!({"id":id,"text":TEXT}), &mut Plain).ok);
    assert_eq!(closed.state(&id), JobState::Done);
    assert_eq!(
        loss(),
        Loss {
            full: 1,
            closed: 1,
            ..Loss::default()
        }
    );
}
