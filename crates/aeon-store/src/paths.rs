//! Where a store lives, and which store a directory belongs to.
//!
//! Three levels, one tool namespace at every level. A session's scratch is its own file so that
//! deleting one run is deleting one directory; a project's durable memory sits beside it; what
//! is true everywhere lives in the data directory, because "I always use make" belongs to the
//! person rather than to whichever project they last opened.
//!
//! ```text
//! <project>/aeon/<tool>/project.db          the checkout's memory, for that tool
//! <project>/aeon/<tool>/<session>/memory.db that run's scratch
//! <project>/aeon/<tool>/<session>/transcript.db
//! ~/.local/share/aeon/<tool>/global.db      yours, everywhere
//! ```
//!
//! Two rules hold here and are load-bearing elsewhere. **The store lives in the project**, so
//! renaming a checkout moves its memory rather than orphaning it. And **every name that becomes
//! a path component is validated**, because tool names arrive from the kernel and session names
//! arrive from a harness: neither is this crate's to trust.

use aeon_model::{ScopeId, SessionId};
use std::path::{Path, PathBuf};

/// The directory a project keeps its memory in.
pub const HOME: &str = "aeon";

/// What marks one as aeon's rather than a directory that happens to be called `aeon`.
///
/// Without it, any directory containing a subdirectory named `aeon` would be mistaken for a
/// project root — including the parent of aeon's own checkout, which is how this was found.
const MARKER: &str = ".store";

/// What goes in the marker: enough to tell a later layout that this is an earlier one.
const MARKER_BODY: &str = "aeon store layout 1\n";

/// Kept out of a checkout by default. Sessions are churn and belong to whoever ran them; a
/// project's own memory is a decision rather than a default, so it is offered commented out.
const IGNORE_BODY: &str = "\
# Session stores: churn, and personal to whoever ran them.
*/*/

# The project's own memory. Commit it deliberately or not at all.
# !*/project.db
";

/// The directory holding aeon's own data.
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

/// Which tool a memory belongs to.
///
/// A path component, so it is validated rather than trusted: `[a-z0-9_-]`, non-empty, no
/// leading dash, never `.` or `..`. The value normally arrives from `SO_PEERCRED` — the kernel
/// naming the program on the other end of the socket — which makes it unspoofable but not
/// automatically safe to join onto a path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tool(String);

impl Tool {
    /// What a CLI invocation with nothing configured belongs to.
    pub const DEFAULT: &'static str = "aeon";

    /// Take `name` as a tool, if it is already usable as one.
    ///
    /// Strict on purpose: this is the path used when somebody typed `--tool`, and silently
    /// rewriting what they typed would put their memories somewhere they did not ask for.
    #[must_use]
    pub fn new(name: &str) -> Option<Self> {
        let usable = !name.is_empty()
            && name.len() <= 32
            && !name.starts_with('-')
            && name != "."
            && name != ".."
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
        usable.then(|| Self(name.to_owned()))
    }

    /// Take a program name as a tool, making it usable first.
    ///
    /// For the peer end of a socket, where the name is whatever the executable is called.
    /// Returns `None` when nothing survives — a caller that cannot be named should be refused
    /// rather than filed under a guess.
    #[must_use]
    pub fn from_program(name: &str) -> Option<Self> {
        let slug: String = name
            .trim()
            .to_ascii_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let slug = slug.trim_matches('-');
        let slug: String = slug.chars().take(32).collect();
        Self::new(slug.trim_end_matches('-'))
    }

    /// The tool as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Tool {
    fn default() -> Self {
        Self(Self::DEFAULT.to_owned())
    }
}

impl std::fmt::Display for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The directory holding every store for `scope`.
///
/// A project's own `aeon/` when the scope is a checkout, so that memory moves with the code.
/// The data directory otherwise: `~/scratch` and `/tmp` should not sprout store directories,
/// and the global scope belongs to nobody's project.
#[must_use]
pub fn home_of(scope: &ScopeId) -> PathBuf {
    if let Some(inside) = project_home(scope) {
        return inside;
    }
    if scope.is_global() {
        return data_dir();
    }
    data_dir().join("scopes").join(file_stem(scope.as_str()))
}

/// The directory inside the project that holds its stores, if the scope has a project at all.
///
/// `None` for the global scope and for a directory that is neither a checkout nor somewhere
/// somebody ran `aeon init`. Callers use it to decide whether there is a home to create: the
/// data directory needs no marker and no ignore file.
#[must_use]
pub fn project_home(scope: &ScopeId) -> Option<PathBuf> {
    if scope.is_global() {
        return None;
    }
    let at = Path::new(scope.as_str());
    let has = at.is_absolute() && (is_home(&at.join(HOME)) || at.join(".git").exists());
    has.then(|| at.join(HOME))
}

