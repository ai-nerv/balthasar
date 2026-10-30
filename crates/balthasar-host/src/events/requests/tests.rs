use super::*;
use crate::events::{Emitted, Emitter, Fact, LayoutOutcome};
use serde_json::{Value, json};

fn id(n: u64) -> Id {
    Id::try_from(format!("{n:032x}")).expect("identity")
}

fn source() -> Source {
    Source {
        parent: process::getpid(),
        uid: process::geteuid(),
        root: Lineage::root(id(1), id(2)),
    }
}

fn request() -> Value {
    json!({"schema":1,"run_id":String::from(id(1)),"agent_id":String::from(id(2)),
        "turn_id":String::from(id(3)),"ask_id":String::from(id(4))})
}

#[test]
fn only_a_bounded_matching_context_from_the_saved_parent_is_accepted() {
    let source = source();
    let accepted = source
        .accept(request(), source.parent, source.uid)
        .expect("parent context");
    assert_eq!(accepted.turn_id, Some(id(3)));
    assert_eq!(accepted.ask_id, Some(id(4)));
    assert_eq!(accepted.run_id, source.root.run_id);
    assert_eq!(accepted.agent_id, source.root.agent_id);
    let other = Pid::from_raw(source.parent.as_raw_pid() + 1).expect("other pid");
    assert!(source.accept(request(), other, source.uid).is_none());
    assert!(
        source
            .accept(
                request(),
                source.parent,
                Uid::from_raw(source.uid.as_raw() ^ 1)
            )
            .is_none()
    );
    for (field, bad) in [
        ("schema", json!(2)),
        ("schema", json!("1")),
        ("run_id", json!(String::from(id(8)))),
        ("agent_id", Value::Null),
        ("turn_id", json!("private-value")),
        ("ask_id", json!("0".repeat(33))),
        ("content", json!("private-content")),
        ("content", json!("x".repeat(1024))),
    ] {
        let mut value = request();
        value[field] = bad;
        assert!(
            source.accept(value, source.parent, source.uid).is_none(),
            "{field}"
        );
    }
    for value in [Value::Null, json!([]), json!({}), json!("private")] {
        assert!(source.accept(value, source.parent, source.uid).is_none());
    }
}

#[test]
fn captured_facts_do_not_rebind_to_later_requests_or_other_threads() {
    let root = source().root;
    let mut tasks = Vec::new();
    for n in 1..=16 {
        tasks.push(std::thread::spawn(move || {
            let bound = Lineage {
                turn_id: Some(id(100 + n)),
                ask_id: Some(id(200 + n)),
                ..root
            };
            let local = Correlation {
                job_id: Some(id(300 + n)),
                ..Correlation::default()
            };
            let captured = scoped(Some(bound), || {
                let captured = capture(local);
                std::thread::yield_now();
                scoped(None, || assert_eq!(capture(local), local));
                assert_eq!(capture(local), captured);
                assert!(
                    std::panic::catch_unwind(|| scoped(None, || panic!("fixture unwind"))).is_err()
                );
                assert_eq!(capture(local), captured);
                assert_eq!(
                    std::thread::spawn(move || capture(local))
                        .join()
                        .expect("detached"),
                    local
                );
                captured
            });
            assert_eq!(capture(local), local);
            let (mut emitter, receiver) = Emitter::new(id(400 + n), [1, 2, 3]);
            emitter.lineage = root;
            scoped(
                Some(Lineage {
                    turn_id: Some(id(999)),
                    ..root
                }),
                || {
                    assert_eq!(
                        emitter.emit(|| (
                            captured,
                            Fact::Layout {
                                outcome: LayoutOutcome::Decided,
                                duration_ms: None
                            }
                        )),
                        Emitted::Queued
                    );
                },
            );
            let event =
                crate::events::decode(&receiver.try_recv().expect("event")).expect("decode");
            assert!(bound.accepts(event.correlation));
            assert_eq!(event.correlation.job_id, local.job_id);
        }));
    }
    for task in tasks {
        task.join().expect("request worker");
    }
}
