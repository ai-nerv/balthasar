//! Where a run's memories end up, and what deleting one run costs.
//!
//! The contract the storage restructure exists for: a session's scratch is in that session's
//! own file, so removing one run removes one directory and nothing else.

use balthasar_model::scratch::Scratch;

use balthasar_host::{Answering, Door, answer};
use balthasar_ipc::{Reply, Request};
use balthasar_model::{AgentId, ScopeId, SessionId, floor};
use balthasar_store::{Scratchpad, Store, Transcript};

const NOW: balthasar_model::Timestamp = 1_756_000_000;

fn scratch(name: &str) -> Scratch {
    Scratch::new("balthasar-layout", name)
}

struct Held {
    store: Store,
    scrollback: Transcript,
    scratch: Scratchpad,
}

impl Held {
    fn under(home: &std::path::Path) -> Self {
        Self {
            store: Store::ephemeral().expect("store"),
            scrollback: Transcript::ephemeral().expect("scrollback"),
            scratch: Scratchpad::at(home),
        }
    }

    fn ask(&mut self, name: &str, args: Vec<serde_json::Value>) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: Some(&mut self.scrollback),
            scratch: Some(&mut self.scratch),
            scope: ScopeId::new("/w/thing"),
            agent: AgentId::main(),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        };
        answer(
            &mut at,
            &Door::Owner,
            &Request {
                call: name.to_owned(),
                args,
            },
        )
    }

    /// The same call, through the socket a harness comes in on.
    fn peer_asks(&mut self, name: &str, args: Vec<serde_json::Value>) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: Some(&mut self.scrollback),
            scratch: Some(&mut self.scratch),
            scope: ScopeId::new("/w/thing"),
            agent: AgentId::main(),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        };
        answer(
            &mut at,
            &Door::Socket(balthasar_ipc::Peer {
                pid: 4242,
                uid: 1000,
                program: Some("/usr/bin/some-harness".to_owned()),
            }),
            &Request {
                call: name.to_owned(),
                args,
            },
        )
    }

    /// The same call, from another agent inside the same run.
    ///
    /// A second `Answering` rather than a second argument, because that is the only way a
    /// second agent exists: the identity is pinned where a connection is accepted, and a test
    /// that could pass it in the call would be testing something the wire cannot do.
    fn agent_asks(&mut self, agent: &str, name: &str, args: Vec<serde_json::Value>) -> Reply {
        let mut at = Answering {
            store: &mut self.store,
            scrollback: Some(&mut self.scrollback),
            scratch: Some(&mut self.scratch),
            scope: ScopeId::new("/w/thing"),
            agent: AgentId::new(agent),
            now: NOW,
            inject_floor: floor::INJECT,
            live_floor: floor::LIVE,
            capture: false,
        };
        answer(
            &mut at,
            &Door::Owner,
            &Request {
                call: name.to_owned(),
                args,
            },
        )
    }

    /// How many memories a reply actually carries.
    ///
    /// One row per memory, so the count is the count.
    fn found(reply: &Reply) -> usize {
        reply.result.len()
    }

    fn said(&mut self, run: &str, text: &str) {
        let reply = self.ask(
            "remember",
            vec![
                serde_json::json!(text),
                serde_json::json!({ "session": run }),
            ],
        );
        assert!(reply.ok, "remember was refused: {reply:?}");
    }
}

#[test]
fn what_a_run_says_lands_in_that_runs_own_file() {
    let home = scratch("own-file");
    let mut held = Held::under(&home);
    held.said("01RUN", "the deploy target is fly.io");

    let run = SessionId::new("01RUN");
    assert!(
        held.scratch.path_of(&run, &AgentId::main()).is_file(),
        "the run got a file of its own"
    );
    assert!(
        held.store.all().expect("all").is_empty(),
        "and the project's store stayed out of it"
    );
}

#[test]
fn a_run_that_only_asks_leaves_no_directory_behind() {
    // A harness that opens a session, searches, and finds nothing should leave the tree as it
    // found it. Otherwise a week of sessions is a week of empty directories.
    let home = scratch("read-only");
    let mut held = Held::under(&home);
    let reply = held.ask(
        "recall",
        vec![
            serde_json::json!("anything"),
            serde_json::json!({ "session": "01QUIET" }),
        ],
    );

    assert!(reply.ok, "{reply:?}");
    assert!(held.scratch.runs().is_empty(), "looking created nothing");
}