/// Whether `dir` is a store home rather than a directory that shares its name.
fn is_home(dir: &Path) -> bool {
    dir.join(MARKER).is_file()
}

/// Create a store home, marking it and keeping it out of the checkout.
///
/// Idempotent, and never overwrites an existing `.gitignore`: whether to commit `project.db` is
/// a decision, and having made it once it should not be unmade by the next write.
pub fn make_home(home: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(home)?;
    let marker = home.join(MARKER);
    if !marker.exists() {
        std::fs::write(&marker, MARKER_BODY)?;
    }
    let ignore = home.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, IGNORE_BODY)?;
    }
    Ok(())
}

/// The file backing `scope` for `tool`.
///
/// The global scope's store is per-tool but not per-project, which is the whole distinction:
/// what is true everywhere for a harness is not what is true everywhere for a shell.
#[must_use]
pub fn scope_path(scope: &ScopeId, tool: &Tool) -> PathBuf {
    if scope.is_global() {
        return data_dir().join(tool.as_str()).join("global.db");
    }
    home_of(scope).join(tool.as_str()).join("project.db")
}

/// The directory holding one run's scratch and scrollback.
#[must_use]
pub fn session_dir(scope: &ScopeId, tool: &Tool, session: &SessionId) -> PathBuf {
    session_dir_in(&home_of(scope).join(tool.as_str()), session)
}

/// One run.s directory under a tool.s home.
///
/// The half a scratchpad needs: it holds one tool.s home and opens runs beneath it, without
/// having to carry a scope around to recompute what it already knows.
#[must_use]
pub fn session_dir_in(home: &Path, session: &SessionId) -> PathBuf {
    home.join(session_stem(session.as_str()))
}

/// One run's scratch.
#[must_use]
pub fn session_path(scope: &ScopeId, tool: &Tool, session: &SessionId) -> PathBuf {
    session_dir(scope, tool, session).join("memory.db")
}

/// One run's scrollback.
///
/// Beside that run's scratch and never inside it: a transcript is orders of magnitude larger
/// than the memories distilled from it, and sharing a file would make every recall walk past it.
#[must_use]
pub fn session_transcript_path(scope: &ScopeId, tool: &Tool, session: &SessionId) -> PathBuf {
    session_dir(scope, tool, session).join("transcript.db")
}

/// Which tools have durable memory in `scope`, in a stable order.
///
/// What a recall with no `--tool` reads across. Missing directories are not an error: a scope
/// nothing has written to yet has no tools, and that is an empty answer rather than a failure.
#[must_use]
pub fn tools_in(scope: &ScopeId) -> Vec<Tool> {
    let home = home_of(scope);
    let Ok(entries) = std::fs::read_dir(&home) else {
        return Vec::new();
    };
    let mut found: Vec<Tool> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let tool = Tool::new(&name.to_string_lossy())?;
            home.join(tool.as_str())
                .join("project.db")
                .is_file()
                .then_some(tool)
        })
        .collect();
    found.sort();
    found
}

/// A session name as a directory name.
///
/// Session identities come from a harness, so this is a boundary rather than a nicety: `..` is
/// a name a harness could reasonably produce and must never become a path component. A name
/// that survives intact is used as it is, and one that does not carries a digest so two mangled
/// names do not land on one directory.
fn session_stem(session: &str) -> String {
    let safe: String = session
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    let usable = safe == session && !safe.is_empty() && safe != "." && safe != "..";
    if usable {
        return safe;
    }
    let digest = aeon_model::content_hash(session);
    let head: String = safe.chars().filter(|c| *c != '-').take(16).collect();
    if head.is_empty() {
        format!("session-{}", &digest[..12])
    } else {
        format!("{head}-{}", &digest[..8])
    }
}

