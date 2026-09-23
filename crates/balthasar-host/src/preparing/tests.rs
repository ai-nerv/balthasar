use super::*;
use balthasar_model::{AgentId, ScopeId, Timestamp, floor};
use balthasar_store::{JobState, Store, Transcript, Turn};

const NOW: Timestamp = 1_756_000_000;
const SCOPE: &str = "/w/thing";

struct Plain;
impl crate::Hooks for Plain {}

fn session() -> SessionId {
    SessionId::new("s-1")
}

struct Held {
    store: Store,
    scrollback: Transcript,
}

impl Held {
    fn new() -> Self {
        Self {
            store: Store::ephemeral().expect("store"),
            scrollback: Transcript::ephemeral().expect("scrollback"),
        }
    }

    fn at(&mut self) -> Answering<'_> {
        Answering {
            store: &mut self.store,
            scrollback: Some(&mut self.scrollback),
            scratch: None,
            scope: ScopeId::new(SCOPE),
            agent: AgentId::main(),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        }
    }

    fn said(&mut self, cursor: u64, role: &str, text: &str) {
        let turn = Turn {
            cursor,
            at: NOW,
            role: role.to_owned(),
            kind: "say".to_owned(),
            text: text.to_owned(),
            ..Turn::default()
        };
        self.scrollback.write(&session(), &turn).expect("write");
    }

    /// Rows read back and acknowledged, which is what lets a preparation over them be offered.
    /// Coverage runs from the start of the session, so an acknowledgment does too.
    fn observed(&mut self, span: (u64, u64)) {
        self.scrollback
            .acknowledge_extraction(&session(), &format!("e-{}", span.1), 0, span.1)
            .expect("acknowledge");
    }

    fn jobs(&self) -> Vec<balthasar_store::Job> {
        self.scrollback.jobs_of(&session()).expect("jobs")
    }

    fn working(&self) -> Option<balthasar_store::Job> {
        self.jobs().into_iter().rfind(|job| job.kind == "working")
    }

    /// A job over a span that has already been read back, which is the ordinary case.
    fn handed(&mut self, span: (u64, u64)) -> balthasar_store::Job {
        self.observed(span);
        self.unobserved(span)
    }

    /// A job over a span nothing has acknowledged yet.
    fn unobserved(&mut self, span: (u64, u64)) -> balthasar_store::Job {
        let policy = json!({ "limit": 20_000 });
        let at = self.at();
        queue(&at, &session(), span, &policy, &mut Plain).expect("queue");
        self.working().expect("a working job")
    }

    fn settle(&mut self, job: &balthasar_store::Job, text: &str) -> bool {
        let at = self.at();
        super::settle(&at, &session(), job, text).expect("settle")
    }

    fn active(&self) -> Option<Generation> {
        self.scrollback
            .active_generation(&session())
            .expect("active")
    }
}

fn answer() -> Value {
    json!({
        "goal": "make the parser accept trailing commas",
        "work": [{ "what": "read the grammar", "stage": "done", "evidence": [1] }],
        "checks": [{ "ran": "cargo test", "passed": false, "said": "1 failed", "evidence": [2] }],
        "corrections": [{ "said": "keep the old behaviour behind a flag", "cursor": 1 }],
        "open": ["decide the flag's name"],
        "references": ["src/parse.rs:120"],
    })
}

mod queueing {
    use super::*;

    #[test]
    fn a_span_about_to_be_cut_is_prepared_from() {
        let mut held = Held::new();
        held.said(1, "user", "make the parser accept trailing commas");
        held.said(2, "assistant", "cargo test says 1 failed");
        let job = held.handed((1, 2));
        assert_eq!(job.spec["role"], "working");
        assert_eq!(job.context["from"], 1);
        assert_eq!(job.context["to"], 2);
        let input = job.spec["input"].as_str().expect("input");
        assert!(input.contains("[1] person:"), "{input}");
        assert!(input.contains("trailing commas"), "{input}");
    }

    #[test]
    fn only_one_preparation_is_outstanding_at_a_time() {
        // A second while the first is unanswered would have both fold the same rows into the
        // same parent, and the later one would win by arriving later.
        let mut held = Held::new();
        held.said(1, "user", "one");
        held.handed((1, 1));
        held.said(2, "user", "two");
        let at = held.at();
        queue(&at, &session(), (1, 2), &json!({}), &mut Plain).expect("queue");
        assert_eq!(
            held.jobs().iter().filter(|j| j.kind == "working").count(),
            1
        );
    }

