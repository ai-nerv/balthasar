use super::*;
use balthasar_model::SessionId;
use balthasar_model::scratch::Scratch;
use balthasar_store::{JobState, Prompt};

struct CompletionHook<F>(F);

impl<F: Fn()> Hooks for CompletionHook<F> {
    fn memory(&self) -> Keeping {
        (self.0)();
        Keeping::default()
    }
}

fn with_hook(harness: &mut Harness, done: Value, hook: impl Fn()) -> Reply {
    let mut at = Answering {
        store: &mut harness.store,
        scrollback: Some(&mut harness.scrollback),
        scratch: None,
        scope: ScopeId::new("/w/thing"),
        agent: balthasar_model::AgentId::main(),
        now: NOW,
        inject_floor: floor::INJECT,
        live_floor: floor::LIVE,
        capture: false,
    };
    let request = Request {
        call: "job_done".into(),
        args: vec![json!(SESSION), done],
    };
    answer_hooked(&mut at, &Door::Owner, &request, &mut CompletionHook(hook))
}

#[test]
fn concurrent_completions_commit_one_effect_and_counter_increment() {
    let (mut first, dir, _faults) = durable("concurrent-completion", false);
    let job = queued(&mut first);
    let done = completion(&job);
    let mut second = Harness::new();
    second.scrollback = Transcript::open(&dir.join("transcript.db")).expect("second connection");
    let (ready, reached) = std::sync::mpsc::channel();
    let (release_a, gate_a) = std::sync::mpsc::channel();
    let (release_b, gate_b) = std::sync::mpsc::channel();
    let ready_b = ready.clone();
    let second_done = done.clone();
    let timeout = std::time::Duration::from_secs(10);
    std::thread::scope(|scope| {
        let first = &mut first;
        let second = &mut second;
        let a = scope.spawn(move || {
            with_hook(first, done, || {
                ready.send(()).expect("first prepared");
                gate_a.recv_timeout(timeout).expect("first released");
            })
        });
        let b = scope.spawn(move || {
            with_hook(second, second_done, || {
                ready_b.send(()).expect("second prepared");
                gate_b.recv_timeout(timeout).expect("second released");
            })
        });
        reached.recv_timeout(timeout).expect("first preparation");
        reached.recv_timeout(timeout).expect("second preparation");
        release_a.send(()).expect("release first");
        release_b.send(()).expect("release second");
        assert!(a.join().expect("first completion").ok);
        assert!(b.join().expect("second completion").ok);
    });
    reopen(&mut first, &dir);
    assert_eq!(first.scrollback.notes().expect("notes").len(), 1);
    assert_eq!(first.scrollback.changes(20).expect("audit").len(), 1);
    assert_eq!(
        first
            .scrollback
            .counter(&SessionId::new(""), "extracts")
            .expect("counter"),
        Some(1)
    );
    assert_eq!(
        first
            .scrollback
            .job(job["id"].as_str().expect("id"))
            .expect("job")
            .expect("kept")
            .state,
        JobState::Done
    );
}

#[test]
fn curation_does_not_lock_during_hooks_or_overwrite_a_replacement_prompt() {
    let (mut harness, dir, _faults) = durable("curation-rebind", false);
    let session = SessionId::new(SESSION);
    harness
        .scrollback
        .keep_prompt(
            &session,
            &Prompt {
                mark: "old-prompt".into(),
                ..Prompt::default()
            },
        )
        .expect("old prompt");
    let id = harness
        .scrollback
        .queue_job(
            &session,
            "curate",
            &json!({}),
            &json!({"mark":"old-prompt","query":"parser","room":1000}),
            NOW,
        )
        .expect("queue");
    harness.scrollback.issue_job(&id, NOW).expect("issue");
    let other = Transcript::open(&dir.join("transcript.db")).expect("other connection");
    let new = Prompt {
        mark: "new-prompt".into(),
        memory: Some(json!({"text":"New context"})),
        ..Prompt::default()
    };
    let reply = with_hook(
        &mut harness,
        json!({"id":id,"text":"{\"chosen\":[],\"notes\":[\"Old helper context\"]}"}),
        || {
            other
                .keep_prompt(&session, &new)
                .expect("hook can write without waiting for the completion transaction");
        },
    );
    assert!(reply.ok, "{:?}", reply.error);
    reopen(&mut harness, &dir);
    assert_eq!(
        harness.scrollback.prompt(&session).expect("prompt"),
        Some(new)
    );
    assert_eq!(
        harness
            .scrollback
            .job(&id)
            .expect("job")
            .expect("kept")
            .state,
        JobState::Done
    );
}