#[test]
fn a_run_can_find_what_it_was_just_told() {
    // Scratch moving to its own file must not make a session unable to search itself.
    let home = scratch("find-own");
    let mut held = Held::under(&home);
    held.said("01RUN", "the deploy target is fly.io");

    let reply = held.ask(
        "recall",
        vec![
            serde_json::json!("deploy target"),
            serde_json::json!({ "session": "01RUN" }),
        ],
    );
    assert!(reply.ok, "{reply:?}");
    assert_eq!(
        Held::found(&reply),
        1,
        "its own scratch answered: {reply:?}"
    );
}

#[test]
fn one_agent_does_not_see_anothers_scratch() {
    // The reason scratch is per agent at all. Five subagents of one run each keep working
    // notes, and one of them reading another's is how a plan that was abandoned in one branch
    // comes back as a fact in the next.
    let home = scratch("agent-isolated");
    let mut held = Held::under(&home);
    let wrote = held.agent_asks(
        "explorer",
        "remember",
        vec![
            serde_json::json!("I am about to try the staging box"),
            serde_json::json!({ "session": "01RUN" }),
        ],
    );
    assert!(wrote.ok, "{wrote:?}");

    let mine = held.agent_asks(
        "explorer",
        "recall",
        vec![
            serde_json::json!("staging box"),
            serde_json::json!({ "session": "01RUN" }),
        ],
    );
    assert_eq!(Held::found(&mine), 1, "it finds its own: {mine:?}");

    let theirs = held.agent_asks(
        "reviewer",
        "recall",
        vec![
            serde_json::json!("staging box"),
            serde_json::json!({ "session": "01RUN" }),
        ],
    );
    assert!(theirs.ok, "{theirs:?}");
    assert_eq!(
        Held::found(&theirs),
        0,
        "the sibling's scratch stayed its own: {theirs:?}"
    );
}

#[test]
fn naming_an_agent_in_a_call_reaches_nothing() {
    // The isolation guarantee, stated as the thing that would take it away: if an agent could
    // say which agent it was, it could say it was the one whose scratch it wanted to read. The
    // identity is pinned to the connection, so the option below is not a parameter — it is
    // ordinary text in a call that does not have one.
    let home = scratch("agent-not-a-parameter");
    let mut held = Held::under(&home);
    let wrote = held.agent_asks(
        "explorer",
        "remember",
        vec![
            serde_json::json!("I am about to try the staging box"),
            serde_json::json!({ "session": "01RUN" }),
        ],
    );
    assert!(wrote.ok, "{wrote:?}");

    let asked = held.agent_asks(
        "reviewer",
        "recall",
        vec![
            serde_json::json!("staging box"),
            serde_json::json!({ "session": "01RUN", "agent": "explorer" }),
        ],
    );
    assert!(asked.ok, "{asked:?}");
    assert_eq!(
        Held::found(&asked),
        0,
        "asking for a sibling's agent got a sibling's nothing: {asked:?}"
    );
    assert_eq!(
        held.scratch.agents_of(&SessionId::new("01RUN")),
        vec![AgentId::new("explorer")],
        "and the call did not bring the named agent into being"
    );
}

#[test]
fn one_run_does_not_see_anothers_scratch() {
    // Session memories are the session's own. Two runs of one project share the project's
    // durable memory and nothing else.
    let home = scratch("isolated");
    let mut held = Held::under(&home);
    held.said("01ONE", "I am about to try the staging box");

    let reply = held.ask(
        "recall",
        vec![
            serde_json::json!("staging box"),
            serde_json::json!({ "session": "01TWO" }),
        ],
    );
    assert!(reply.ok, "{reply:?}");
    assert_eq!(
        Held::found(&reply),
        0,
        "the other run’s scratch stayed its own: {reply:?}"
    );
}