    #[test]
    fn a_span_already_covered_is_not_prepared_again() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.handed((1, 1));
        assert!(held.settle(&job, &answer().to_string()));
        let id = held.working().expect("job").id;
        held.scrollback
            .activate_generation(&format!("g-{id}"), None, None)
            .expect("activate")
            .expect("ready");
        held.scrollback
            .settle_job(&id, JobState::Done, None, NOW)
            .expect("settle");
        let at = held.at();
        queue(&at, &session(), (1, 1), &json!({}), &mut Plain).expect("queue");
        assert_eq!(
            held.jobs().iter().filter(|j| j.kind == "working").count(),
            1
        );
    }

    #[test]
    fn an_empty_span_asks_for_nothing() {
        let mut held = Held::new();
        let at = held.at();
        queue(&at, &session(), (5, 1), &json!({}), &mut Plain).expect("queue");
        queue(&at, &session(), (1, 5), &json!({}), &mut Plain).expect("queue");
        assert!(held.working().is_none());
    }

    #[test]
    fn what_is_active_goes_in_so_it_can_be_carried_forward() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.handed((1, 1));
        assert!(held.settle(&job, &answer().to_string()));
        held.scrollback
            .activate_generation(&format!("g-{}", job.id), None, None)
            .expect("activate")
            .expect("ready");
        held.scrollback
            .settle_job(&job.id, JobState::Done, None, NOW)
            .expect("settle");
        held.said(2, "user", "two");
        let at = held.at();
        queue(&at, &session(), (2, 2), &json!({ "limit": 1 }), &mut Plain).expect("queue");
        let next = held
            .jobs()
            .into_iter()
            .rfind(|j| j.kind == "working")
            .expect("a second job");
        let input = next.spec["input"].as_str().expect("input");
        assert!(input.contains("The working state so far"), "{input}");
        assert!(input.contains("trailing commas"), "{input}");
        assert_eq!(next.context["parent"], format!("g-{}", job.id));
    }

    #[test]
    fn a_row_too_wide_for_the_budget_is_stubbed_rather_than_sent_whole() {
        let mut held = Held::new();
        held.said(1, "assistant", &"x".repeat(OF_EACH * 2));
        let job = held.handed((1, 1));
        let input = job.spec["input"].as_str().expect("input");
        assert!(input.len() < OF_EACH * 2, "{}", input.len());
    }
}

mod settling {
    use super::*;

    #[test]
    fn a_sound_answer_becomes_a_generation_that_is_ready_but_not_active() {
        // Ready rather than active: what a request is laid out from changes at a turn boundary,
        // not when a background helper happens to answer.
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.handed((1, 1));
        assert!(held.settle(&job, &answer().to_string()));
        assert!(held.active().is_none());
        let made = held
            .scrollback
            .generation(&format!("g-{}", job.id))
            .expect("read")
            .expect("a generation");
        assert_eq!(made.state, Ready::Ready);
        assert_eq!(made.covers, (1, 1));
        assert_eq!(made.policy["limit"], 20_000);
        assert_eq!(made.provenance["job"], job.id);
    }