#[test]
fn a_reissued_job_cannot_receive_effects_prepared_for_its_old_attempt() {
    let (mut harness, dir, _faults) = durable("job-reissued", false);
    let job = queued(&mut harness);
    let id = job["id"].as_str().expect("id");
    let other = Transcript::open(&dir.join("transcript.db")).expect("other connection");
    let reply = with_hook(&mut harness, completion(&job), || {
        other
            .issue_job(id, NOW + 1)
            .expect("reissued during preparation");
    });
    assert!(!reply.ok);
    assert!(
        reply
            .error
            .expect("reason")
            .contains("changed during completion")
    );
    reopen(&mut harness, &dir);
    assert!(harness.scrollback.notes().expect("notes").is_empty());
    assert!(harness.scrollback.changes(20).expect("audit").is_empty());
    let kept = harness.scrollback.job(id).expect("job").expect("kept");
    assert_eq!(kept.attempts, 2);
    assert_eq!(kept.state, JobState::Issued);
}

fn fail_completion(faults: &rusqlite::Connection) {
    faults
        .execute_batch(
            "CREATE TRIGGER fail_completion BEFORE UPDATE OF state ON job
        WHEN NEW.state = 'done' BEGIN SELECT RAISE(ABORT, 'completion marker failed'); END;",
        )
        .expect("completion fault");
}

#[test]
fn failed_completion_rolls_back_notes_counters_and_followup_jobs() {
    for review in [false, true] {
        let (mut harness, dir, faults) = durable(
            if review {
                "complete-review"
            } else {
                "complete-apply"
            },
            review,
        );
        let job = queued(&mut harness);
        let mut done = completion(&job);
        done["usage"] = json!({"input":10,"output":20});
        done["model"] = json!("synthetic");
        let session = SessionId::new(SESSION);
        let project = SessionId::new("");
        let before = harness.scrollback.jobs_of(&session).expect("before jobs");
        let count = harness
            .scrollback
            .counter(&project, "extracts")
            .expect("counter");
        fail_completion(&faults);
        assert!(!harness.ask("job_done", done.clone()).ok);
        reopen(&mut harness, &dir);
        assert!(harness.scrollback.notes().expect("notes").is_empty());
        assert!(harness.scrollback.changes(20).expect("audit").is_empty());
        assert_eq!(
            harness
                .scrollback
                .counter(&project, "extracts")
                .expect("counter"),
            count
        );
        assert_eq!(harness.scrollback.jobs_of(&session).expect("jobs"), before);
        faults
            .execute_batch("DROP TRIGGER fail_completion")
            .expect("remove fault");
        assert!(harness.ask("job_done", done.clone()).ok);
        let committed = harness
            .scrollback
            .jobs_of(&session)
            .expect("committed jobs");
        assert!(harness.ask("job_done", done).ok);
        reopen(&mut harness, &dir);
        assert_eq!(
            harness.scrollback.jobs_of(&session).expect("jobs"),
            committed
        );
        assert_eq!(harness.scrollback.changes(20).expect("audit").len(), 1);
        assert_eq!(
            harness.scrollback.notes().expect("notes").len(),
            usize::from(!review)
        );
        assert_eq!(
            harness
                .scrollback
                .counter(&project, "extracts")
                .expect("counter"),
            Some(count.unwrap_or(0) + 1)
        );
        let completed = harness
            .scrollback
            .job(job["id"].as_str().expect("id"))
            .expect("job")
            .expect("kept");
        assert_eq!(completed.state, JobState::Done);
        assert_eq!(completed.result.expect("result")["model"], "synthetic");
        assert_eq!(
            committed.iter().filter(|j| j.kind == "review").count(),
            usize::from(review)
        );
    }
}

