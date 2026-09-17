use super::*;
use balthasar_model::{MemoryId, WitnessKind, scratch::Scratch};

fn durable(name: &str) -> (Harness, Scratch, Vec<MemoryId>) {
    let dir = Scratch::new("layout-atomic", name);
    let mut harness = Harness {
        store: Store::open(&dir.join("memory.db")).expect("owned memory"),
        scrollback: Transcript::open(&dir.join("transcript.db")).expect("owned transcript"),
    };
    let mut memories = Vec::new();
    for cursor in 0..2 {
        let text = format!("The parser has stage {cursor}.");
        harness.observe(json!({"cursor":cursor,"role":"user","kind":"user","text":text}));
        memories.push(
            harness
                .store
                .scratch_for(&ScopeId::new("/w/thing"), &SessionId::new(SESSION), &text)
                .expect("scratch")
                .expect("kept"),
        );
    }
    (harness, dir, memories)
}

fn proposal(harness: &Harness, summary: bool) -> String {
    let slots = if summary {
        json!([{"kind":"summary","covers":[0,1],"text":"Two parser stages"}])
    } else {
        json!([{"kind":"stub","cursor":0},{"kind":"stub","cursor":1}])
    };
    harness
        .scrollback
        .propose(
            &SessionId::new(SESSION),
            "atomic-model",
            100,
            &json!({"slots":slots}),
            &json!({}),
            NOW,
        )
        .expect("proposal")
}

fn apply(harness: &mut Harness, id: &str) -> Reply {
    harness.ask(
        "applied",
        vec![json!(SESSION), json!({"id":id,"usage":{"input":120}})],
    )
}

fn reopen(harness: &mut Harness, dir: &Scratch) {
    harness.scrollback = Transcript::open(&dir.join("transcript.db")).expect("reopen transcript");
    harness.store = Store::open(&dir.join("memory.db")).expect("reopen memory");
}

fn distillations(harness: &Harness, id: &MemoryId) -> usize {
    harness
        .store
        .witnesses_of(id)
        .expect("witnesses")
        .iter()
        .filter(|w| w.kind == WitnessKind::Distillation)
        .count()
}

fn pending(harness: &Harness) -> Vec<balthasar_store::LayoutEffect> {
    harness
        .scrollback
        .layout_effects(
            &SessionId::new(SESSION),
            &balthasar_model::AgentId::main(),
            &ScopeId::new("/w/thing"),
        )
        .expect("outbox")
}

#[test]
fn a_purged_target_is_not_replaced_by_new_memory_with_the_same_text() {
    let (mut harness, dir, memories) = durable("purged-target");
    let id = proposal(&harness, true);
    let faults = rusqlite::Connection::open(dir.join("transcript.db")).expect("faults");
    faults.execute_batch("CREATE TRIGGER fail_target_ack BEFORE UPDATE OF done ON layout_effect BEGIN SELECT RAISE(ABORT, 'ack failed'); END;").expect("fault");
    assert!(!apply(&mut harness, &id).ok);
    assert_eq!(distillations(&harness, &memories[0]), 1);
    balthasar_store::purge(&mut harness.store, &memories[0]).expect("purge original");
    harness
        .observe(json!({"cursor":0,"role":"user","kind":"user","text":"The parser has stage 0."}));
    let replacement = harness
        .store
        .scratch_for(
            &ScopeId::new("/w/thing"),
            &SessionId::new(SESSION),
            "The parser has stage 0.",
        )
        .expect("scratch")
        .expect("replacement");
    assert_ne!(replacement, memories[0]);
    reopen(&mut harness, &dir);
    faults
        .execute_batch("DROP TRIGGER fail_target_ack")
        .expect("remove fault");
    assert!(apply(&mut harness, &id).ok);
    assert_eq!(
        distillations(&harness, &replacement),
        0,
        "old intent must not acquire a replacement target"
    );
    assert!(
        harness
            .store
            .get(&memories[0])
            .expect("old memory")
            .is_none()
    );
    assert!(pending(&harness).is_empty());
}