    #[test]
    fn what_it_holds_reads_back_as_the_state_it_answered() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.handed((1, 1));
        assert!(held.settle(&job, &answer().to_string()));
        let made = held
            .scrollback
            .generation(&format!("g-{}", job.id))
            .expect("read")
            .expect("a generation");
        let state: Working = serde_json::from_str(&made.content).expect("a working state");
        assert_eq!(state.goal, "make the parser accept trailing commas");
        assert_eq!(state.checks[0].passed, Some(false));
    }

    #[test]
    fn an_answer_that_is_not_a_state_installs_nothing() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.handed((1, 1));
        assert!(!held.settle(&job, "I folded everything in, it all looks fine now"));
        assert!(
            held.scrollback
                .generations_of(&session())
                .expect("read")
                .is_empty()
        );
    }

    #[test]
    fn a_preparation_that_drops_a_failure_leaves_what_is_active_standing() {
        // The refusal is `Working::may_replace`'s; what matters here is that nothing is written,
        // so the next request is still laid out from the state that remembers the failure.
        let mut held = Held::new();
        held.said(1, "user", "one");
        let first = held.handed((1, 1));
        assert!(held.settle(&first, &answer().to_string()));
        held.scrollback
            .activate_generation(&format!("g-{}", first.id), None, None)
            .expect("activate")
            .expect("ready");
        held.scrollback
            .settle_job(&first.id, JobState::Done, None, NOW)
            .expect("settle");
        held.said(2, "user", "two");
        let second = {
            let at = held.at();
            queue(&at, &session(), (2, 2), &json!({}), &mut Plain).expect("queue");
            held.jobs()
                .into_iter()
                .rfind(|j| j.kind == "working")
                .expect("a second job")
        };
        let forgetful = json!({ "goal": "make the parser accept trailing commas",
            "corrections": [{ "said": "keep the old behaviour behind a flag", "cursor": 1 }] });
        assert!(!held.settle(&second, &forgetful.to_string()));
        let active = held.active().expect("still active");
        assert_eq!(active.id, format!("g-{}", first.id));
        assert!(
            held.scrollback
                .generation(&format!("g-{}", second.id))
                .expect("read")
                .is_none()
        );
    }

    #[test]
    fn a_retry_overwrites_its_own_partial_work_rather_than_leaving_two() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.handed((1, 1));
        assert!(held.settle(&job, &answer().to_string()));
        let mut again = answer();
        again["goal"] = json!("make the parser accept trailing commas, behind a flag");
        assert!(held.settle(&job, &again.to_string()));
        let made = held.scrollback.generations_of(&session()).expect("read");
        assert_eq!(made.len(), 1);
        assert!(made[0].content.contains("behind a flag"));
    }
}

/// Queuing a reading of the rows is not a reading of them.
mod mem_pending_extract_cannot_cut {
    use super::*;

    #[test]
    fn a_span_nothing_has_read_back_yet_is_prepared_but_not_offered() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.unobserved((1, 1));
        assert!(held.settle(&job, &answer().to_string()));
        let made = held
            .scrollback
            .generation(&format!("g-{}", job.id))
            .expect("read")
            .expect("a generation");
        assert_eq!(made.state, Ready::Building);
    }

    #[test]
    fn it_is_not_activated_at_a_boundary_either() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.unobserved((1, 1));
        assert!(held.settle(&job, &answer().to_string()));
        let at = held.at();
        assert_eq!(at_boundary(&at, &session(), None).expect("boundary"), None);
        assert!(held.active().is_none());
    }

    #[test]
    fn coverage_arriving_later_promotes_it_at_the_next_boundary() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.unobserved((1, 1));
        assert!(held.settle(&job, &answer().to_string()));
        held.observed((1, 1));
        let at = held.at();
        let now = at_boundary(&at, &session(), None).expect("boundary");
        assert_eq!(now, Some(format!("g-{}", job.id)));
    }

    #[test]
    fn coverage_short_of_the_span_is_not_coverage_of_it() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        held.said(2, "user", "two");
        let job = held.unobserved((1, 2));
        assert!(held.settle(&job, &answer().to_string()));
        held.observed((1, 1));
        let at = held.at();
        assert_eq!(at_boundary(&at, &session(), None).expect("boundary"), None);
    }
}

/// Between turns, what was prepared earlier takes effect on its own.
mod mem_ready_activation_needs_no_model_call {
    use super::*;

    #[test]
    fn activating_queues_nothing_and_asks_nobody() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.handed((1, 1));
        assert!(held.settle(&job, &answer().to_string()));
        held.scrollback
            .settle_job(&job.id, JobState::Done, None, NOW)
            .expect("settle");
        let before = held.jobs().len();
        let at = held.at();
        let now = at_boundary(&at, &session(), None).expect("boundary");
        assert_eq!(now, Some(format!("g-{}", job.id)));
        assert_eq!(held.jobs().len(), before, "a boundary asks for nothing");
        assert_eq!(held.active().expect("active").covers, (1, 1));
    }

    #[test]
    fn the_furthest_prepared_span_wins_and_the_one_before_it_retires() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let first = held.handed((1, 1));
        assert!(held.settle(&first, &answer().to_string()));
        held.scrollback
            .settle_job(&first.id, JobState::Done, None, NOW)
            .expect("settle");
        let at = held.at();
        at_boundary(&at, &session(), None).expect("boundary");
        held.said(2, "user", "two");
        let second = held.handed((1, 2));
        assert!(held.settle(&second, &answer().to_string()));
        let at = held.at();
        let now = at_boundary(&at, &session(), None).expect("boundary");
        assert_eq!(now, Some(format!("g-{}", second.id)));
        let made = held.scrollback.generations_of(&session()).expect("read");
        let first = made
            .iter()
            .find(|g| g.id == format!("g-{}", first.id))
            .expect("the first");
        assert_eq!(first.state, Ready::Retired);
    }

    #[test]
    fn a_span_amended_since_it_was_prepared_is_not_activated() {
        // The rows it stands for are not the rows it read.
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.handed((1, 1));
        assert!(held.settle(&job, &answer().to_string()));
        held.said(1, "user", "one, and another thing");
        let at = held.at();
        assert_eq!(at_boundary(&at, &session(), None).expect("boundary"), None);
        assert!(held.active().is_none());
    }

    #[test]
    fn one_prepared_for_more_room_than_the_request_has_is_not_activated() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.handed((1, 1));
        assert!(held.settle(&job, &answer().to_string()));
        let at = held.at();
        assert_eq!(
            at_boundary(&at, &session(), Some(10)).expect("boundary"),
            None,
            "prepared against 20,000 tokens of room, offered 10"
        );
        let at = held.at();
        assert!(
            at_boundary(&at, &session(), Some(20_000))
                .expect("boundary")
                .is_some()
        );
    }

    #[test]
    fn a_boundary_with_nothing_prepared_does_nothing() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let at = held.at();
        assert_eq!(at_boundary(&at, &session(), None).expect("boundary"), None);
    }
}

