//! Runs are listed newest first, and a clock that reads in whole seconds does not get a vote on
//! which of two runs is newer.

use balthasar_model::SessionId;
use balthasar_model::scratch::Scratch;
use balthasar_store::Transcript;

#[test]
fn two_runs_opened_in_the_same_second_list_the_later_one_first() {
    let dir = Scratch::new("balthasar-ordering", "tie");
    let mut held = Transcript::open(&dir.join("transcript.db")).expect("open");
    for name in ["first", "second", "third"] {
        held.open_run(&SessionId::new(name), "/w", "/w", "harness", 100)
            .expect("open a run");
    }
    let listed: Vec<String> = held
        .runs(10)
        .expect("runs")
        .iter()
        .map(|run| run.session.as_str().to_owned())
        .collect();
    assert_eq!(listed, ["third", "second", "first"]);
}