#[test]
fn failed_empty_completion_does_not_duplicate_its_retry() {
    let (mut harness, dir, faults) = durable("empty-completion", false);
    let job = queued(&mut harness);
    let session = SessionId::new(SESSION);
    let before = harness.scrollback.jobs_of(&session).expect("before");
    let done = json!({"id":job["id"],"text":"{\"ops\":[]}"});
    fail_completion(&faults);
    assert!(!harness.ask("job_done", done.clone()).ok);
    reopen(&mut harness, &dir);
    assert_eq!(harness.scrollback.jobs_of(&session).expect("jobs"), before);
    faults
        .execute_batch("DROP TRIGGER fail_completion")
        .expect("remove fault");
    assert!(harness.ask("job_done", done.clone()).ok);
    assert!(harness.ask("job_done", done).ok);
    let jobs = harness.scrollback.jobs_of(&session).expect("jobs");
    assert_eq!(jobs.len(), before.len() + 1);
    assert_eq!(
        jobs.iter().filter(|j| j.context["again"] == true).count(),
        1
    );
}

#[test]
fn failed_review_completion_keeps_all_changes_staged() {
    let (mut harness, dir, faults) = durable("review-completion", true);
    let job = queued(&mut harness);
    let done = json!({"id":job["id"],"text":json!({"ops":[
        {"op":"add","title":"Rule","text":"Always use uv.","pinned":true},
        {"op":"add","title":"Parser","text":"The parser is Rust."}
    ]}).to_string()});
    assert!(harness.ask("job_done", done).ok);
    let session = SessionId::new(SESSION);
    let review = harness
        .scrollback
        .jobs_of(&session)
        .expect("jobs")
        .into_iter()
        .find(|j| j.kind == "review")
        .expect("review");
    harness
        .scrollback
        .issue_job(&review.id, NOW)
        .expect("issue");
    let before = harness.scrollback.changes(20).expect("staged");
    let done = json!({"id":review.id,"text":"{\"approve\":[\"C-1\"],\"reject\":[\"C-2\"]}"});
    fail_completion(&faults);
    assert!(!harness.ask("job_done", done.clone()).ok);
    reopen(&mut harness, &dir);
    assert!(harness.scrollback.notes().expect("notes").is_empty());
    assert_eq!(harness.scrollback.changes(20).expect("audit"), before);
    faults
        .execute_batch("DROP TRIGGER fail_completion")
        .expect("remove fault");
    assert!(harness.ask("job_done", done.clone()).ok);
    assert!(harness.ask("job_done", done).ok);
    assert_eq!(harness.scrollback.notes().expect("notes").len(), 1);
    assert_eq!(
        harness
            .scrollback
            .change("C-1")
            .expect("change")
            .expect("kept")
            .state,
        "applied"
    );
    assert_eq!(
        harness
            .scrollback
            .change("C-2")
            .expect("change")
            .expect("kept")
            .state,
        "rejected"
    );
}

#[test]
fn failed_summary_completion_restores_the_previous_summary() {
    let (mut harness, dir, faults) = durable("summary-completion", false);
    let session = SessionId::new(SESSION);
    harness
        .scrollback
        .keep_summary(&session, 0, 2, "Old summary", NOW)
        .expect("old");
    let before = harness.scrollback.summary(&session).expect("summary");
    let id = harness
        .scrollback
        .queue_job(
            &session,
            "summarise",
            &json!({}),
            &json!({"from":0,"to":6}),
            NOW,
        )
        .expect("queue");
    harness.scrollback.issue_job(&id, NOW).expect("issue");
    let done = json!({"id":id,"text":"New summary"});
    fail_completion(&faults);
    assert!(!harness.ask("job_done", done.clone()).ok);
    reopen(&mut harness, &dir);
    assert_eq!(
        harness.scrollback.summary(&session).expect("summary"),
        before
    );
    faults
        .execute_batch("DROP TRIGGER fail_completion")
        .expect("remove fault");
    assert!(harness.ask("job_done", done.clone()).ok);
    assert!(harness.ask("job_done", done).ok);
    assert_eq!(
        harness
            .scrollback
            .summary(&session)
            .expect("summary")
            .expect("kept")
            .text,
        "New summary"
    );
}