/// The rows a state cites stay readable however many states have been built over them.
mod mem_evidence_recall_after_many_generations {
    use super::*;

    #[test]
    fn evidence_still_resolves_to_the_original_rows() {
        let mut held = Held::new();
        held.said(1, "user", "keep the old behaviour behind a flag");
        held.said(2, "assistant", "cargo test says 1 failed");
        for to in 1..=2 {
            let job = held.handed((1, to));
            assert!(held.settle(&job, &answer().to_string()));
            held.scrollback
                .settle_job(&job.id, JobState::Done, None, NOW)
                .expect("settle");
            let at = held.at();
            at_boundary(&at, &session(), None).expect("boundary");
        }
        // Whatever retention does to derived generations, the rows they cite are originals.
        held.scrollback
            .trim_generations(&session(), 0)
            .expect("trim");
        let active = held.active().expect("active");
        let state: Working = serde_json::from_str(&active.content).expect("a state");
        let cited = state.checks[0].evidence[0];
        let rows = held
            .scrollback
            .read(
                &session(),
                &Want::Span {
                    from: cited,
                    to: cited,
                },
                &Budget {
                    tokens: usize::MAX,
                    turns: usize::MAX,
                },
            )
            .expect("read");
        let row = rows
            .turns
            .iter()
            .find(|t| t.cursor == cited)
            .expect("the cited row");
        assert_eq!(row.text, "cargo test says 1 failed");
    }
}

/// What the active working state does to the request it is projected into.
mod mem_stable_task_keeps_prefix {
    use super::*;
    use balthasar_store::Summary;

    fn summary(from: u64, to: u64) -> Summary {
        Summary {
            from,
            to,
            text: "they talked about the parser".to_owned(),
            at: NOW,
        }
    }

    fn active(held: &mut Held, span: (u64, u64)) {
        for n in span.0..=span.1 {
            held.said(n, "user", "one");
        }
        let job = held.handed(span);
        assert!(held.settle(&job, &answer().to_string()));
        let at = held.at();
        at_boundary(&at, &session(), None).expect("boundary");
    }

    #[test]
    fn with_nothing_active_the_summary_is_what_goes() {
        let held = Held::new();
        let was = summary(0, 4);
        let now = projection(&held.scrollback, &session(), Some(was.clone())).expect("projection");
        assert_eq!(now, Some(was));
    }

    #[test]
    fn an_active_state_replaces_the_summary_it_reaches_past() {
        let mut held = Held::new();
        active(&mut held, (1, 4));
        let now = projection(&held.scrollback, &session(), Some(summary(0, 2)))
            .expect("projection")
            .expect("a projection");
        assert_eq!((now.from, now.to), (1, 4));
        assert!(
            now.text
                .contains("Goal: make the parser accept trailing commas")
        );
        assert!(!now.text.contains("they talked about the parser"));
    }

    #[test]
    fn a_summary_that_reaches_further_is_left_alone() {
        // The state stands for rows the summary has already covered and gone past; swapping it in
        // would put the older, shorter span in the slot.
        let mut held = Held::new();
        active(&mut held, (1, 4));
        let was = summary(0, 40);
        let now = projection(&held.scrollback, &session(), Some(was.clone())).expect("projection");
        assert_eq!(now, Some(was));
    }