#[test]
fn deleting_one_run_is_deleting_one_directory() {
    // The property the whole restructure is for.
    let home = scratch("delete-one");
    let mut held = Held::under(&home);
    held.said("01ONE", "mine");
    held.said("01TWO", "theirs");
    assert_eq!(held.scratch.runs().len(), 2);

    let one = SessionId::new("01ONE");
    let two = SessionId::new("01TWO");
    std::fs::remove_dir_all(balthasar_store::run_dir_in(&home, &one)).expect("rm");

    assert_eq!(held.scratch.runs().len(), 1);
    assert!(!held.scratch.path_of(&one, &AgentId::main()).exists());
    assert!(
        held.scratch.path_of(&two, &AgentId::main()).is_file(),
        "the neighbour survived"
    );
}

#[test]
fn a_durable_memory_still_goes_to_the_project() {
    // Nothing about per-run files changes where a fact belongs: a memory with no session is
    // the project's, and every run of it shares that.
    let home = scratch("durable");
    let mut held = Held::under(&home);
    let reply = held.ask("remember", vec![serde_json::json!("we always use make")]);

    assert!(reply.ok, "{reply:?}");
    assert_eq!(held.store.all().expect("all").len(), 1);
    assert!(held.scratch.runs().is_empty(), "no run owns it");
}

/// What a reply says about one field.
fn field<'a>(reply: &'a Reply, name: &str) -> &'a serde_json::Value {
    reply
        .result
        .first()
        .and_then(|v| v.get(name))
        .unwrap_or(&serde_json::Value::Null)
}

#[test]
fn a_peer_may_archive_the_run_it_is_in_but_not_remove_it() {
    // The ceiling that has existed since the socket did, used for the first time. A peer that
    // could remove rows could empty a project one run at a time, and unlike a bad write there
    // is no ladder to catch it afterwards.
    let home = scratch("peer-forget");
    let mut held = Held::under(&home);
    held.said("01RUN", "the deploy target is fly.io");
    let run = SessionId::new("01RUN");

    let refused = held.peer_asks(
        "forget",
        vec![
            serde_json::json!("01RUN"),
            serde_json::json!({ "session": true, "purge": true }),
        ],
    );
    assert!(!refused.ok, "a peer may not remove a run");
    assert!(
        held.scratch.path_of(&run, &AgentId::main()).is_file(),
        "and nothing went"
    );

    let archived = held.peer_asks(
        "forget",
        vec![
            serde_json::json!("01RUN"),
            serde_json::json!({ "session": true }),
        ],
    );
    assert!(archived.ok, "{archived:?}");
    assert_eq!(field(&archived, "archived"), 1, "its own scratch stopped");
    assert!(
        held.scratch.path_of(&run, &AgentId::main()).is_file(),
        "archiving keeps every word of it"
    );
}

#[test]
fn the_owner_removing_a_run_reaches_all_three_of_its_files() {
    let home = scratch("owner-forget");
    let mut held = Held::under(&home);
    held.said("01RUN", "the deploy token is hunter2-nevershare");
    held.said("01SAFE", "something a neighbour knows");
    let run = SessionId::new("01RUN");
    let neighbour = SessionId::new("01SAFE");

    let gone = held.ask(
        "forget",
        vec![
            serde_json::json!("01RUN"),
            serde_json::json!({ "session": true, "purge": true }),
        ],
    );
    assert!(gone.ok, "{gone:?}");
    assert_eq!(field(&gone, "scratch"), true, "its own file went");
    assert!(!held.scratch.path_of(&run, &AgentId::main()).exists());
    assert!(
        held.scratch.path_of(&neighbour, &AgentId::main()).is_file(),
        "and the neighbour kept everything of its own"
    );
}

#[test]
fn forgetting_a_run_nobody_has_heard_of_is_refused() {
    // A typo must not answer as a successful purge of nothing: the caller's next move is to
    // stop looking for the run it meant.
    let home = scratch("no-such-run");
    let mut held = Held::under(&home);
    let reply = held.ask(
        "forget",
        vec![
            serde_json::json!("01NOPE"),
            serde_json::json!({ "session": true, "purge": true }),
        ],
    );
    assert!(!reply.ok);
    assert!(
        reply.error.unwrap_or_default().contains("no run called"),
        "and said why"
    );
}