#[test]
fn promotion_does_not_abandon_a_pending_targets_rescore() {
    let (mut harness, dir, memories) = durable("promoted-target");
    let id = proposal(&harness, true);
    let faults = rusqlite::Connection::open(dir.join("memory.db")).expect("faults");
    faults.execute_batch("CREATE TRIGGER fail_target_score BEFORE UPDATE OF confidence ON memory BEGIN SELECT RAISE(ABORT, 'rescore failed'); END;").expect("fault");
    assert!(!apply(&mut harness, &id).ok);
    let witness = balthasar_model::Witness::new(
        balthasar_model::WitnessId::new("promote-target"),
        WitnessKind::Imperative,
        SessionId::new(SESSION),
        ScopeId::new("/w/thing"),
        NOW,
    );
    assert!(
        harness
            .store
            .promote(&memories[0], balthasar_model::Tier::Fact, witness, NOW)
            .is_err()
    );
    assert_eq!(
        harness
            .store
            .get(&memories[0])
            .expect("memory")
            .expect("kept")
            .tier,
        balthasar_model::Tier::Fact
    );
    reopen(&mut harness, &dir);
    faults
        .execute_batch("DROP TRIGGER fail_target_score")
        .expect("remove fault");
    assert!(apply(&mut harness, &id).ok);
    assert!(
        harness
            .store
            .get(&memories[0])
            .expect("memory")
            .expect("kept")
            .confidence
            > 0.0,
        "promotion must not hide an unfinished rescore"
    );
    assert_eq!(distillations(&harness, &memories[0]), 1);
    assert!(pending(&harness).is_empty());
}

#[test]
fn an_outbox_enqueue_fault_rolls_back_the_whole_confirmation() {
    let (mut harness, dir, memories) = durable("enqueue");
    let id = proposal(&harness, true);
    let faults = rusqlite::Connection::open(dir.join("transcript.db")).expect("faults");
    faults.execute_batch("CREATE TRIGGER fail_enqueue BEFORE INSERT ON layout_effect WHEN NEW.cursor = 1 BEGIN SELECT RAISE(ABORT, 'enqueue failed'); END;").expect("enqueue fault");
    assert!(!apply(&mut harness, &id).ok);
    reopen(&mut harness, &dir);
    assert!(pending(&harness).is_empty());
    assert!(harness.rows().iter().all(|t| t.state == State::Live));
    assert!(
        harness
            .scrollback
            .proposal(&id)
            .expect("proposal")
            .expect("kept")
            .applied
            .is_none()
    );
    assert!(
        harness
            .scrollback
            .measured("atomic-model")
            .expect("factor")
            .is_none()
    );
    assert!(memories.iter().all(|m| distillations(&harness, m) == 0));
    faults
        .execute_batch("DROP TRIGGER fail_enqueue")
        .expect("remove fault");
    assert!(apply(&mut harness, &id).ok);
    assert!(memories.iter().all(|m| distillations(&harness, m) == 1));
}

#[test]
fn a_target_binding_fault_prevents_any_cross_store_write() {
    let (mut harness, dir, memories) = durable("target-binding-fault");
    let id = proposal(&harness, true);
    let faults = rusqlite::Connection::open(dir.join("transcript.db")).expect("faults");
    faults.execute_batch("CREATE TRIGGER fail_binding BEFORE UPDATE OF resolved ON layout_effect BEGIN SELECT RAISE(ABORT, 'target binding failed'); END;").expect("fault");
    assert!(!apply(&mut harness, &id).ok);
    reopen(&mut harness, &dir);
    assert!(memories.iter().all(|m| distillations(&harness, m) == 0));
    let effects = pending(&harness);
    assert_eq!(effects.len(), 2);
    assert!(
        effects
            .iter()
            .all(|e| harness.scrollback.layout_target(e).expect("target")
                == balthasar_store::LayoutTarget::Unresolved)
    );
    faults
        .execute_batch("DROP TRIGGER fail_binding")
        .expect("remove fault");
    assert!(apply(&mut harness, &id).ok);
    assert!(memories.iter().all(|m| distillations(&harness, m) == 1));
}

