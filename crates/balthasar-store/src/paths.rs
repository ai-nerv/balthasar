//! Where a store lives, and which store a directory belongs to.
//!
//! ```text
//! <project>/balthasar/<tool>/project.db                   the checkout's memory, for that tool
//! <project>/balthasar/<tool>/<session>/<agent>/memory.db  that agent's scratch, in that run
//! ~/.local/share/balthasar/<tool>/global.db               yours, everywhere
//! ```
//!
//! The store lives in the project, so renaming a checkout moves its memory rather than orphaning
//! it. Every name that becomes a path component is validated: tool names arrive from the kernel,
//! session and agent names from a harness.

use balthasar_model::{AgentId, ScopeId, SessionId};
use std::path::{Path, PathBuf};

/// The directory a project keeps its memory in.
pub const HOME: &str = "balthasar";

/// Kept out of a checkout by default. The project's own memory is offered commented out.
const IGNORE_BODY: &str = "\
# Session stores: churn, and personal to whoever ran them.
*/*/

# The project's own memory. Commit it deliberately or not at all.
# !*/project.db
";

/// The directory holding balthasar's own data: `$XDG_DATA_HOME/balthasar`, falling back to
/// `~/.local/share/balthasar`.
#[must_use]
pub fn data_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join("balthasar");
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(home).join(".local/share/balthasar");
    }
    // Per user: a fixed name in the shared temporary directory is the first caller's to own.
    let uid = rustix::process::getuid().as_raw();
    std::env::temp_dir().join(format!("balthasar-{uid}"))
}

/// Which tool a memory belongs to.
///
/// A path component, so it is validated rather than trusted: `[a-z0-9_-]`, non-empty, no leading
/// dash, never `.` or `..`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tool(String);

impl Tool {
    /// What a CLI invocation with nothing configured belongs to.
    pub const DEFAULT: &'static str = "balthasar";

    /// Take `name` as a tool, if it is already usable as one.
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

    /// Take a program name as a tool, making it usable first; `None` when nothing survives.
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
    crate::layout::is_home(dir)
}

/// Create a store home, marking it and keeping it out of the checkout. Overwrites neither an
/// existing `.gitignore` nor an existing marker; layout moves are [`crate::layout`]'s to record.
pub fn make_home(home: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(home)?;
    let marker = home.join(crate::layout::MARKER);
    if !marker.exists() {
        std::fs::write(&marker, crate::layout::marker_body())?;
    }
    let ignore = home.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(&ignore, IGNORE_BODY)?;
    }
    Ok(())
}

/// The file backing `scope` for `tool`.
#[must_use]
pub fn scope_path(scope: &ScopeId, tool: &Tool) -> PathBuf {
    if scope.is_global() {
        return data_dir().join(tool.as_str()).join("global.db");
    }
    home_of(scope).join(tool.as_str()).join("project.db")
}

/// The directory holding one agent's scratch, in one run.
#[must_use]
pub fn session_dir(scope: &ScopeId, tool: &Tool, session: &SessionId, agent: &AgentId) -> PathBuf {
    session_dir_in(&home_of(scope).join(tool.as_str()), session, agent)
}

/// One agent's directory under a tool's home.
#[must_use]
pub fn session_dir_in(home: &Path, session: &SessionId, agent: &AgentId) -> PathBuf {
    run_dir_in(home, session).join(path_stem(agent.as_str()))
}

/// One whole run's directory under a tool's home, every agent of it included.
#[must_use]
pub fn run_dir_in(home: &Path, session: &SessionId) -> PathBuf {
    home.join(path_stem(session.as_str()))
}

/// One agent's scratch.
#[must_use]
pub fn session_path(scope: &ScopeId, tool: &Tool, session: &SessionId, agent: &AgentId) -> PathBuf {
    session_dir(scope, tool, session, agent).join("memory.db")
}

/// Which tools have durable memory in `scope`, in a stable order.
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

