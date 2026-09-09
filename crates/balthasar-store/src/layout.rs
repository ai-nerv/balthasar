//! Which layout a store home is written in, and moving an older one forward.
//!
//! Layout 2 puts an agent between a run and its scratch — `<tool>/<session>/<agent>/memory.db`
//! rather than `<tool>/<session>/memory.db`. Nothing else moves: `project.db` and the scrollback
//! stay shared. Migration is lazy and structural: it happens on the first [`Scratchpad::at`]
//! over a tree, it moves rather than copies, and it is a no-op the second time.
//!
//! [`Scratchpad::at`]: crate::Scratchpad::at

use std::path::{Path, PathBuf};

/// What marks a directory as balthasar's rather than one that happens to be called `balthasar`.
pub(crate) const MARKER: &str = ".store";

/// What every marker starts with, and what a layout number follows.
const HEADING: &str = "balthasar store layout ";

/// The layout this build writes.
pub(crate) const LAYOUT: u32 = 2;

/// The files SQLite keeps beside a database, which move with it. A WAL left behind when its
/// database moves is a checkpoint nothing will ever apply, and the last writes are simply gone.
const SIBLINGS: [&str; 2] = ["-wal", "-shm"];

/// What a marker should say.
#[must_use]
pub(crate) fn marker_body() -> String {
    format!("{HEADING}{LAYOUT}\n")
}

/// Which layout `home` is written in, if it is a store home at all. `None` for a directory with
/// no marker and for a marker this cannot read.
#[must_use]
pub(crate) fn layout_of(home: &Path) -> Option<u32> {
    let body = std::fs::read_to_string(home.join(MARKER)).ok()?;
    body.trim().strip_prefix(HEADING)?.trim().parse().ok()
}

/// Whether `dir` is a store home rather than a directory that shares its name. Any layout counts,
/// including a newer one: not recognising it would make [`crate::scope_of`] keep walking up.
#[must_use]
pub(crate) fn is_home(dir: &Path) -> bool {
    layout_of(dir).is_some()
}

/// Bring a tool's runs forward to the current layout, if they are not already. Answers how many
/// runs moved. The marker is consulted only as a cost decision: a home already stamped with this
/// layout skips the walk, and a tree with no marker at all is still walked, because that is what
/// `--store` and the data directory look like.
///
/// # Errors
/// When a directory cannot be read or a file cannot be moved.
pub(crate) fn bring_forward(tool_home: &Path) -> std::io::Result<usize> {
    let stamped = tool_home.parent().and_then(layout_of);
    if stamped.is_some_and(|found| found >= LAYOUT) {
        return Ok(0);
    }
    let Some((home, _)) = tool_home.parent().zip(stamped) else {
        // Nowhere to keep a stamp, so this tool's runs are all there is to bring forward.
        return move_runs(tool_home);
    };

    // Every tool under the home, not only the one being opened. The marker is one stamp for the
    // whole tree, so stamping it after migrating a single tool would hide a sibling tool's runs.
    let mut moved = 0;
    for tool in std::fs::read_dir(home)?.flatten() {
        moved += move_runs(&tool.path())?;
    }
    // Written explicitly, because `make_home` refuses to touch a marker that already exists: a
    // function that creates a home must not decide an existing tree's layout.
    std::fs::write(home.join(MARKER), marker_body())?;
    Ok(moved)
}

/// Move every run's scratch down into the agent that wrote it, `main` by default. A run whose
/// agent directory already holds scratch is left exactly as it is: two files claiming to be one
/// agent's scratch is a question this cannot answer.
fn move_runs(tool_home: &Path) -> std::io::Result<usize> {
    let Ok(entries) = std::fs::read_dir(tool_home) else {
        // A home that does not exist yet is the ordinary state before the first write.
        return Ok(0);
    };

    let mut moved = 0;
    for run in entries.flatten() {
        let dir = run.path();
        let was = dir.join("memory.db");
        if !was.is_file() {
            continue;
        }
        let now = dir.join(balthasar_model::AgentId::MAIN);
        if now.join("memory.db").exists() {
            continue;
        }
        std::fs::create_dir_all(&now)?;
        // A database and its write-ahead log move together or not at all: leaving the WAL at the
        // old path reads as a run already brought forward, and the writes it holds are lost.
        let mut undo = Vec::new();
        let mut stopped = None;
        for suffix in std::iter::once("").chain(SIBLINGS) {
            let from = beside(&was, suffix);
            if !from.exists() {
                continue;
            }
            let to = beside(&now.join("memory.db"), suffix);
            match std::fs::rename(&from, &to) {
                Ok(()) => undo.push((to, from)),
                Err(why) => {
                    stopped = Some(why);
                    break;
                }
            }
        }
        if let Some(why) = stopped {
            for (to, from) in undo.into_iter().rev() {
                // Nothing further to try if the undo fails, and `why` is the error to report.
                let _ = std::fs::rename(&to, &from);
            }
            return Err(why);
        }
        moved += 1;
    }
    Ok(moved)
}

