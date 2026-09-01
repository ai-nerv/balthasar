//! Where a store lives, and which store a directory belongs to.
//!
//! Two stores are consulted on every recall, project first: what is true everywhere, and what
//! is true here. A project fact shadows a global one in the same slot. A harness that keeps a
//! per-directory preference cache has the lower half of this and not the upper: "I always use
//! make" belongs to the person, not to whichever project they last opened.

use aeon_model::ScopeId;
use std::path::{Path, PathBuf};

/// The directory holding every scope's store.
///
/// `$XDG_DATA_HOME/aeon`, falling back to `~/.local/share/aeon`. Beside the data rather than in
/// the configuration: a store is not something anybody hand-edits, and losing it should cost
/// memories rather than settings.
#[must_use]
pub fn data_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("aeon");
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(home).join(".local/share/aeon");
    }
    std::env::temp_dir().join("aeon")
}

/// The file backing `scope`.
///
/// Named after the scope rather than hashed, when the name is safe to use as one. A directory
/// listing that reads `work-thing.db` is worth more than one that reads `3f9a1c….db`, and the
/// hash is only reached for when a name would not survive being a filename.
#[must_use]
pub fn scope_path(scope: &ScopeId) -> PathBuf {
    data_dir().join(format!("{}.db", file_stem(scope.as_str())))
}

/// A scope name as a filename: the last component, plus a short digest so two projects with the
/// same basename do not share a store.
fn file_stem(scope: &str) -> String {
    if scope == "global" {
        return "global".to_owned();
    }
    let leaf: String = scope
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("scope")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let digest = aeon_model::content_hash(scope);
    format!("{leaf}-{}", &digest[..8])
}

/// Which scope `cwd` belongs to.
///
/// The repository root, so five worktrees of one project share one memory rather than starting
/// each other's amnesia. Falling back to the directory itself when there is no repository.
///
/// Lua will decide this from M2 — a monorepo is one scope, `~/scratch` is probably none — and
/// this stays as the answer when nothing is registered.
#[must_use]
pub fn scope_of(cwd: &Path) -> ScopeId {
    git_common_dir(cwd).map_or_else(
        || ScopeId::new(cwd.to_string_lossy().into_owned()),
        |root| ScopeId::new(root.to_string_lossy().into_owned()),
    )
}

/// The repository a directory is in, walking up.
///
/// Reads `.git` when it is a file, because that is what a worktree has: a pointer at the real
/// repository. Following it is the difference between one memory per project and one per
/// checkout.
fn git_common_dir(from: &Path) -> Option<PathBuf> {
    let mut at = from;
    loop {
        let dot = at.join(".git");
        if dot.is_dir() {
            return Some(at.to_owned());
        }
        if dot.is_file() {
            // `gitdir: /path/to/repo/.git/worktrees/name` — the repository is two levels up
            // from the worktree entry.
            let pointer = std::fs::read_to_string(&dot).ok()?;
            let target = pointer.trim().strip_prefix("gitdir:")?.trim();
            let target = PathBuf::from(target);
            return target
                .ancestors()
                .find(|a| a.file_name().is_some_and(|n| n == ".git"))
                .and_then(|git| git.parent())
                .map(Path::to_owned)
                .or(Some(at.to_owned()));
        }
        at = at.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_global_store_is_named_plainly() {
        assert_eq!(file_stem("global"), "global");
    }

    #[test]
    fn a_project_store_is_readable_and_still_unique() {
        // A directory listing that reads `aeon-1a2b3c4d.db` is worth more than one that reads
        // a bare hash, and the suffix keeps two projects named `web` apart.
        let one = file_stem("/home/you/work/web");
        let two = file_stem("/home/you/play/web");
        assert!(one.starts_with("web-"), "{one}");
        assert_ne!(one, two);
    }

    #[test]
    fn a_scope_name_that_would_not_survive_a_filename_is_made_to() {
        let stem = file_stem("/home/you/work/a b:c");
        assert!(!stem.contains(' ') && !stem.contains(':'), "{stem}");
    }

    /// A scratch directory nothing else is using.
    ///
    /// Named after the test rather than shared, because two tests tidying up one tree race
    /// each other and the loser fails somewhere unrelated.
    fn scratch(name: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("aeon-paths-{name}"));
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).expect("mkdir");
        at
    }

    #[test]
    fn everything_under_a_repository_shares_its_scope() {
        // The contract: a scope is a project, not a directory. Working in `src/` and working
        // in the root are the same project and must not be two memories.
        let root = scratch("in-repo");
        let deep = root.join("crates/thing/src");
        std::fs::create_dir_all(root.join(".git")).expect("mkdir");
        std::fs::create_dir_all(&deep).expect("mkdir");

        assert_eq!(scope_of(&deep), scope_of(&root));
        assert_eq!(scope_of(&deep).as_str(), root.to_string_lossy());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn every_worktree_of_a_repository_shares_one_scope() {
        // Without this, `git worktree add` starts a project with amnesia about the project it
        // is a copy of. A worktree's `.git` is a file pointing at the real one.
        let root = scratch("worktree");
        let work = root.join("checkout");
        let tree = root.join("tree");
        std::fs::create_dir_all(work.join(".git/worktrees/tree")).expect("mkdir");
        std::fs::create_dir_all(&tree).expect("mkdir");
        std::fs::write(
            tree.join(".git"),
            format!("gitdir: {}/.git/worktrees/tree\n", work.display()),
        )
        .expect("write");

        assert_eq!(scope_of(&tree), scope_of(&work));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_directory_in_no_repository_is_its_own_scope() {
        // Deliberately a path that does not exist: any real temporary directory may sit under
        // somebody else's checkout, and a test that walks to the root is a test about the
        // machine it runs on rather than about this function.
        let nowhere = Path::new("/aeon-no-such-root-9f3a/deep/inside");
        assert_eq!(scope_of(nowhere).as_str(), nowhere.to_string_lossy());
    }
}
