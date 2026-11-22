//! F4: a repository built here, and what its commits do to what the project holds.

#![allow(clippy::expect_used, clippy::indexing_slicing)]

use balthasar_distil::read_commits;
use balthasar_lua::Settings;
use balthasar_model::{
    Body, Memory, NoteKind, ScopeId, SessionId, Tier, Timestamp, Witness, WitnessId, WitnessKind,
    scratch::Scratch,
};
use balthasar_store::{Recall, Store, mint};
use std::path::Path;
use std::process::Command;

const NOW: Timestamp = 1_756_000_000;

/// A repository of the test's own, or `None` where there is no git to build it with.
struct Repo {
    dir: Scratch,
}

impl Repo {
    fn new(name: &str) -> Option<Self> {
        let dir = Scratch::new("balthasar-commits", name);
        let repo = Self { dir };
        repo.git(&["init", "-q"]).then_some(repo)
    }

    fn git(&self, args: &[&str]) -> bool {
        Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "maintenance.auto=false",
                "-c",
                "gc.auto=0",
            ])
            .args(args)
            .current_dir(&*self.dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .is_ok_and(|out| out.status.success())
    }

    fn commit(&self, files: &[(&str, &str)], subject: &str) {
        for (path, text) in files {
            let at = self.dir.join(path);
            if let Some(parent) = at.parent() {
                std::fs::create_dir_all(parent).expect("dir");
            }
            std::fs::write(&at, text).expect("write");
        }
        assert!(self.git(&["add", "-A"]), "add");
        assert!(
            self.git(&["commit", "-q", "-m", subject]),
            "commit {subject}"
        );
    }

    fn scope(&self) -> ScopeId {
        ScopeId::new(self.dir.to_string_lossy().into_owned())
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

fn said(store: &mut Store, scope: &ScopeId, text: &str) {
    let memory = Memory::new(
        mint(NOW - 100),
        Tier::Fact,
        scope.clone(),
        Body::note(text, NoteKind::Claim),
        NOW - 100,
    );
    let witness = Witness::new(
        WitnessId::new(format!("w-{}", text.len())),
        WitnessKind::Imperative,
        SessionId::new("s1"),
        scope.clone(),
        NOW - 100,
    );
    store.remember(memory, witness, NOW - 100).expect("said");
}

fn open(store: &Store, query: &str) -> Vec<String> {
    let mut ask = Recall::of(query, NOW);
    ask.floor = 0.0;
    store
        .recall(&ask)
        .expect("recall")
        .into_iter()
        .map(|hit| hit.memory.text())
        .collect()
}

#[test]
fn a_swap_closes_the_memory_and_records_what_the_line_became() {
    let Some(repo) = Repo::new("swap") else {
        return;
    };
    repo.commit(&[("config.toml", "codec = \"gzip\"\n")], "add config");
    let mut store = Store::ephemeral().expect("store");
    let scope = repo.scope();
    said(&mut store, &scope, "we compress the transport with gzip");
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("first read");
    repo.commit(
        &[("config.toml", "codec = \"zstd\"\n")],
        "update dependencies",
    );
    let read = read_commits(&mut store, &scope, &Settings::default(), NOW).expect("read");
    assert_eq!(read.closed, 1, "{read:?}");
    let found = open(&store, "transport");
    assert!(found.iter().any(|t| t.contains("zstd")), "{found:?}");
    assert!(!found.iter().any(|t| t.contains("gzip")), "{found:?}");
}

#[test]
fn a_vanished_value_closes_with_nothing_in_its_place() {
    let Some(repo) = Repo::new("vanished") else {
        return;
    };
    repo.commit(
        &[("deploy.sh", "heroku push\n"), ("README.md", "notes\n")],
        "add deploy",
    );
    let mut store = Store::ephemeral().expect("store");
    let scope = repo.scope();
    said(&mut store, &scope, "we deploy with heroku");
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("first read");
    repo.commit(
        &[("deploy.sh", "echo deploys are manual now\nexit 0\n")],
        "rework deploy",
    );
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("read");
    assert!(
        open(&store, "deploy heroku")
            .iter()
            .all(|t| !t.contains("heroku"))
    );
}

#[test]
fn a_vocabulary_word_closes_nothing() {
    let Some(repo) = Repo::new("vocabulary") else {
        return;
    };
    let files: Vec<(String, &str)> = (0..5)
        .map(|n| (format!("src/f{n}.rs"), "// fast path\n"))
        .collect();
    let named: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), *t)).collect();
    repo.commit(&named, "add files");
    let mut store = Store::ephemeral().expect("store");
    let scope = repo.scope();
    said(&mut store, &scope, "keep the parser on the fast path");
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("first read");
    repo.commit(&[("src/f0.rs", "// slow path\n")], "tweak");
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("read");
    assert!(
        open(&store, "parser")
            .iter()
            .any(|t| t.contains("fast path"))
    );
}

