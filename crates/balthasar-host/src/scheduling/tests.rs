use super::*;
use balthasar_model::SessionId;
use serde_json::json;

fn job(id: &str, kind: &str, state: JobState, blocking: bool) -> Job {
    Job {
        id: id.to_owned(),
        session: SessionId::new("s-1"),
        kind: kind.to_owned(),
        state,
        attempts: 0,
        spec: json!({ "blocking": blocking }),
        context: json!({}),
        result: None,
        issued: None,
    }
}

fn queued(id: &str, kind: &str) -> Job {
    job(id, kind, JobState::Queued, false)
}

/// A backlog that came due together leaves in batches, not all at once.
mod mem_scheduler_coalesces_pressure {
    use super::*;

    #[test]
    fn nothing_more_goes_out_once_the_high_mark_is_reached() {
        assert_eq!(Room::given(HIGH, false), Room::Full);
        assert_eq!(Room::given(HIGH + 5, false), Room::Full);
    }

    #[test]
    fn what_is_left_under_the_high_mark_is_what_may_go() {
        assert_eq!(Room::given(0, false), Room::For(HIGH));
        assert_eq!(Room::given(HIGH - 1, false), Room::For(1));
    }

    #[test]
    fn a_pass_that_filled_up_waits_for_the_low_mark_rather_than_topping_up() {
        // Topping up one at a time keeps the queue permanently at the high mark, and every job
        // in it is a model call. Draining to the low mark batches them instead.
        assert_eq!(Room::given(HIGH - 1, true), Room::Full);
        assert_eq!(Room::given(LOW, true), Room::For(HIGH - LOW));
    }

    #[test]
    fn a_job_a_request_waits_on_is_urgent_and_the_rest_are_not() {
        assert!(urgent(&job("J-1", "curate", JobState::Queued, true)));
        assert!(!urgent(&queued("J-2", "curate")));
        assert!(!urgent(&Job {
            spec: json!({}),
            ..queued("J-3", "extract")
        }));
    }
}

/// When several come due together, what the request in front of us needs goes first.
mod ordering {
    use super::*;

    #[test]
    fn the_queue_is_read_in_the_order_that_serves_the_next_request() {
        let jobs = vec![
            queued("J-1", "tidy"),
            queued("J-2", "retitle"),
            queued("J-3", "extract"),
            queued("J-4", "curate"),
        ];
        let out: Vec<&str> = ordered(&jobs).iter().map(|j| j.kind.as_str()).collect();
        assert_eq!(out, ["curate", "extract", "retitle", "tidy"]);
    }

    #[test]
    fn two_of_a_kind_keep_the_order_they_were_queued_in() {
        let jobs = vec![queued("J-1", "extract"), queued("J-2", "extract")];
        let out: Vec<&str> = ordered(&jobs).iter().map(|j| j.id.as_str()).collect();
        assert_eq!(out, ["J-1", "J-2"]);
    }

    #[test]
    fn a_kind_nobody_ranked_goes_last_rather_than_first() {
        let jobs = vec![queued("J-1", "something-new"), queued("J-2", "tidy")];
        let out: Vec<&str> = ordered(&jobs).iter().map(|j| j.id.as_str()).collect();
        assert_eq!(out, ["J-2", "J-1"]);
        assert!(rank("something-new") >= rank("tidy"));
    }

    #[test]
    fn what_is_not_queued_is_not_in_the_queue() {
        let jobs = vec![
            job("J-1", "extract", JobState::Issued, false),
            job("J-2", "extract", JobState::Done, false),
            job("J-3", "extract", JobState::Failed, false),
            queued("J-4", "tidy"),
        ];
        let out: Vec<&str> = ordered(&jobs).iter().map(|j| j.id.as_str()).collect();
        assert_eq!(out, ["J-4"]);
    }
}