#[test]
fn failed_curate_completion_restores_the_prompt_memory() {
    let (mut harness, dir, faults) = durable("curate-completion", false);
    let session = SessionId::new(SESSION);
    let before = Prompt {
        mark: "prompt-one".into(),
        memory: Some(json!({"text":"Original context"})),
        ..Prompt::default()
    };
    harness
        .scrollback
        .keep_prompt(&session, &before)
        .expect("prompt");
    let id = harness
        .scrollback
        .queue_job(
            &session,
            "curate",
            &json!({}),
            &json!({"mark":"prompt-one","query":"parser","room":1000}),
            NOW,
        )
        .expect("queue");
    harness.scrollback.issue_job(&id, NOW).expect("issue");
    let done = json!({"id":id,"text":"{\"chosen\":[],\"notes\":[\"The parser is Rust.\"]}"});
    fail_completion(&faults);
    assert!(!harness.ask("job_done", done.clone()).ok);
    reopen(&mut harness, &dir);
    assert_eq!(
        harness.scrollback.prompt(&session).expect("prompt"),
        Some(before)
    );
    faults
        .execute_batch("DROP TRIGGER fail_completion")
        .expect("remove fault");
    assert!(harness.ask("job_done", done.clone()).ok);
    assert!(harness.ask("job_done", done).ok);
    let prompt = harness
        .scrollback
        .prompt(&session)
        .expect("prompt")
        .expect("kept");
    assert_eq!(prompt.memory.expect("memory")["curated"], true);
}

fn durable(name: &str, review: bool) -> (Harness, Scratch, rusqlite::Connection) {
    let dir = Scratch::new("note-atomic", name);
    let path = dir.join("transcript.db");
    let mut harness = Harness::keeping(Keeping {
        review,
        ..Keeping::default()
    });
    harness.scrollback = Transcript::open(&path).expect("owned transcript");
    let faults = rusqlite::Connection::open(path).expect("fault injector");
    (harness, dir, faults)
}

fn queued(harness: &mut Harness) -> Value {
    harness.turn(0, "Always use uv.");
    harness.layout(0)["jobs"][0].clone()
}

fn completion(job: &Value) -> Value {
    json!({"id":job["id"], "text":json!({"ops":[
        {"op":"add","title":"Package manager","text":"Always use uv.","pinned":true}
    ]}).to_string()})
}

fn fail_at(faults: &rusqlite::Connection, state: &str) {
    faults
        .execute_batch(&format!(
            "CREATE TRIGGER fail_audit BEFORE UPDATE OF state ON note_change
        WHEN NEW.state = '{state}' BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END;"
        ))
        .expect("audit fault");
}

fn reopen(harness: &mut Harness, dir: &Scratch) {
    harness.scrollback = Transcript::open(&dir.join("transcript.db")).expect("reopen");
}

#[test]
fn failed_application_rolls_back_the_note_and_its_proposal_before_retry() {
    let (mut harness, dir, faults) = durable("application", false);
    let job = queued(&mut harness);
    let done = completion(&job);
    fail_at(&faults, "applied");
    assert!(!harness.ask("job_done", done.clone()).ok);
    reopen(&mut harness, &dir);
    assert!(harness.scrollback.notes().expect("notes").is_empty());
    assert!(harness.scrollback.changes(20).expect("audit").is_empty());
    faults
        .execute_batch("DROP TRIGGER fail_audit")
        .expect("remove fault");
    assert!(harness.ask("job_done", done.clone()).ok);
    assert!(harness.ask("job_done", done).ok);
    reopen(&mut harness, &dir);
    assert_eq!(harness.scrollback.notes().expect("notes").len(), 1);
    let audit = harness.scrollback.changes(20).expect("audit");
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].state, "applied");
}