/// A harness-supplied name as a directory name.
///
/// `..` must never become a path component. A name that survives intact is used as it is; one
/// that does not carries a digest so two mangled names do not land on one directory.
fn path_stem(name: &str) -> String {
    let safe: String = name
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
    let usable = safe == name && !safe.is_empty() && safe != "." && safe != "..";
    if usable {
        return safe;
    }
    let digest = balthasar_model::content_hash(name);
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
    let digest = balthasar_model::content_hash(scope);
    format!("{leaf}-{}", &digest[..8])
}

/// Which scope `cwd` belongs to.
///
/// In order: a store home somebody already made, then the repository, then the directory itself.
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
///
/// The walk stops at the first shared directory, not at the filesystem root: one leftover
/// `/tmp/balthasar/.store` would otherwise reparent every path under `/tmp` into a single scope.
fn nearest_home(from: &Path) -> Option<PathBuf> {
    home_below(from, is_shared)
}

/// The walk itself, with the ceiling passed in so a test can put one where it can plant a home.
fn home_below(from: &Path, ceiling: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let mut at = from;
    loop {
        if ceiling(at) {
            return None;
        }
        if is_home(&at.join(HOME)) {
            return Some(at.to_owned());
        }
        at = at.parent()?;
    }
}

/// Whether `dir` is a directory nobody in particular owns, and so cannot scope a project.
///
/// The temporary directory, the parent of a home directory, and the filesystem root. Compared by
/// path rather than by mode: `/home` is just as shared and has no sticky world-writable bit.
fn is_shared(dir: &Path) -> bool {
    if dir.parent().is_none() {
        return true;
    }
    if dir == std::env::temp_dir() {
        return true;
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .as_deref()
        .and_then(Path::parent)
        .is_some_and(|above| dir == above)
}

/// Whether a checkout rooted at `dir` would sweep in directories that have nothing to do with it:
/// the shared directories, and a home directory, where a dotfiles repository would otherwise win.
fn too_broad(dir: &Path) -> bool {
    if is_shared(dir) {
        return true;
    }
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .is_some_and(|home| dir == Path::new(&home))
}

/// The repository a directory is in, walking up. Reads `.git` when it is a file, because that is
/// what a worktree has: a pointer at the real repository.
fn git_common_dir(from: &Path) -> Option<PathBuf> {
    let mut at = from;
    loop {
        if too_broad(at) {
            return None;
        }
        let dot = at.join(".git");
        if dot.is_dir() {
            return Some(at.to_owned());
        }
        if dot.is_file() {
            // `gitdir: .../.git/worktrees/name` — the repository is two levels up from the entry.
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
    use balthasar_model::scratch::Scratch;

    /// A scratch directory nothing else is using, named after the test rather than shared.
    fn scratch(name: &str) -> Scratch {
        Scratch::new("balthasar-paths", name)
    }

    #[test]
    fn a_tool_name_that_would_escape_its_directory_is_refused() {
        assert!(Tool::new("..").is_none());
        assert!(Tool::new(".").is_none());
        assert!(Tool::new("a/b").is_none());
        assert!(Tool::new("").is_none());
        assert!(Tool::new("-lead").is_none());
        assert!(Tool::new("Magi").is_none(), "case is not silently folded");
        assert_eq!(Tool::new("harness").expect("valid").as_str(), "harness");
    }

    #[test]
    fn a_program_name_is_made_usable_rather_than_refused() {
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
        for hostile in ["..", ".", "../../etc", "a/b"] {
            let stem = path_stem(hostile);
            assert!(!stem.contains('/'), "{hostile} -> {stem}");
            assert_ne!(stem, "..", "{hostile}");
            assert_ne!(stem, ".", "{hostile}");
        }
    }

    #[test]
    fn an_agent_name_is_held_to_the_same_boundary_as_a_session_name() {
        let home = Path::new("/w/p/balthasar/harness");
        let run = SessionId::new("01K5X8ZQ");
        let hostile = session_dir_in(home, &run, &AgentId::new("../../etc"));

        assert_eq!(
            hostile.parent(),
            Some(run_dir_in(home, &run).as_path()),
            "it stayed inside its run: {hostile:?}"
        );
        let leaf = hostile.file_name().expect("a name").to_string_lossy();
        assert!(leaf != ".." && leaf != "." && !leaf.contains('/'), "{leaf}");
    }

    #[test]
    fn two_mangled_session_names_do_not_share_a_directory() {
        assert_ne!(path_stem("a/b"), path_stem("a:b"));
    }

    #[test]
    fn an_ordinary_session_name_is_left_alone() {
        assert_eq!(path_stem("01K5X8ZQ"), "01K5X8ZQ");
    }

    #[test]
    fn a_directory_named_balthasar_is_not_mistaken_for_a_store() {
        // This checkout is called `balthasar`; without the marker its parent resolves as a root.
        let root = scratch("lookalike");
        std::fs::create_dir_all(root.join(HOME)).expect("mkdir");
        assert!(!is_home(&root.join(HOME)));
        assert!(nearest_home(&root).is_none());

        make_home(&root.join(HOME)).expect("make");
        assert!(is_home(&root.join(HOME)));
    }

    #[test]
    fn the_shared_directories_are_the_ones_nobody_owns() {
        assert!(is_shared(&std::env::temp_dir()));
        assert!(is_shared(std::path::Path::new("/")));
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        if let Some(above) = home.as_deref().and_then(Path::parent) {
            assert!(is_shared(above), "{}", above.display());
        }
    }

    #[test]
    fn a_store_home_in_a_shared_directory_scopes_nothing() {
        // A leftover `.store` in the temporary directory otherwise resolves every path under it
        // to one scope. The ceiling is passed in because no test may write into the real `/tmp`.
        let root = scratch("ceiling");
        let shared = root.join("shared");
        let under = shared.join("project");
        std::fs::create_dir_all(&under).expect("mkdir");
        make_home(&shared.join(HOME)).expect("make");

        let stops = |dir: &Path| dir == shared;
        assert_eq!(
            home_below(&under, stops),
            None,
            "the walk passed the ceiling"
        );
        assert_eq!(home_below(&shared, stops), None);

        assert_eq!(home_below(&under, |_| false), Some(shared.clone()));
    }

    #[test]
    fn a_store_home_scopes_the_subtree_it_sits_in() {
        let root = scratch("subtree");
        let package = root.join("crates/thing");
        std::fs::create_dir_all(root.join(".git")).expect("mkdir");
        std::fs::create_dir_all(package.join("src")).expect("mkdir");
        make_home(&package.join(HOME)).expect("make");

        assert_eq!(
            scope_of(&package.join("src")).as_str(),
            package.to_string_lossy()
        );
        assert_eq!(scope_of(&root).as_str(), root.to_string_lossy());
    }

    #[test]
    fn a_repository_keeps_its_memory_inside_itself() {
        let root = scratch("in-project");
        std::fs::create_dir_all(root.join(".git")).expect("mkdir");
        let scope = scope_of(&root);
        let path = scope_path(&scope, &Tool::default());

        assert!(path.starts_with(&root), "{}", path.display());
        assert_eq!(path, root.join("balthasar/balthasar/project.db"));
    }

    #[test]
    fn renaming_a_project_keeps_its_memory() {
        let root = scratch("rename");
        let before = root.join("before");
        std::fs::create_dir_all(before.join(".git")).expect("mkdir");
        make_home(&before.join(HOME)).expect("make");
        let was = scope_path(&scope_of(&before), &Tool::default());
        let relative = was.strip_prefix(&before).expect("under the project");

        let after = root.join("after");
        std::fs::rename(&before, &after).expect("mv");
        let now = scope_path(&scope_of(&after), &Tool::default());

        assert_eq!(
            now,
            after.join(relative),
            "the store moved with the project"
        );
    }

    #[test]
    fn everything_under_a_repository_shares_its_scope() {
        let root = scratch("in-repo");
        let deep = root.join("crates/thing/src");
        std::fs::create_dir_all(root.join(".git")).expect("mkdir");
        std::fs::create_dir_all(&deep).expect("mkdir");

        assert_eq!(scope_of(&deep), scope_of(&root));
        assert_eq!(scope_of(&deep).as_str(), root.to_string_lossy());
    }

    #[test]
    fn every_worktree_of_a_repository_shares_one_scope() {
        // A worktree's `.git` is a file pointing at the real one.
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
    }

    #[test]
    fn a_directory_in_no_repository_is_its_own_scope() {
        // Deliberately a path that does not exist: a real temporary directory may sit under
        // somebody else's checkout.
        let nowhere = Path::new("/balthasar-no-such-root-9f3a/deep/inside");
        assert_eq!(scope_of(nowhere).as_str(), nowhere.to_string_lossy());
    }

    #[test]
    fn a_scope_with_no_project_keeps_its_memory_in_the_data_directory() {
        let nowhere = ScopeId::new("/balthasar-no-such-root-9f3a/deep");
        let path = scope_path(&nowhere, &Tool::default());
        assert!(path.starts_with(data_dir()), "{}", path.display());
    }

    #[test]
    fn the_global_store_is_per_tool_and_not_per_project() {
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
        assert_eq!(
            one.parent().and_then(Path::parent),
            two.parent().and_then(Path::parent)
        );
    }

    #[test]
    fn every_agent_of_a_run_keeps_its_scratch_under_that_run() {
        let scope = ScopeId::new("/balthasar-no-such-root-9f3a/p");
        let tool = Tool::default();
        let run = SessionId::new("01K5X8ZQ");
        let one = session_dir(&scope, &tool, &run, &AgentId::main());
        let two = session_dir(&scope, &tool, &run, &AgentId::new("reviewer"));

        assert_ne!(one, two, "two agents, two directories");
        assert_eq!(one.parent(), two.parent(), "and one run above them");
        assert_eq!(
            session_path(&scope, &tool, &run, &AgentId::main()),
            one.join("memory.db")
        );
    }

    #[test]
    fn a_run_directory_sits_beside_the_project_store_it_promotes_into() {
        let scope = ScopeId::new("/balthasar-no-such-root-9f3a/p");
        let tool = Tool::default();
        let run = SessionId::new("01K5X8ZQ");
        assert_eq!(
            session_dir(&scope, &tool, &run, &AgentId::main())
                .parent()
                .and_then(Path::parent),
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
        assert_eq!(
            names,
            vec!["harness", "oslo"],
            "a directory with no store is not a tool"
        );
    }

    #[test]
    fn a_store_home_keeps_sessions_out_of_the_checkout() {
        let root = scratch("ignore");
        let home = root.join(HOME);
        make_home(&home).expect("make");
        let body = std::fs::read_to_string(home.join(".gitignore")).expect("read");
        assert!(body.contains("*/*/"), "sessions are ignored");
        assert!(
            body.contains("# !*/project.db"),
            "committing is offered, not chosen"
        );

        std::fs::write(home.join(".gitignore"), "mine\n").expect("write");
        make_home(&home).expect("again");
        assert_eq!(
            std::fs::read_to_string(home.join(".gitignore")).expect("read"),
            "mine\n"
        );
    }

    #[test]
    fn a_repository_in_a_shared_directory_scopes_nothing() {
        // A `.git` at the top of the temporary directory, or in a home directory, made every
        // directory below it resolve to that one root.
        let shared = std::env::temp_dir();
        assert!(too_broad(&shared), "{}", shared.display());

        let under = balthasar_model::scratch::Scratch::new("balthasar-git-ceiling", "probe");
        assert_eq!(git_common_dir(&under), None);
        assert_eq!(scope_of(&under).as_str(), under.to_string_lossy());
    }

    #[test]
    fn a_home_directory_is_too_broad_to_be_a_project() {
        // A repository at `$HOME` is a dotfiles repository, not the project an unrelated
        // subdirectory belongs to; `is_shared` still lets a deliberate store here count.
        let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) else {
            return;
        };
        let home = std::path::Path::new(&home);
        assert!(too_broad(home), "{}", home.display());
        assert!(!is_shared(home), "an explicit store here would still count");
    }

    #[test]
    fn a_real_checkout_is_still_found() {
        let root = scratch("checkout");
        std::fs::create_dir_all(root.join(".git")).expect("mkdir");
        let deep = root.join("crates/thing/src");
        std::fs::create_dir_all(&deep).expect("mkdir");
        assert_eq!(git_common_dir(&deep).as_deref(), Some(&*root));
    }
}
