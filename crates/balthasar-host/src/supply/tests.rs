use super::*;
use balthasar_model::{Body, Memory, MemoryId, ScopeId, Tier, Timestamp};
use balthasar_store::Turn;

const NOW: Timestamp = 1_756_000_000;

fn turn(cursor: u64, role: &str, error: bool) -> Turn {
    Turn {
        cursor,
        at: NOW,
        role: role.to_owned(),
        kind: if role == "user" { "user" } else { "tool" }.to_owned(),
        tool: (role != "user").then(|| "read".to_owned()),
        error,
        ..Turn::default()
    }
}

fn held(through: u64) -> Value {
    json!({ "query": "why does it fail", "text": "…", "ids": ["m-1"], "tokens": 4,
            "through": through })
}

/// A memory chosen once per prompt is not chosen once per conversation.
mod mem_relevance_tracks_new_error_without_user_turn {
    use super::*;

    #[test]
    fn a_failing_tool_call_reopens_the_selection() {
        // Nobody types when a command fails mid-run. If only a prompt could refresh this, what
        // the project knows about that very error would wait for the person to notice it.
        let rows = vec![turn(1, "user", false), turn(2, "assistant", true)];
        assert!(moved(&rows, Some(&held(1))));
        assert!(Supplied::kept(Some(&held(1)), "why does it fail", true).is_none());
    }

    #[test]
    fn a_clean_run_of_tool_rows_keeps_the_one_it_has() {
        let mut rows = vec![turn(1, "user", false)];
        rows.extend((2..200).map(|n| turn(n, "assistant", false)));
        assert!(!moved(&rows, Some(&held(1))));
        assert!(Supplied::kept(Some(&held(1)), "why does it fail", false).is_some());
    }

    #[test]
    fn a_failure_already_accounted_for_does_not_reopen_it_again() {
        let rows = vec![turn(1, "user", false), turn(2, "assistant", true)];
        assert!(!moved(&rows, Some(&held(2))), "selected after the failure");
    }

    #[test]
    fn a_selection_kept_before_this_was_recorded_is_taken_as_moved() {
        let old = json!({ "query": "why does it fail", "text": "…", "ids": [], "tokens": 0 });
        assert!(
            moved(&[], Some(&old)),
            "no mark is no evidence it still holds"
        );
    }

    #[test]
    fn nothing_kept_is_nothing_to_reopen() {
        assert!(!moved(&[turn(1, "user", false)], None));
    }
}

/// Something a person said, arriving without being a new prompt.
mod mem_new_correction_invalidates_cached_selection {
    use super::*;

    #[test]
    fn a_turn_from_a_person_or_another_agent_reopens_it() {
        let rows = vec![turn(1, "user", false), turn(2, "user", false)];
        assert!(moved(&rows, Some(&held(1))));
    }

    #[test]
    fn and_the_query_alone_still_settles_it_when_nothing_moved() {
        assert!(Supplied::kept(Some(&held(9)), "something else", false).is_none());
        assert!(Supplied::kept(Some(&held(9)), "why does it fail", false).is_some());
    }
}

/// Two copies of one claim are one claim's worth of room.
mod mem_duplicate_memory_does_not_fill_budget {
    use super::*;

    #[test]
    fn the_same_text_is_written_once() {
        assert_eq!(alike("Use uv, not pip."), alike("use uv, not pip"));
        assert_eq!(alike("Use  uv,\n not pip"), alike("Use uv, not pip!"));
    }

    #[test]
    fn claims_that_differ_are_not_folded_together() {
        assert_ne!(alike("Use uv, not pip"), alike("Use pip, not uv"));
        assert_ne!(alike("tests run on CI"), alike("tests run on a GPU"));
    }

    #[test]
    fn a_difference_only_of_punctuation_inside_the_line_still_counts() {
        assert_ne!(alike("run make, then test"), alike("run make then test"));
    }
}

/// A memory certain enough to be written under "From memory:".
fn memory(text: &str) -> balthasar_store::Scored {
    let mut held = Memory::new(
        MemoryId::new(format!("M-{}", text.trim_end_matches(['.', '!']).len())),
        Tier::Fact,
        ScopeId::new("/w/thing"),
        Body::fact("project", "rule", text),
        NOW,
    );
    held.confidence = 1.0;
    balthasar_store::Scored {
        memory: held,
        score: 1.0,
        semantic: None,
        lexical: 1.0,
        entity: 0.0,
        frecency: 0.0,
        confidence: 1.0,
        strength: 1.0,
        near: true,
    }
}

#[test]
fn a_packed_slot_never_writes_one_claim_twice() {
    struct Plain;
    impl crate::Hooks for Plain {}
    let mut store = balthasar_store::Store::ephemeral().expect("store");
    let at = Answering {
        store: &mut store,
        scrollback: None,
        scratch: None,
        scope: ScopeId::new("/w/thing"),
        agent: balthasar_model::AgentId::main(),
        now: NOW,
        inject_floor: 0.0,
        live_floor: 0.0,
        capture: false,
    };
    let found = [
        memory("Use uv, not pip."),
        memory("use uv, not pip"),
        memory("Tests run on CI"),
    ];
    let packed = pack(&at, &found, 200, 4, &mut Plain).expect("a slot");
    assert_eq!(
        packed.text.matches("uv, not pip").count(),
        1,
        "{}",
        packed.text
    );
    assert!(packed.text.contains("Tests run on CI"), "{}", packed.text);
}
