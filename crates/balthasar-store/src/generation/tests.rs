use super::*;
use serde_json::json;

const NOW: Timestamp = 1_756_000_000;

fn held() -> Transcript {
    Transcript::ephemeral().expect("scrollback")
}

fn session() -> SessionId {
    SessionId::new("01GEN")
}

fn built(id: &str, covers: (u64, u64)) -> Generation {
    Generation {
        id: id.to_owned(),
        session: session(),
        state: Ready::Building,
        parent: None,
        covers,
        policy: json!({ "capacity": 128_000, "limit": 64_000 }),
        provenance: json!({ "version": 1, "sources": [] }),
        content: format!("working state {id}"),
        made: NOW,
    }
}

/// One generation replaces another, or neither does.
mod mem_generation_atomic_activation {
    use super::*;

    #[test]
    fn activating_one_retires_what_was_active() {
        let store = held();
        for (id, covers) in [("g1", (0, 9)), ("g2", (0, 19))] {
            store.put_generation(&built(id, covers)).expect("put");
            store.ready_generation(id).expect("ready");
        }
        assert_eq!(
            store.activate_generation("g1", None, None).expect("db"),
            Ok(())
        );
        assert_eq!(
            store
                .active_generation(&session())
                .expect("db")
                .map(|g| g.id),
            Some("g1".to_owned())
        );

        assert_eq!(
            store.activate_generation("g2", None, None).expect("db"),
            Ok(())
        );
        assert_eq!(
            store
                .active_generation(&session())
                .expect("db")
                .map(|g| g.id),
            Some("g2".to_owned()),
            "the newer one is what a request is laid out from"
        );
        assert_eq!(
            store.generation("g1").expect("db").expect("kept").state,
            Ready::Retired,
            "the older one is retired, not deleted"
        );
    }

    #[test]
    fn a_session_never_has_two_active_at_once() {
        // Carried by the index rather than the code: a process that dies between the retire and
        // the activate reopens on one of them, never both.
        let store = held();
        for id in ["g1", "g2"] {
            store.put_generation(&built(id, (0, 9))).expect("put");
            store.ready_generation(id).expect("ready");
            assert_eq!(
                store.activate_generation(id, None, None).expect("db"),
                Ok(())
            );
        }
        let active: Vec<_> = store
            .generations_of(&session())
            .expect("db")
            .into_iter()
            .filter(|g| g.state == Ready::Active)
            .collect();
        assert_eq!(active.len(), 1, "{active:?}");
    }

    #[test]
    fn only_a_ready_one_may_be_activated() {
        let store = held();
        store.put_generation(&built("g1", (0, 9))).expect("put");
        assert_eq!(
            store.activate_generation("g1", None, None).expect("db"),
            Err(Stale::NotReady(Ready::Building)),
            "a half-built generation is not offered"
        );
        assert!(store.active_generation(&session()).expect("db").is_none());
    }

    #[test]
    fn a_generation_nobody_wrote_cannot_be_activated() {
        assert_eq!(
            held()
                .activate_generation("nothing", None, None)
                .expect("db"),
            Err(Stale::Unknown)
        );
    }
}

/// A generation describing rows that have since changed is not activated.
mod mem_stale_source_rejected {
    use super::*;

    #[test]
    fn an_amendment_inside_its_span_makes_it_stale() {
        let store = held();
        store.put_generation(&built("g1", (10, 19))).expect("put");
        store.ready_generation("g1").expect("ready");
        assert_eq!(
            store.activate_generation("g1", Some(12), None).expect("db"),
            Err(Stale::Amended { at: 12 }),
            "a row it represents was revised after it was built"
        );
        assert!(store.active_generation(&session()).expect("db").is_none());
    }

    #[test]
    fn an_amendment_at_its_first_row_still_makes_it_stale() {
        let store = held();
        store.put_generation(&built("g1", (10, 19))).expect("put");
        store.ready_generation("g1").expect("ready");
        assert!(
            store
                .activate_generation("g1", Some(10), None)
                .expect("db")
                .is_err()
        );
    }
}

/// Rows appended after a generation was built do not invalidate what it already represents.
mod mem_append_preserves_usable_prefix {
    use super::*;

    #[test]
    fn an_amendment_after_its_span_leaves_it_usable() {
        // The distinction the plan asks for: appended suffixes are harmless, amendments to
        // already represented rows are not.
        let store = held();
        store.put_generation(&built("g1", (0, 9))).expect("put");
        store.ready_generation("g1").expect("ready");
        assert_eq!(
            store.activate_generation("g1", Some(10), None).expect("db"),
            Ok(())
        );
        assert_eq!(
            store
                .active_generation(&session())
                .expect("db")
                .map(|g| g.covers),
            Some((0, 9)),
            "it still represents its own rows"
        );
    }

    #[test]
    fn nothing_amended_at_all_is_usable() {
        let store = held();
        store.put_generation(&built("g1", (0, 9))).expect("put");
        store.ready_generation("g1").expect("ready");
        assert_eq!(
            store.activate_generation("g1", None, None).expect("db"),
            Ok(())
        );
    }
}

/// What a reopened store finds, and what it keeps.
mod mem_generation_reopen_recovery {
    use super::*;