#[test]
fn a_deleted_file_closes_nothing() {
    let Some(repo) = Repo::new("deleted") else {
        return;
    };
    repo.commit(&[("config/codec.toml", "codec = \"gzip\"\n")], "add");
    let mut store = Store::ephemeral().expect("store");
    let scope = repo.scope();
    said(&mut store, &scope, "we compress the transport with gzip");
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("first read");
    assert!(repo.git(&["rm", "-q", "-r", "config"]));
    assert!(repo.git(&["commit", "-q", "-m", "drop config"]));
    let read = read_commits(&mut store, &scope, &Settings::default(), NOW).expect("read");
    assert_eq!(read.closed, 0, "{read:?}");
}

#[test]
fn a_chain_of_changes_closes_the_middle() {
    let Some(repo) = Repo::new("chain") else {
        return;
    };
    repo.commit(&[("config.toml", "codec = \"gzip\"\n")], "add");
    let mut store = Store::ephemeral().expect("store");
    let scope = repo.scope();
    said(&mut store, &scope, "we compress the transport with gzip");
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("first read");
    repo.commit(&[("config.toml", "codec = \"zstd\"\n")], "one");
    repo.commit(&[("config.toml", "codec = \"brotli\"\n")], "two");
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("read");
    let found = open(&store, "transport");
    assert!(found.iter().any(|t| t.contains("brotli")), "{found:?}");
    assert!(!found.iter().any(|t| t.contains("zstd")), "{found:?}");
}

#[test]
fn unrelated_commits_close_nothing() {
    let Some(repo) = Repo::new("unrelated") else {
        return;
    };
    repo.commit(&[("a.txt", "one\n")], "start");
    let mut store = Store::ephemeral().expect("store");
    let scope = repo.scope();
    said(&mut store, &scope, "we compress the transport with gzip");
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("first read");
    for n in 0..30 {
        repo.commit(&[("a.txt", &format!("line {n}\n"))], &format!("chore {n}"));
    }
    let read = read_commits(&mut store, &scope, &Settings::default(), NOW).expect("read");
    assert_eq!(read.closed, 0, "{read:?}");
    assert!(repo.path().exists());
}

#[test]
fn turned_off_reads_nothing() {
    let Some(repo) = Repo::new("off") else { return };
    repo.commit(&[("config.toml", "codec = \"gzip\"\n")], "add");
    let mut store = Store::ephemeral().expect("store");
    let settings = Settings {
        commits: None,
        ..Settings::default()
    };
    let read = read_commits(&mut store, &repo.scope(), &settings, NOW).expect("read");
    assert_eq!(read.commits, 0);
}

#[test]
fn a_rename_closes_nothing() {
    let Some(repo) = Repo::new("rename") else {
        return;
    };
    repo.commit(&[("config.toml", "codec = \"gzip\"\n")], "add");
    let mut store = Store::ephemeral().expect("store");
    let scope = repo.scope();
    said(&mut store, &scope, "we compress the transport with gzip");
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("first read");
    assert!(repo.git(&["mv", "config.toml", "settings.toml"]));
    assert!(repo.git(&["commit", "-q", "-m", "rename"]));
    let read = read_commits(&mut store, &scope, &Settings::default(), NOW).expect("read");
    assert_eq!(read.closed, 0, "{read:?}");
}

#[test]
fn an_older_dated_commit_arriving_later_is_still_read() {
    let Some(repo) = Repo::new("pulled") else {
        return;
    };
    repo.commit(&[("config.toml", "codec = \"gzip\"\n")], "add");
    let mut store = Store::ephemeral().expect("store");
    let scope = repo.scope();
    said(&mut store, &scope, "we compress the transport with gzip");
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("first read");
    std::fs::write(repo.path().join("config.toml"), "codec = \"zstd\"\n").expect("write");
    assert!(repo.git(&["add", "-A"]));
    let dated = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "old",
        ])
        .current_dir(repo.path())
        .env("GIT_COMMITTER_DATE", "2001-01-01T00:00:00")
        .env("GIT_AUTHOR_DATE", "2001-01-01T00:00:00")
        .status()
        .expect("commit");
    assert!(dated.success());
    let read = read_commits(&mut store, &scope, &Settings::default(), NOW).expect("read");
    assert_eq!(read.closed, 1, "{read:?}");
}

#[test]
fn a_revert_closes_what_its_commit_recorded() {
    let Some(repo) = Repo::new("revert") else {
        return;
    };
    repo.commit(&[("config.toml", "codec = \"gzip\"\n")], "add");
    let mut store = Store::ephemeral().expect("store");
    let scope = repo.scope();
    said(&mut store, &scope, "we compress the transport with gzip");
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("first read");
    repo.commit(&[("config.toml", "codec = \"zstd\"\n")], "move to zstd");
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("read");
    assert!(repo.git(&["revert", "--no-edit", "HEAD"]));
    read_commits(&mut store, &scope, &Settings::default(), NOW).expect("read the revert");
    let found = open(&store, "transport");
    assert!(!found.iter().any(|t| t.contains("zstd")), "{found:?}");
}