#[test]
fn a_resolved_missing_target_cannot_acquire_a_later_replacement() {
    let (mut harness, dir, memories) = durable("missing-target");
    let id = proposal(&harness, true);
    balthasar_store::purge(&mut harness.store, &memories[0]).expect("purge before resolution");
    let faults = rusqlite::Connection::open(dir.join("transcript.db")).expect("faults");
    faults.execute_batch("CREATE TRIGGER fail_missing_ack BEFORE UPDATE OF done ON layout_effect BEGIN SELECT RAISE(ABORT, 'ack failed'); END;").expect("fault");
    assert!(!apply(&mut harness, &id).ok);
    assert_eq!(
        harness
            .scrollback
            .layout_target(&pending(&harness)[0])
            .expect("target"),
        balthasar_store::LayoutTarget::Missing
    );
    harness
        .observe(json!({"cursor":0,"role":"user","kind":"user","text":"The parser has stage 0."}));
    let replacement = harness
        .store
        .scratch_for(
            &ScopeId::new("/w/thing"),
            &SessionId::new(SESSION),
            "The parser has stage 0.",
        )
        .expect("scratch")
        .expect("replacement");
    reopen(&mut harness, &dir);
    faults
        .execute_batch("DROP TRIGGER fail_missing_ack")
        .expect("remove fault");
    assert!(apply(&mut harness, &id).ok);
    assert_eq!(distillations(&harness, &replacement), 0);
    assert!(pending(&harness).is_empty());
}

#[test]
fn an_existing_witness_recovers_a_legacy_unbound_promoted_target() {
    let (mut harness, dir, memories) = durable("legacy-target");
    let id = proposal(&harness, true);
    let faults = rusqlite::Connection::open(dir.join("memory.db")).expect("faults");
    faults.execute_batch("CREATE TRIGGER fail_legacy_score BEFORE UPDATE OF confidence ON memory BEGIN SELECT RAISE(ABORT, 'score failed'); END;").expect("fault");
    assert!(!apply(&mut harness, &id).ok);
    let witness = balthasar_model::Witness::new(
        balthasar_model::WitnessId::new("legacy-promotion"),
        WitnessKind::Imperative,
        SessionId::new(SESSION),
        ScopeId::new("/w/thing"),
        NOW,
    );
    assert!(
        harness
            .store
            .promote(&memories[0], balthasar_model::Tier::Fact, witness, NOW)
            .is_err()
    );
    let previous = pending(&harness);
    let old = rusqlite::Connection::open(dir.join("transcript.db")).expect("legacy fixture");
    old.execute_batch("ALTER TABLE layout_effect DROP COLUMN target; ALTER TABLE layout_effect DROP COLUMN resolved;").expect("pre-target schema");
    drop(old);
    reopen(&mut harness, &dir);
    assert_eq!(pending(&harness), previous);
    faults
        .execute_batch("DROP TRIGGER fail_legacy_score")
        .expect("remove fault");
    assert!(apply(&mut harness, &id).ok);
    assert_eq!(distillations(&harness, &memories[0]), 1);
    assert!(
        harness
            .store
            .get(&memories[0])
            .expect("memory")
            .expect("kept")
            .confidence
            > 0.0
    );
    assert!(pending(&harness).is_empty());
}

#[test]
fn a_failed_ack_replays_once_and_discards_completed_payloads() {
    let (mut harness, dir, memories) = durable("ack");
    let id = proposal(&harness, true);
    let faults = rusqlite::Connection::open(dir.join("transcript.db")).expect("faults");
    faults.execute_batch("CREATE TRIGGER fail_ack BEFORE UPDATE OF done ON layout_effect BEGIN SELECT RAISE(ABORT, 'ack failed'); END;").expect("ack fault");
    assert!(!apply(&mut harness, &id).ok);
    assert_eq!(distillations(&harness, &memories[0]), 1);
    reopen(&mut harness, &dir);
    assert_eq!(pending(&harness).len(), 2);
    faults
        .execute_batch("DROP TRIGGER fail_ack")
        .expect("remove fault");
    harness.one("layout", small());
    assert!(apply(&mut harness, &id).ok);
    assert!(pending(&harness).is_empty());
    for memory in memories {
        assert_eq!(distillations(&harness, &memory), 1);
    }
    let kept_text: String = faults
        .query_row(
            "SELECT group_concat(text, '') FROM layout_effect",
            [],
            |r| r.get(0),
        )
        .expect("payload");
    assert_eq!(kept_text, "");
    assert_eq!(
        harness.scrollback.measured("atomic-model").expect("factor"),
        Some((1.2, 1))
    );
}