/// One of SQLite's siblings of `path`, or `path` itself when the suffix is empty.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    if suffix.is_empty() {
        return path.to_owned();
    }
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use balthasar_model::scratch::Scratch;

    fn scratch(name: &str) -> Scratch {
        Scratch::new("balthasar-layout", name)
    }

    /// A tool home holding `runs` sessions in the old shape, each with a WAL beside it.
    fn old_tree(at: &Path, runs: &[&str]) {
        std::fs::create_dir_all(at).expect("mkdir");
        std::fs::write(
            at.parent().expect("a home").join(MARKER),
            "balthasar store layout 1\n",
        )
        .expect("marker");
        for run in runs {
            let dir = at.join(run);
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(dir.join("memory.db"), format!("scratch of {run}")).expect("db");
            std::fs::write(dir.join("memory.db-wal"), "uncheckpointed").expect("wal");
        }
    }

    #[test]
    fn a_marker_nothing_can_parse_is_not_a_layout() {
        let root = scratch("unparsed");
        std::fs::create_dir_all(&*root).expect("mkdir");
        std::fs::write(root.join(MARKER), "hello\n").expect("write");
        assert_eq!(layout_of(&root), None);
        assert!(!is_home(&root));

        std::fs::write(root.join(MARKER), marker_body()).expect("write");
        assert_eq!(layout_of(&root), Some(LAYOUT));
        assert!(is_home(&root));
    }

    #[test]
    fn a_home_written_by_a_newer_balthasar_is_still_a_home() {
        let root = scratch("newer");
        std::fs::create_dir_all(&*root).expect("mkdir");
        std::fs::write(root.join(MARKER), format!("{HEADING}{}\n", LAYOUT + 1)).expect("write");
        assert!(is_home(&root));
        assert_eq!(bring_forward(&root.join("harness")).expect("forward"), 0);
    }

    #[test]
    fn an_old_tree_keeps_every_run_and_its_uncheckpointed_writes() {
        let root = scratch("migrate");
        let home = root.join("balthasar/harness");
        old_tree(&home, &["01ONE", "01TWO", "01THREE"]);

        assert_eq!(bring_forward(&home).expect("forward"), 3);

        for run in ["01ONE", "01TWO", "01THREE"] {
            let now = home.join(run).join("main");
            assert_eq!(
                std::fs::read_to_string(now.join("memory.db")).expect("db"),
                format!("scratch of {run}"),
                "the scratch moved rather than being made anew"
            );
            assert!(now.join("memory.db-wal").is_file(), "and its WAL with it");
            assert!(
                !home.join(run).join("memory.db").exists(),
                "nothing was left in the old place"
            );
        }
        assert_eq!(
            layout_of(&root.join("balthasar")),
            Some(LAYOUT),
            "the marker says which layout the tree is in now"
        );
    }

    #[test]
    fn migrating_a_tree_twice_moves_nothing_the_second_time() {
        let root = scratch("twice");
        let home = root.join("balthasar/harness");
        old_tree(&home, &["01ONE"]);

        assert_eq!(bring_forward(&home).expect("first"), 1);
        assert_eq!(bring_forward(&home).expect("again"), 0);
        assert_eq!(
            std::fs::read_to_string(home.join("01ONE/main/memory.db")).expect("db"),
            "scratch of 01ONE"
        );
    }

    #[test]
    fn opening_one_tool_brings_every_tool_in_the_home_forward() {
        let root = scratch("siblings");
        let home = root.join("balthasar");
        old_tree(&home.join("harness"), &["01ONE"]);
        old_tree(&home.join("oslo"), &["01TWO"]);

        assert_eq!(bring_forward(&home.join("harness")).expect("forward"), 2);
        assert!(home.join("harness/01ONE/main/memory.db").is_file());
        assert!(home.join("oslo/01TWO/main/memory.db").is_file());
    }

    #[test]
    fn a_run_that_already_has_agent_scratch_is_left_alone() {
        let root = scratch("collision");
        let home = root.join("balthasar/harness");
        old_tree(&home, &["01ONE"]);
        std::fs::create_dir_all(home.join("01ONE/main")).expect("mkdir");
        std::fs::write(home.join("01ONE/main/memory.db"), "already here").expect("db");

        assert_eq!(bring_forward(&home).expect("forward"), 0);
        assert_eq!(
            std::fs::read_to_string(home.join("01ONE/main/memory.db")).expect("db"),
            "already here"
        );
        assert!(
            home.join("01ONE/memory.db").is_file(),
            "and the older file is still there to be looked at"
        );
    }

    #[test]
    fn a_tree_with_no_marker_is_still_brought_forward() {
        let root = scratch("markerless");
        let home = root.join("runs");
        std::fs::create_dir_all(home.join("01ONE")).expect("mkdir");
        std::fs::write(home.join("01ONE/memory.db"), "scratch").expect("db");

        assert_eq!(bring_forward(&home).expect("forward"), 1);
        assert!(home.join("01ONE/main/memory.db").is_file());
    }

    #[test]
    fn a_run_whose_wal_cannot_move_is_left_whole() {
        // A directory standing where the WAL must land is what makes the second rename fail.
        let root = scratch("partial");
        let home = root.join("balthasar/harness");
        old_tree(&home, &["01ONE"]);
        std::fs::create_dir_all(home.join("01ONE/main/memory.db-wal/held")).expect("mkdir");

        assert!(bring_forward(&home).is_err(), "the WAL could not be moved");
        assert!(
            home.join("01ONE/memory.db").is_file(),
            "the database went back to where its WAL still is"
        );
        assert!(home.join("01ONE/memory.db-wal").is_file());
        assert!(
            !home.join("01ONE/main/memory.db").exists(),
            "and nothing was left at the new path for the next process to trust"
        );
    }
}