#[test]
fn failed_review_keeps_the_original_staged_change_and_no_applied_note() {
    let (mut harness, dir, faults) = durable("review", true);
    let job = queued(&mut harness);
    assert!(harness.ask("job_done", completion(&job)).ok);
    fail_at(&faults, "applied");
    assert!(!harness.ask("approve", json!({"changes":["C-1"]})).ok);
    reopen(&mut harness, &dir);
    assert!(harness.scrollback.notes().expect("notes").is_empty());
    assert_eq!(
        harness
            .scrollback
            .change("C-1")
            .expect("audit")
            .expect("change")
            .state,
        "staged"
    );
    faults
        .execute_batch("DROP TRIGGER fail_audit")
        .expect("remove fault");
    harness.one("approve", json!({"changes":["C-1"]}));
    assert_eq!(harness.scrollback.notes().expect("notes").len(), 1);
    assert_eq!(harness.scrollback.changes(20).expect("audit").len(), 1);
}

#[test]
fn failed_undo_does_not_retire_the_note_or_commit_a_revert() {
    let (mut harness, dir, faults) = durable("undo", false);
    let job = queued(&mut harness);
    assert!(harness.ask("job_done", completion(&job)).ok);
    let original = harness.scrollback.note("N-1").expect("note").expect("kept");
    fail_at(&faults, "undone");
    assert!(!harness.ask("undo", json!({"change":"C-1"})).ok);
    reopen(&mut harness, &dir);
    assert_eq!(
        harness.scrollback.note("N-1").expect("note").expect("kept"),
        original
    );
    let audit = harness.scrollback.changes(20).expect("audit");
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].state, "applied");
    faults
        .execute_batch("DROP TRIGGER fail_audit")
        .expect("remove fault");
    harness.one("undo", json!({"change":"C-1"}));
    assert!(harness.scrollback.notes().expect("notes").is_empty());
    let audit = harness.scrollback.changes(20).expect("audit");
    assert_eq!(audit.len(), 2);
    assert_eq!(audit[0].op, "revert");
    assert_eq!(audit[1].state, "undone");
}

#[test]
fn a_later_operation_failure_rolls_back_the_entire_proposed_batch() {
    let (mut harness, dir, faults) = durable("batch", false);
    let job = queued(&mut harness);
    let done = json!({"id":job["id"], "text":json!({"ops":[
        {"op":"add","title":"Package manager","text":"Always use uv.","pinned":true},
        {"op":"add","title":"Second","text":"The parser is Rust."}
    ]}).to_string()});
    faults
        .execute_batch(
            "CREATE TRIGGER fail_second BEFORE INSERT ON note
        WHEN NEW.title = 'Second' BEGIN SELECT RAISE(ABORT, 'second note failed'); END;",
        )
        .expect("second operation fault");
    assert!(!harness.ask("job_done", done.clone()).ok);
    reopen(&mut harness, &dir);
    assert!(harness.scrollback.notes().expect("notes").is_empty());
    assert!(harness.scrollback.changes(20).expect("audit").is_empty());
    faults
        .execute_batch("DROP TRIGGER fail_second")
        .expect("remove fault");
    assert!(harness.ask("job_done", done).ok);
    assert_eq!(harness.scrollback.notes().expect("notes").len(), 2);
    assert_eq!(harness.scrollback.changes(20).expect("audit").len(), 2);
}

#[test]
fn partial_updates_in_one_batch_preserve_each_others_fields() {
    let (mut harness, dir, _faults) = durable("partial-updates", false);
    harness.scrollback.put_note(None, Some(&json!({"title":"Parser","text":"The parser is Rust.","description":"Old description"})), NOW).expect("base");
    let job = queued(&mut harness);
    assert!(
        harness
            .ask(
                "job_done",
                json!({"id":job["id"],"text":json!({"ops":[
        {"op":"update","id":"N-1","text":"The parser is Go."},
        {"op":"update","id":"N-1","description":"A Go parser."}
    ]}).to_string()})
            )
            .ok
    );
    reopen(&mut harness, &dir);
    let note = harness.scrollback.note("N-1").expect("note").expect("kept");
    assert_eq!(note.text, "The parser is Go.");
    assert_eq!(note.description, "A Go parser.");
    let audit = harness.scrollback.changes(20).expect("audit");
    assert_eq!(audit.len(), 2);
    assert_eq!(
        audit[0].before.as_ref().expect("before")["text"],
        "The parser is Go."
    );
}