#[test]
fn pending_effects_use_the_captured_source_not_a_later_amendment() {
    let (mut harness, dir, memories) = durable("source-amended");
    let id = proposal(&harness, true);
    let faults = rusqlite::Connection::open(dir.join("memory.db")).expect("faults");
    faults.execute_batch("CREATE TRIGGER fail_source BEFORE INSERT ON witness BEGIN SELECT RAISE(ABORT, 'witness failed'); END;").expect("fault");
    assert!(!apply(&mut harness, &id).ok);
    harness.observe(
        json!({"cursor":0,"role":"user","kind":"user","text":"Replacement parser source"}),
    );
    let replacement = harness
        .store
        .scratch_for(
            &ScopeId::new("/w/thing"),
            &SessionId::new(SESSION),
            "Replacement parser source",
        )
        .expect("scratch")
        .expect("replacement");
    reopen(&mut harness, &dir);
    faults
        .execute_batch("DROP TRIGGER fail_source")
        .expect("remove fault");
    assert!(apply(&mut harness, &id).ok);
    assert_eq!(distillations(&harness, &memories[0]), 1);
    assert_eq!(distillations(&harness, &replacement), 0);
    assert!(pending(&harness).is_empty());
}

#[test]
fn concurrent_replays_do_not_duplicate_witnesses_or_calibration() {
    let (mut first, dir, memories) = durable("concurrent");
    let id = proposal(&first, true);
    let faults = rusqlite::Connection::open(dir.join("memory.db")).expect("faults");
    faults.execute_batch("CREATE TRIGGER fail_concurrent BEFORE INSERT ON witness BEGIN SELECT RAISE(ABORT, 'witness failed'); END;").expect("fault");
    assert!(!apply(&mut first, &id).ok);
    faults
        .execute_batch("DROP TRIGGER fail_concurrent")
        .expect("remove fault");
    let mut second = Harness {
        store: Store::open(&dir.join("memory.db")).expect("second store"),
        scrollback: Transcript::open(&dir.join("transcript.db")).expect("second transcript"),
    };
    std::thread::scope(|scope| {
        let a = scope.spawn(|| apply(&mut first, &id));
        let b = scope.spawn(|| apply(&mut second, &id));
        assert!(a.join().expect("first").ok);
        assert!(b.join().expect("second").ok);
    });
    reopen(&mut first, &dir);
    assert!(pending(&first).is_empty());
    assert!(memories.iter().all(|m| distillations(&first, m) == 1));
    assert_eq!(
        first.scrollback.measured("atomic-model").expect("factor"),
        Some((1.2, 1))
    );
}

#[test]
fn layout_child() {
    let Some(dir) = std::env::var_os("BALTHASAR_LAYOUT_ATOMIC_CHILD") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let mut harness = Harness {
        store: Store::open(&dir.join("memory.db")).expect("store"),
        scrollback: Transcript::open(&dir.join("transcript.db")).expect("transcript"),
    };
    assert!(!apply(&mut harness, "L-1").ok);
    std::fs::write(dir.join("ready"), b"effect applied; ack failed").expect("ready");
    std::thread::sleep(std::time::Duration::from_secs(60));
}

struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_killed_process_leaves_cross_store_effects_replayable() {
    let (mut harness, dir, memories) = durable("killed");
    let id = proposal(&harness, true);
    let faults = rusqlite::Connection::open(dir.join("transcript.db")).expect("faults");
    faults.execute_batch("CREATE TRIGGER fail_death_ack BEFORE UPDATE OF done ON layout_effect BEGIN SELECT RAISE(ABORT, 'ack failed'); END;").expect("fault");
    let mut child = Child(
        std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "atomic::layout_child", "--nocapture"])
            .env("BALTHASAR_LAYOUT_ATOMIC_CHILD", &*dir)
            .current_dir(&*dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("owned child"),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !dir.join("ready").exists() {
        assert!(child.0.try_wait().expect("status").is_none());
        assert!(
            std::time::Instant::now() < deadline,
            "child did not reach fault point"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(distillations(&harness, &memories[0]), 1);
    child.0.kill().expect("kill");
    child.0.wait().expect("reap");
    reopen(&mut harness, &dir);
    assert_eq!(pending(&harness).len(), 2);
    faults
        .execute_batch("DROP TRIGGER fail_death_ack")
        .expect("remove fault");
    assert!(apply(&mut harness, &id).ok);
    assert!(pending(&harness).is_empty());
    assert!(memories.iter().all(|m| distillations(&harness, m) == 1));
}

#[test]
fn failed_mark_rolls_back_the_applied_marker_and_rows() {
    local_fault("mark");
}

#[test]
fn failed_factor_rolls_back_the_applied_marker_and_rows() {
    local_fault("factor");
}

fn local_fault(point: &str) {
    let (mut harness, dir, _memories) = durable(point);
    let id = proposal(&harness, false);
    let faults = rusqlite::Connection::open(dir.join("transcript.db")).expect("faults");
    faults.execute_batch(if point == "mark" {
            "CREATE TRIGGER fail_layout BEFORE UPDATE OF state ON turn WHEN NEW.cursor = 1 BEGIN SELECT RAISE(ABORT, 'mark failed'); END;"
        } else {
            "CREATE TRIGGER fail_layout BEFORE INSERT ON factor BEGIN SELECT RAISE(ABORT, 'factor failed'); END;"
        }).expect("fault trigger");
    assert!(!apply(&mut harness, &id).ok);
    reopen(&mut harness, &dir);
    assert!(
        harness
            .scrollback
            .proposal(&id)
            .expect("proposal")
            .expect("kept")
            .applied
            .is_none(),
        "{point}"
    );
    assert!(harness.rows().iter().all(|t| t.state == State::Live));
    assert!(
        harness
            .scrollback
            .measured("atomic-model")
            .expect("factor")
            .is_none()
    );
    faults
        .execute_batch("DROP TRIGGER fail_layout")
        .expect("remove fault");
    assert!(apply(&mut harness, &id).ok);
    assert!(apply(&mut harness, &id).ok);
    assert!(harness.rows().iter().all(|t| t.state == State::Masked));
    assert_eq!(
        harness.scrollback.measured("atomic-model").expect("factor"),
        Some((1.2, 1))
    );
}

#[test]
fn failed_witness_is_replayed_after_reopening() {
    cross_store_fault("witness");
}

#[test]
fn failed_rescore_is_replayed_without_duplicate_witnesses() {
    cross_store_fault("score");
}

fn cross_store_fault(point: &str) {
    let (mut harness, dir, memories) = durable(point);
    let id = proposal(&harness, true);
    let faults = rusqlite::Connection::open(dir.join("memory.db")).expect("faults");
    faults.execute_batch(if point == "witness" {
            "CREATE TRIGGER fail_effect BEFORE INSERT ON witness BEGIN SELECT RAISE(ABORT, 'witness failed'); END;"
        } else {
            "CREATE TRIGGER fail_effect BEFORE UPDATE OF confidence ON memory BEGIN SELECT RAISE(ABORT, 'score failed'); END;"
        }).expect("effect fault");
    assert!(!apply(&mut harness, &id).ok);
    reopen(&mut harness, &dir);
    faults
        .execute_batch("DROP TRIGGER fail_effect")
        .expect("remove fault");
    assert!(apply(&mut harness, &id).ok);
    assert!(apply(&mut harness, &id).ok);
    reopen(&mut harness, &dir);
    for memory in memories {
        assert_eq!(distillations(&harness, &memory), 1, "{point}");
        assert!(
            harness
                .store
                .get(&memory)
                .expect("memory")
                .expect("kept")
                .confidence
                > 0.0
        );
    }
    assert_eq!(
        harness.scrollback.measured("atomic-model").expect("factor"),
        Some((1.2, 1))
    );
}
