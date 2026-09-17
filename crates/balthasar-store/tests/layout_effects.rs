use balthasar_model::{AgentId, MemoryId, ScopeId, SessionId, scratch::Scratch};
use balthasar_store::{LayoutEffect, LayoutTarget, Prompt, Transcript, Turn};
use serde_json::json;

fn effect() -> LayoutEffect {
    LayoutEffect {
        layout: "L-1".into(),
        session: SessionId::new("child-transcript"),
        run: SessionId::new("root-run"),
        agent: AgentId::new("child"),
        scope: ScopeId::new("/project"),
        cursor: 0,
        text: "Original source".into(),
        at: 123,
    }
}

#[test]
fn target_resolution_is_immutable_and_cannot_ack_unresolved_or_revive_finished_effects() {
    let store = Transcript::ephemeral().expect("store");
    let record = effect();
    store.queue_layout_effect(&record).expect("queue");
    assert!(store.finish_layout_effect(&record).is_err());
    let first = MemoryId::new("first");
    let next = MemoryId::new("next");
    assert_eq!(
        store
            .bind_layout_target(&record, Some(&first))
            .expect("first resolution"),
        LayoutTarget::Bound(first.clone())
    );
    assert_eq!(
        store
            .bind_layout_target(&record, Some(&next))
            .expect("second resolution"),
        LayoutTarget::Bound(first.clone())
    );
    assert_eq!(
        store
            .bind_layout_target(&record, None)
            .expect("missing resolution"),
        LayoutTarget::Bound(first)
    );
    let mut wrong = record.clone();
    wrong.agent = AgentId::main();
    assert_eq!(
        store
            .bind_layout_target(&wrong, Some(&next))
            .expect("wrong owner"),
        LayoutTarget::Finished
    );
    store.finish_layout_effect(&record).expect("finish");
    assert_eq!(
        store
            .bind_layout_target(&record, Some(&next))
            .expect("finished"),
        LayoutTarget::Finished
    );
    let mut missing = record.clone();
    missing.cursor = 1;
    store.queue_layout_effect(&missing).expect("queue missing");
    assert_eq!(
        store.bind_layout_target(&missing, None).expect("missing"),
        LayoutTarget::Missing
    );
    assert_eq!(
        store
            .bind_layout_target(&missing, Some(&next))
            .expect("replacement"),
        LayoutTarget::Missing
    );
    balthasar_store::purge_run(&store, &record.session).expect("purge");
    assert_eq!(
        store
            .bind_layout_target(&missing, Some(&next))
            .expect("purged"),
        LayoutTarget::Finished
    );
}

#[test]
fn competing_target_resolutions_return_the_same_durable_winner() {
    let dir = Scratch::new("layout-target", "concurrent");
    let path = dir.join("transcript.db");
    let first = Transcript::open(&path).expect("first connection");
    let second = Transcript::open(&path).expect("second connection");
    let record = effect();
    first.queue_layout_effect(&record).expect("queue");
    let gate = std::sync::Barrier::new(3);
    let winner = std::thread::scope(|scope| {
        let gate = &gate;
        let record = &record;
        let a = scope.spawn(move || {
            gate.wait();
            first
                .bind_layout_target(record, Some(&MemoryId::new("first")))
                .expect("first bind")
        });
        let b = scope.spawn(move || {
            gate.wait();
            second
                .bind_layout_target(record, Some(&MemoryId::new("second")))
                .expect("second bind")
        });
        gate.wait();
        let a = a.join().expect("first result");
        let b = b.join().expect("second result");
        assert_eq!(a, b);
        assert!(matches!(a, LayoutTarget::Bound(_)));
        a
    });
    let reopened = Transcript::open(&path).expect("reopen");
    assert_eq!(
        reopened.layout_target(&record).expect("persisted winner"),
        winner
    );
}

