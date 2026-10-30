//! A decision that reached the code: the same change measured with balthasar reading the
//! repository's commits and without, against a repository built here.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_model::scratch::Scratch;
use balthasar_testkit::{Arm, Measured, Scenario, Score, measure_between};
use std::process::Command;

const START: balthasar_model::Timestamp = 1_756_000_000;
const SESSIONS: usize = 10;

fn git(dir: &Scratch, args: &[&str]) -> bool {
    Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(&**dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .is_ok_and(|out| out.status.success())
}

fn commit(dir: &Scratch, script: &str, subject: &str) {
    std::fs::write(dir.join("deploy.sh"), script).expect("write");
    assert!(git(dir, &["add", "-A"]) && git(dir, &["commit", "-q", "-m", subject]));
}

/// One run of the scenario in a fresh repository, the deploy target changing in a commit
/// halfway, read by balthasar after each session when `reading`.
fn ran(name: &str, reading: bool) -> Option<(Score, Measured)> {
    let dir = Scratch::new("balthasar-coded", name);
    if !git(&dir, &["init", "-q"]) {
        return None;
    }
    commit(&dir, "#!/bin/sh\nmake deploy-fly\n", "deploy script");
    let scenario = Scenario::decision_in_code(&dir.to_string_lossy(), SESSIONS, START);
    let settings = balthasar_lua::Settings::default();
    let mut between = |store: &mut balthasar_store::Store,
                       scope: &balthasar_model::ScopeId,
                       n: usize,
                       at: balthasar_model::Timestamp| {
        if n + 1 == SESSIONS / 2 {
            commit(&dir, "#!/bin/sh\nmake deploy-render\n", "move the deploy");
        }
        if reading {
            balthasar_distil::read_commits(store, scope, &settings, at)?;
        }
        Ok(())
    };
    Some(measure_between(&scenario, Arm::Memory, &mut between).expect("a run"))
}

#[test]
fn reading_the_commits_closes_the_stated_decision_the_code_moved_off() {
    let (Some((blind, unread)), Some((seen, read))) = (ran("blind", false), ran("read", true))
    else {
        return;
    };
    assert!(seen.recalls > blind.recalls, "{seen:?} against {blind:?}");
    assert!(
        read.asserted_stale < unread.asserted_stale,
        "{} against {}",
        read.asserted_stale,
        unread.asserted_stale
    );
    assert_eq!((read.stale_served, unread.stale_served), (0, 0));
}