    #[test]
    fn the_persons_words_and_a_failing_check_reach_the_request() {
        let mut held = Held::new();
        active(&mut held, (1, 4));
        let now = projection(&held.scrollback, &session(), None)
            .expect("projection")
            .expect("a projection");
        assert!(
            now.text.contains("keep the old behaviour behind a flag"),
            "{}",
            now.text
        );
        assert!(now.text.contains("Still failing"), "{}", now.text);
        assert!(now.text.contains("cargo test"), "{}", now.text);
    }

    #[test]
    fn a_state_that_renders_to_nothing_leaves_the_summary_standing() {
        let mut held = Held::new();
        held.said(1, "user", "one");
        let job = held.handed((1, 1));
        assert!(held.settle(&job, &json!({}).to_string()));
        let at = held.at();
        at_boundary(&at, &session(), None).expect("boundary");
        let was = summary(0, 0);
        let now = projection(&held.scrollback, &session(), Some(was.clone())).expect("projection");
        assert_eq!(now, Some(was), "an empty state is not a projection");
    }

    #[test]
    fn projecting_twice_over_an_unchanged_state_gives_the_same_text() {
        // The slot is budgeted and counted from this text, so an unstable rendering would move
        // the prefix under a task that has not changed.
        let mut held = Held::new();
        active(&mut held, (1, 4));
        let once = projection(&held.scrollback, &session(), None).expect("projection");
        let twice = projection(&held.scrollback, &session(), None).expect("projection");
        assert_eq!(once, twice);
    }
}

/// A topic dropped long ago is still traceable to the rows it came from.
mod mem_old_topic_recalls_source {
    use super::*;

    #[test]
    fn a_reference_a_retired_state_carried_still_names_its_row() {
        let mut held = Held::new();
        held.said(
            1,
            "user",
            "the parser is in src/parse.rs and it is hand-written",
        );
        held.said(2, "assistant", "cargo test says 1 failed");
        let first = held.handed((1, 2));
        assert!(held.settle(&first, &answer().to_string()));
        held.scrollback
            .settle_job(&first.id, JobState::Done, None, NOW)
            .expect("settle");
        {
            let at = held.at();
            at_boundary(&at, &session(), None).expect("boundary");
        }
        // Many prompts later, on another topic entirely.
        for n in 3..30 {
            held.said(n, "user", "something else entirely");
        }
        let later = held.handed((3, 29));
        let mut moved_on = answer();
        moved_on["goal"] = json!("something else entirely");
        moved_on["references"] = json!([]);
        assert!(held.settle(&later, &moved_on.to_string()));
        held.scrollback
            .settle_job(&later.id, JobState::Done, None, NOW)
            .expect("settle");
        {
            let at = held.at();
            at_boundary(&at, &session(), None).expect("boundary");
        }
        held.scrollback
            .trim_generations(&session(), 0)
            .expect("trim");

        // The state that named src/parse.rs is gone; the row it named is not.
        let rows = held
            .scrollback
            .read(
                &session(),
                &Want::Span { from: 1, to: 1 },
                &Budget {
                    tokens: usize::MAX,
                    turns: usize::MAX,
                },
            )
            .expect("read");
        let row = rows.turns.iter().find(|t| t.cursor == 1).expect("row 1");
        assert!(row.text.contains("src/parse.rs"), "{}", row.text);
    }

    #[test]
    fn a_correction_is_carried_across_every_generation_that_follows_it() {
        // Not a retrieval: a correction cannot be left behind by a later state at all, so an old
        // topic's constraint is still in the projection however many generations have passed.
        let mut held = Held::new();
        held.said(1, "user", "keep the old behaviour behind a flag");
        let first = held.handed((1, 1));
        assert!(held.settle(&first, &answer().to_string()));
        held.scrollback
            .settle_job(&first.id, JobState::Done, None, NOW)
            .expect("settle");
        let at = held.at();
        at_boundary(&at, &session(), None).expect("boundary");
        held.said(2, "user", "now do something unrelated");
        let later = held.handed((1, 2));
        let forgot = json!({ "goal": "something unrelated" });
        assert!(
            !held.settle(&later, &forgot.to_string()),
            "a state that drops the correction is not installed"
        );
        let active = held.active().expect("the first still stands");
        assert!(
            active
                .content
                .contains("keep the old behaviour behind a flag")
        );
    }
}