    #[test]
    fn a_rebuilt_candidate_overwrites_its_own_partial_work() {
        // A preparer that died and retried names the same id rather than leaving two halves.
        let store = held();
        store.put_generation(&built("g1", (0, 9))).expect("put");
        let mut again = built("g1", (0, 19));
        again.content = "the whole of it".to_owned();
        store.put_generation(&again).expect("put again");
        let found = store.generation("g1").expect("db").expect("one");
        assert_eq!(found.covers, (0, 19));
        assert_eq!(found.content, "the whole of it");
        assert_eq!(store.generations_of(&session()).expect("db").len(), 1);
    }

    #[test]
    fn everything_written_is_read_back_as_it_was() {
        let store = held();
        let one = built("g1", (3, 7));
        store.put_generation(&one).expect("put");
        let found = store.generation("g1").expect("db").expect("one");
        assert_eq!(found, one);
    }

    #[test]
    fn retired_generations_are_bounded_without_touching_the_originals() {
        let store = held();
        for n in 0..5 {
            let id = format!("g{n}");
            store.put_generation(&built(&id, (0, n))).expect("put");
            store.ready_generation(&id).expect("ready");
            assert_eq!(
                store.activate_generation(&id, None, None).expect("db"),
                Ok(())
            );
        }
        // Four retired, one active.
        assert_eq!(store.trim_generations(&session(), 2).expect("trim"), 2);
        let left = store.generations_of(&session()).expect("db");
        assert_eq!(left.len(), 3, "{left:?}");
        assert_eq!(
            left.iter().filter(|g| g.state == Ready::Active).count(),
            1,
            "the active one is never trimmed"
        );
    }
}

/// The schema arrives the same way however many times it is applied.
mod mem_generation_migration_is_idempotent {
    use super::*;

    #[test]
    fn applying_the_schema_again_changes_nothing() {
        let store = held();
        store.put_generation(&built("g1", (0, 9))).expect("put");
        store.db().execute_batch(SCHEMA).expect("again");
        store.db().execute_batch(SCHEMA).expect("and again");
        assert_eq!(
            store.generation("g1").expect("db").map(|g| g.covers),
            Some((0, 9)),
            "what was written survived the migration being reapplied"
        );
    }
}

/// A generation prepared for more room than the request now has is not activated.
mod outgrown {
    use super::*;

    fn prepared_for(limit: u64) -> Generation {
        let mut held = built("g1", (0, 9));
        held.policy = json!({ "capacity": 128_000, "limit": limit });
        held
    }

    #[test]
    fn a_smaller_limit_than_it_was_prepared_for_refuses_it() {
        let store = held();
        store.put_generation(&prepared_for(64_000)).expect("put");
        store.ready_generation("g1").expect("ready");
        assert_eq!(
            store
                .activate_generation("g1", None, Some(16_000))
                .expect("db"),
            Err(Stale::Outgrown {
                prepared_for: 64_000,
                now: 16_000
            }),
            "it was built to fill room the request no longer has"
        );
    }

    #[test]
    fn the_same_or_more_room_is_fine() {
        for now in [64_000_u64, 128_000] {
            let store = held();
            store.put_generation(&prepared_for(64_000)).expect("put");
            store.ready_generation("g1").expect("ready");
            assert_eq!(
                store
                    .activate_generation("g1", None, Some(now))
                    .expect("db"),
                Ok(()),
                "limit now {now}"
            );
        }
    }

    #[test]
    fn a_caller_that_names_no_limit_is_not_second_guessed() {
        let store = held();
        store.put_generation(&prepared_for(64_000)).expect("put");
        store.ready_generation("g1").expect("ready");
        assert_eq!(
            store.activate_generation("g1", None, None).expect("db"),
            Ok(())
        );
    }
}

/// A piece recorded twice is one reading.
mod mem_duplicate_chunk_applies_once {
    use super::*;

    const ROW: u64 = 7;
    const REV: &str = "abc123";

    #[test]
    fn recording_the_same_piece_again_changes_nothing() {
        // A replayed answer names pieces already read; counting one twice would say a row was
        // read further than it was.
        let store = held();
        let piece = Piece {
            offset: 0,
            length: 10,
        };
        for _ in 0..3 {
            store
                .read_piece(&session(), ROW, REV, piece)
                .expect("record");
        }
        assert_eq!(
            store.pieces_read(&session(), ROW, REV).expect("read"),
            vec![piece]
        );
    }

    #[test]
    fn the_same_offset_of_another_revision_is_a_different_piece() {
        let store = held();
        let piece = Piece {
            offset: 0,
            length: 10,
        };
        store.read_piece(&session(), ROW, REV, piece).expect("one");
        store
            .read_piece(&session(), ROW, "other", piece)
            .expect("two");
        assert_eq!(
            store.pieces_read(&session(), ROW, REV).expect("read").len(),
            1
        );
        assert_eq!(
            store
                .pieces_read(&session(), ROW, "other")
                .expect("read")
                .len(),
            1
        );
    }

    #[test]
    fn pieces_come_back_lowest_first_whatever_order_they_went_in() {
        let store = held();
        for offset in [20_usize, 0, 10] {
            store
                .read_piece(&session(), ROW, REV, Piece { offset, length: 10 })
                .expect("record");
        }
        let read = store.pieces_read(&session(), ROW, REV).expect("read");
        assert_eq!(
            read.iter().map(|p| p.offset).collect::<Vec<_>>(),
            [0, 10, 20]
        );
    }

    #[test]
    fn a_row_nothing_was_read_from_has_no_pieces() {
        assert!(
            held()
                .pieces_read(&session(), ROW, REV)
                .expect("read")
                .is_empty()
        );
    }
}