#[test]
fn outbox_sources_survive_reopen_and_are_scoped_to_their_owner() {
    let dir = Scratch::new("layout-effects", "identity");
    let path = dir.join("transcript.db");
    let record = effect();
    let store = Transcript::open(&path).expect("store");
    store.queue_layout_effect(&record).expect("queue");
    drop(store);
    let store = Transcript::open(&path).expect("reopen");
    assert_eq!(
        store
            .layout_effects(&record.session, &record.agent, &record.scope)
            .expect("owned"),
        vec![record.clone()]
    );
    assert!(
        store
            .layout_effects(&record.run, &record.agent, &record.scope)
            .expect("other transcript")
            .is_empty()
    );
    assert!(
        store
            .layout_effects(&record.session, &AgentId::main(), &record.scope)
            .expect("other agent")
            .is_empty()
    );
    assert!(
        store
            .layout_effects(&record.session, &record.agent, &ScopeId::new("/other"))
            .expect("other project")
            .is_empty()
    );
    store
        .bind_layout_target(&record, None)
        .expect("missing target");
    store.finish_layout_effect(&record).expect("ack");
    assert!(
        store
            .layout_effects(&record.session, &record.agent, &record.scope)
            .expect("finished")
            .is_empty()
    );
}

#[test]
fn purging_a_root_run_removes_pending_and_completed_child_intents() {
    let dir = Scratch::new("layout-effects", "purge");
    let path = dir.join("transcript.db");
    let store = Transcript::open(&path).expect("store");
    let record = effect();
    store
        .bind_run(&record.session, Some(record.run.as_str()))
        .expect("bind");
    store.queue_layout_effect(&record).expect("pending");
    let mut completed = record.clone();
    completed.cursor = 1;
    store
        .queue_layout_effect(&completed)
        .expect("queue completed");
    store
        .bind_layout_target(&completed, None)
        .expect("missing target");
    store.finish_layout_effect(&completed).expect("ack");
    balthasar_store::purge_run(&store, &record.run).expect("purge root");
    drop(store);
    let raw = rusqlite::Connection::open(&path).expect("inspect");
    let count: i64 = raw
        .query_row("SELECT count(*) FROM layout_effect", [], |r| r.get(0))
        .expect("count");
    assert_eq!(count, 0);
}

#[test]
fn adding_the_outbox_preserves_legacy_transcripts_notes_jobs_and_applied_layouts() {
    let dir = Scratch::new("layout-effects", "migration");
    let path = dir.join("transcript.db");
    let mut store = Transcript::open(&path).expect("fixture");
    let session = SessionId::new("legacy");
    store
        .write(
            &session,
            &Turn {
                cursor: 0,
                role: "user".into(),
                text: "Legacy source".into(),
                ..Turn::default()
            },
        )
        .expect("turn");
    store
        .put_note(
            None,
            Some(&json!({"title":"Legacy note","text":"Original note"})),
            1,
        )
        .expect("note");
    let job = store
        .queue_job(
            &session,
            "extract",
            &json!({"input":"Legacy source"}),
            &json!({"from":0,"to":0}),
            1,
        )
        .expect("job");
    store.issue_job(&job, 1).expect("issue");
    store
        .keep_summary(&session, 0, 0, "Legacy summary", 1)
        .expect("summary");
    store
        .keep_prompt(
            &session,
            &Prompt {
                mark: "old".into(),
                ..Prompt::default()
            },
        )
        .expect("prompt");
    let layout = store
        .propose(
            &session,
            "legacy-model",
            10,
            &json!({"slots":[]}),
            &json!({}),
            1,
        )
        .expect("layout");
    store
        .confirm(&layout, Some(10), 1)
        .expect("old applied marker");
    let before = (
        store.replay(&session).expect("turns"),
        store.notes().expect("notes"),
        store.job(&job).expect("job"),
        store.proposal(&layout).expect("layout"),
        store.summary(&session).expect("summary"),
        store.prompt(&session).expect("prompt"),
    );
    drop(store);
    let old = rusqlite::Connection::open(&path).expect("old fixture");
    old.execute_batch("DROP TABLE layout_effect")
        .expect("pre-outbox schema");
    drop(old);
    let reopened = Transcript::open(&path).expect("migrated");
    let after = (
        reopened.replay(&session).expect("turns"),
        reopened.notes().expect("notes"),
        reopened.job(&job).expect("job"),
        reopened.proposal(&layout).expect("layout"),
        reopened.summary(&session).expect("summary"),
        reopened.prompt(&session).expect("prompt"),
    );
    assert_eq!(before, after);
    assert!(
        reopened
            .layout_effects(&session, &AgentId::main(), &ScopeId::new("/project"))
            .expect("outbox")
            .is_empty()
    );
}
