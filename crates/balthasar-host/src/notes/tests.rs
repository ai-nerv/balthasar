use super::*;

#[test]
fn a_day_is_written_as_a_date() {
    assert_eq!(day(0), "1970-01-01");
    assert_eq!(day(1_756_000_000), "2025-08-24");
    assert_eq!(day(951_782_400), "2000-02-29");
}

#[test]
fn memory_settings_fall_back_to_the_shipped_ones() {
    assert_eq!(Keeping::read(None), Keeping::default());
    let said = json!({ "review": true, "extract_every": 3, "tidy_every": 0 });
    let keeping = Keeping::read(Some(&said));
    assert!(keeping.review);
    assert_eq!(keeping.extract_every, 3);
    assert_eq!(
        keeping.tidy_every, 10,
        "zero would tidy after every extraction"
    );
    assert_eq!(keeping.contradict_every, 8);
    assert_eq!(
        Keeping::read(Some(&json!({ "contradict_every": 3 }))).contradict_every,
        3
    );
}

/// What decides that a span is worth extracting from.
mod extraction_due {
    use crate::notes::extraction::{Progress, due};

    /// A span of `users` user rows and `rows` rows in all, neither cut nor retried.
    fn span(users: usize, rows: usize) -> Progress {
        Progress {
            users,
            rows,
            cut: false,
            retry: false,
        }
    }

    const ROWS: u32 = 40;

    #[test]
    fn a_span_comes_due_once_enough_user_rows_are_in_it() {
        assert!(!due(span(0, 0), 3, ROWS));
        assert!(!due(span(2, 4), 3, ROWS));
        assert!(due(span(3, 6), 3, ROWS));
        assert!(due(span(9, 18), 3, ROWS));
    }

    #[test]
    fn a_cut_or_a_retry_asks_for_it_whatever_the_count() {
        // A cut is the last chance to read rows before they leave the live set.
        let asked = |cut, retry| {
            due(
                Progress {
                    users: 0,
                    rows: 0,
                    cut,
                    retry,
                },
                3,
                ROWS,
            )
        };
        assert!(asked(true, false));
        assert!(asked(false, true));
    }

    #[test]
    fn mem_tool_only_run_advances_coverage() {
        // One user row and a long autonomous run after it: counted in user rows alone this never
        // came due, so what the run read was never read back. Rows of any kind bring it due.
        assert!(
            !due(span(0, 10), 3, ROWS),
            "a short run still waits for a user turn"
        );
        assert!(
            due(span(0, 40), 3, ROWS),
            "a run of tool rows alone comes due on what it added"
        );
        assert!(due(span(0, 400), 3, ROWS));
    }

    #[test]
    fn counting_rows_can_be_turned_off_without_stalling_user_counting() {
        // `extract_rows = 0` waits for user turns alone, which is what it meant before.
        assert!(!due(span(0, 1_000), 3, 0));
        assert!(due(span(3, 1_000), 3, 0));
    }

    #[test]
    fn asking_for_every_nothing_comes_due_at_once() {
        // `extract_every = 0` means every span is due, which is what a zero setting has to mean
        // if it is not to stall extraction entirely.
        assert!(due(span(0, 0), 0, ROWS));
    }
}