/// A scope name as a filename, for the scopes that have no project to live in.
fn file_stem(scope: &str) -> String {
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
/// In order: a store home somebody already made, then the repository, then the directory
/// itself. The first rule is what lets `aeon init` in a subdirectory scope memory to a subtree
/// — which is how a monorepo gets per-package memory without this crate knowing what a monorepo
/// is. The second is why five worktrees of one project share one memory rather than starting
/// each other's amnesia.
#[must_use]
pub fn scope_of(cwd: &Path) -> ScopeId {
    if let Some(home) = nearest_home(cwd) {
        return ScopeId::new(home.to_string_lossy().into_owned());
    }
    git_common_dir(cwd).map_or_else(
        || ScopeId::new(cwd.to_string_lossy().into_owned()),
        |root| ScopeId::new(root.to_string_lossy().into_owned()),
    )
}

/// The closest ancestor holding a store home, if any.
fn nearest_home(from: &Path) -> Option<PathBuf> {
    let mut at = from;
    loop {
        if is_home(&at.join(HOME)) {
            return Some(at.to_owned());
        }
        at = at.parent()?;
    }
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
    fn a_tool_name_that_would_escape_its_directory_is_refused() {
        // The reason this type exists. A tool name becomes a path component, and `..` arriving
        // from a peer must not be able to write outside the store home.
        assert!(Tool::new("..").is_none());
        assert!(Tool::new(".").is_none());
        assert!(Tool::new("a/b").is_none());
        assert!(Tool::new("").is_none());
        assert!(Tool::new("-lead").is_none());
        assert!(Tool::new("Axon").is_none(), "case is not silently folded");
        assert_eq!(Tool::new("harness").expect("valid").as_str(), "harness");
    }

    #[test]
    fn a_program_name_is_made_usable_rather_than_refused() {
        // `--tool` is strict because somebody typed it; a program name is not typed by anybody
        // and is worth salvaging.
        assert_eq!(
            Tool::from_program("My Harness").expect("slug").as_str(),
            "my-harness"
        );
        assert_eq!(
            Tool::from_program("/usr/bin/thing").expect("slug").as_str(),
            "usr-bin-thing"
        );
        assert!(Tool::from_program("///").is_none(), "nothing survives");
        assert!(Tool::from_program("").is_none());
    }

    #[test]
    fn a_session_name_cannot_climb_out_of_the_store() {
        // A session identity is whatever a harness calls its run. `..` is a name a harness
        // could plausibly produce and must never become a path component.
        for hostile in ["..", ".", "../../etc", "a/b"] {
            let stem = session_stem(hostile);
            assert!(!stem.contains('/'), "{hostile} -> {stem}");
            assert_ne!(stem, "..", "{hostile}");
            assert_ne!(stem, ".", "{hostile}");
        }
    }

    #[test]
    fn two_mangled_session_names_do_not_share_a_directory() {
        // Without the digest, every unusual name would collapse onto one directory and two
        // runs would overwrite each other's scrollback.
        assert_ne!(session_stem("a/b"), session_stem("a:b"));
    }

    #[test]
    fn an_ordinary_session_name_is_left_alone() {
        assert_eq!(session_stem("01K5X8ZQ"), "01K5X8ZQ");
    }

    #[test]
    fn a_directory_named_aeon_is_not_mistaken_for_a_store() {
        // Found the hard way: aeon's own checkout is called `aeon`, so without the marker the
        // parent of this repository would resolve as a project root.
        let root = scratch("lookalike");
        std::fs::create_dir_all(root.join(HOME)).expect("mkdir");
        assert!(!is_home(&root.join(HOME)));
        assert!(nearest_home(&root).is_none());

        make_home(&root.join(HOME)).expect("make");
        assert!(is_home(&root.join(HOME)));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_store_home_scopes_the_subtree_it_sits_in() {
        // What makes per-package memory in a monorepo possible without this crate knowing what
        // a monorepo is: the home is nearer than the repository root, so it wins.
        let root = scratch("subtree");
        let package = root.join("crates/thing");
        std::fs::create_dir_all(root.join(".git")).expect("mkdir");
        std::fs::create_dir_all(package.join("src")).expect("mkdir");
        make_home(&package.join(HOME)).expect("make");

        assert_eq!(scope_of(&package.join("src")).as_str(), package.to_string_lossy());
        assert_eq!(scope_of(&root).as_str(), root.to_string_lossy());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_repository_keeps_its_memory_inside_itself() {
        // The point of the restructure: the store is in the project, so it is visible, movable
        // and deletable with the checkout.
        let root = scratch("in-project");
        std::fs::create_dir_all(root.join(".git")).expect("mkdir");
        let scope = scope_of(&root);
        let path = scope_path(&scope, &Tool::default());

        assert!(path.starts_with(&root), "{}", path.display());
        assert_eq!(path, root.join("aeon/aeon/project.db"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn renaming_a_project_keeps_its_memory() {
        // The bug this replaces: scope used to be hashed into a filename in the data
        // directory, so `mv` produced a different hash, an empty store, and no error.
        let root = scratch("rename");
        let before = root.join("before");
        std::fs::create_dir_all(before.join(".git")).expect("mkdir");
        make_home(&before.join(HOME)).expect("make");
        let was = scope_path(&scope_of(&before), &Tool::default());
        let relative = was.strip_prefix(&before).expect("under the project");

        let after = root.join("after");
        std::fs::rename(&before, &after).expect("mv");
        let now = scope_path(&scope_of(&after), &Tool::default());

        assert_eq!(now, after.join(relative), "the store moved with the project");
        let _ = std::fs::remove_dir_all(&root);
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

    #[test]
    fn a_scope_with_no_project_keeps_its_memory_in_the_data_directory() {
        // `/tmp` and `~/scratch` should not sprout store directories.
        let nowhere = ScopeId::new("/aeon-no-such-root-9f3a/deep");
        let path = scope_path(&nowhere, &Tool::default());
        assert!(path.starts_with(data_dir()), "{}", path.display());
    }

    #[test]
    fn the_global_store_is_per_tool_and_not_per_project() {
        // What is true everywhere for a harness is not what is true everywhere for a shell,
        // but neither belongs to a checkout.
        let tool = Tool::new("oslo").expect("valid");
        let path = scope_path(&ScopeId::global(), &tool);
        assert_eq!(path, data_dir().join("oslo/global.db"));
    }

    #[test]
    fn two_tools_in_one_project_do_not_share_a_store() {
        let root = scratch("two-tools");
        std::fs::create_dir_all(root.join(".git")).expect("mkdir");
        let scope = scope_of(&root);
        let one = scope_path(&scope, &Tool::new("harness").expect("valid"));
        let two = scope_path(&scope, &Tool::new("oslo").expect("valid"));

        assert_ne!(one, two);
        assert_eq!(one.parent().and_then(Path::parent), two.parent().and_then(Path::parent));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_run_keeps_its_scratch_and_its_scrollback_in_one_directory() {
        // So that deleting one run is deleting one directory.
        let scope = ScopeId::new("/aeon-no-such-root-9f3a/p");
        let tool = Tool::default();
        let run = SessionId::new("01K5X8ZQ");
        let dir = session_dir(&scope, &tool, &run);

        assert_eq!(session_path(&scope, &tool, &run), dir.join("memory.db"));
        assert_eq!(
            session_transcript_path(&scope, &tool, &run),
            dir.join("transcript.db")
        );
    }

    #[test]
    fn a_session_directory_sits_beside_the_project_store_it_promotes_into() {
        let scope = ScopeId::new("/aeon-no-such-root-9f3a/p");
        let tool = Tool::default();
        let run = SessionId::new("01K5X8ZQ");
        assert_eq!(
            session_dir(&scope, &tool, &run).parent(),
            scope_path(&scope, &tool).parent()
        );
    }

    #[test]
    fn listing_tools_finds_only_the_ones_with_memory() {
        let root = scratch("tools-in");
        std::fs::create_dir_all(root.join(".git")).expect("mkdir");
        let home = root.join(HOME);
        make_home(&home).expect("make");
        std::fs::create_dir_all(home.join("harness")).expect("mkdir");
        std::fs::create_dir_all(home.join("oslo")).expect("mkdir");
        std::fs::create_dir_all(home.join("empty")).expect("mkdir");
        std::fs::write(home.join("harness/project.db"), "").expect("write");
        std::fs::write(home.join("oslo/project.db"), "").expect("write");

        let found = tools_in(&scope_of(&root));
        let names: Vec<&str> = found.iter().map(Tool::as_str).collect();
        assert_eq!(names, vec!["harness", "oslo"], "a directory with no store is not a tool");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_store_home_keeps_sessions_out_of_the_checkout() {
        let root = scratch("ignore");
        let home = root.join(HOME);
        make_home(&home).expect("make");
        let body = std::fs::read_to_string(home.join(".gitignore")).expect("read");
        assert!(body.contains("*/*/"), "sessions are ignored");
        assert!(body.contains("# !*/project.db"), "committing is offered, not chosen");

        std::fs::write(home.join(".gitignore"), "mine\n").expect("write");
        make_home(&home).expect("again");
        assert_eq!(std::fs::read_to_string(home.join(".gitignore")).expect("read"), "mine\n");
        let _ = std::fs::remove_dir_all(&root);
    }
}
