use super::*;

const NOW: Timestamp = 1_756_000_000;

fn session() -> SessionId {
    SessionId::new("s-1")
}

fn since_of(progress: u64, seconds: Timestamp) -> Since {
    Since { progress, seconds }
}

/// Being asked more often does not buy more jobs.
mod mem_poll_rate_does_not_raise_jobs {
    use super::*;

    #[test]
    fn nothing_new_is_never_due_however_long_it_has_been() {
        let held = Transcript::ephemeral().expect("scrollback");
        mark(&held, &session(), "title_asked", 12, NOW).expect("mark");
        for polls in 0..100 {
            let at = NOW + polls * 3_600;
            let since = since(&held, &session(), "title_asked", 12, at).expect("since");
            assert!(!due(since, 8), "poll {polls} raised a job out of nothing");
        }
    }

    #[test]
    fn asking_does_not_itself_count_as_progress() {
        // The old rule counted the rounds it was offered, so a harness that laid out twice as
        // often asked twice as often over the same conversation.
        let held = Transcript::ephemeral().expect("scrollback");
        mark(&held, &session(), "title_asked", 12, NOW).expect("mark");
        let later = NOW + APART;
        for _ in 0..50 {
            let since = since(&held, &session(), "title_asked", 13, later).expect("since");
            assert!(!due(since, 8), "one new row is not eight");
        }
    }

    #[test]
    fn enough_new_source_is_due_once_and_then_not_again() {
        let held = Transcript::ephemeral().expect("scrollback");
        mark(&held, &session(), "title_asked", 12, NOW).expect("mark");
        let later = NOW + APART;
        let first = since(&held, &session(), "title_asked", 20, later).expect("since");
        assert!(due(first, 8), "eight new rows is eight");
        mark(&held, &session(), "title_asked", 20, later).expect("mark");
        let next = since(&held, &session(), "title_asked", 20, later + 10_000).expect("since");
        assert!(!due(next, 8));
    }
}

mod bounds {
    use super::*;

    #[test]
    fn one_that_has_never_run_is_due_as_soon_as_there_is_enough() {
        let held = Transcript::ephemeral().expect("scrollback");
        let since = since(&held, &session(), "title_asked", 8, NOW).expect("since");
        assert_eq!(since.progress, 8);
        assert_eq!(since.seconds, Timestamp::MAX);
        assert!(due(since, 8));
    }

    #[test]
    fn a_source_that_moves_fast_is_a_reason_to_ask_sooner_not_later() {
        // A configured threshold is the whole rule: holding a burst back for a minute would
        // override what a configuration asked for, and progress already stops idle polling.
        assert!(due(since_of(1_000, 0), 8));
        assert!(due(since_of(8, 0), 8));
        assert!(
            !due(since_of(7, 10_000), 8),
            "and the threshold still binds"
        );
    }

    #[test]
    fn a_threshold_of_zero_asks_for_no_threshold_and_is_held_apart_by_time_instead() {
        // Without one, every pass would qualify; the floor is what stops that and nothing else.
        assert!(due(since_of(0, APART), 0));
        assert!(!due(since_of(0, APART - 1), 0));
    }

    #[test]
    fn a_mark_that_ran_ahead_of_the_source_does_not_wrap() {
        let held = Transcript::ephemeral().expect("scrollback");
        mark(&held, &session(), "clash_over", 40, NOW).expect("mark");
        let since = since(&held, &session(), "clash_over", 10, NOW + 10_000).expect("since");
        assert_eq!(since.progress, 0, "forgetting claims is not progress");
        assert!(!due(since, 8));
    }

    #[test]
    fn each_session_keeps_its_own_mark() {
        // The count used to be the project's, so two busy runs together reached a shared
        // threshold twice as fast as either did.
        let held = Transcript::ephemeral().expect("scrollback");
        mark(&held, &session(), "title_asked", 40, NOW).expect("mark");
        let other = SessionId::new("s-2");
        let since = since(&held, &other, "title_asked", 8, NOW).expect("since");
        assert_eq!(since.progress, 8);
    }
}
