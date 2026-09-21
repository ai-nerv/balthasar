//! Where a store lives, and which store a directory belongs to.
//! ```text
//! <project>/.balthasar/<tool>/project.db                   the checkout's memory, for that tool
//! <project>/.balthasar/<tool>/<session>/<agent>/memory.db  that agent's scratch, in that run
//! ~/.local/share/balthasar/<tool>/global.db               yours, everywhere
//! ```
//! In the project, so a renamed checkout keeps its memory; every path component is validated.

use balthasar_model::{AgentId, ScopeId, SessionId};
use std::path::{Path, PathBuf};

/// The directory a project keeps its memory in: hidden, like the rest of a checkout's tool state.
pub const HOME: &str = ".balthasar";

/// Where it was kept before, in plain sight. A store found there is moved to [`HOME`].
const LEGACY_HOME: &str = "balthasar";

/// Kept out of a checkout, so it never shows in `git status`; keeping it is offered commented out.
const IGNORE_BODY: &str = "\
# balthasar's memory for this checkout. None of it is committed unless you mean it to be: to keep
# the project's own memory, replace `*` with the two lines under it.
*
# */*/
# !*/project.db
";

/// What an older balthasar wrote there. Found as it was left, it is replaced; edited, it is not.
const OLD_IGNORE_BODY: &str = "\
# Session stores: churn, and personal to whoever ran them.
*/*/

# The project's own memory. Commit it deliberately or not at all.
# !*/project.db
";

/// Balthasar's own data: `$XDG_DATA_HOME/balthasar`, else `~/.local/share/balthasar`.
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

/// Which tool a memory belongs to. A path component, so validated rather than trusted:
/// `[a-z0-9_-]`, non-empty, no leading dash, never `.` or `..`.
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
    if !at.is_absolute() {
        return None;
    }
    let home = at.join(HOME);
    // A store under the old name is moved, or used where it is when it cannot be; never a
    // checkout that shares the name: in a superproject `balthasar/` is the code itself.
    let legacy = at.join(LEGACY_HOME);
    if !home.exists() && only_a_store(&legacy) && !legacy.join(".git").exists() {
        if std::fs::rename(&legacy, &home).is_err() {
            return Some(legacy);
        }
        let _ = keep_out(&home);
    }
    (is_home(&home) || at.join(".git").exists()).then_some(home)
}

/// Whether `dir` is a store home rather than a directory that shares its name.
fn is_home(dir: &Path) -> bool {
    crate::layout::is_home(dir)
}

/// Whether an old-name directory holds a store and nothing else, never a checkout with a marker.
fn only_a_store(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    is_home(dir)
        && entries.flatten().all(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            entry.path().is_dir()
                || name == crate::layout::MARKER
                || name == ".gitignore"
                || [".db", ".db-wal", ".db-shm"]
                    .iter()
                    .any(|end| name.ends_with(end))
        })
}

/// Create a store home, marking it and keeping it out of the checkout. Overwrites neither a
/// `.gitignore` somebody wrote nor an existing marker; layout moves are `crate::layout`'s to record.
pub fn make_home(home: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(home)?;
    let marker = home.join(crate::layout::MARKER);
    if !marker.exists() {
        std::fs::write(&marker, crate::layout::marker_body())?;
    }
    keep_out(home)
}

/// Write the `.gitignore` a store home keeps: where there is none, or where the one there is what
/// an older balthasar wrote and nobody has touched since.
fn keep_out(home: &Path) -> std::io::Result<()> {
    let ignore = home.join(".gitignore");
    let ours = !ignore.exists()
        || std::fs::read_to_string(&ignore).is_ok_and(|said| said == OLD_IGNORE_BODY);
    if ours {
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
        // Under the old name too: that is a project whose store has not been moved yet.
        if is_home(&at.join(HOME)) || only_a_store(&at.join(LEGACY_HOME)) {
            return Some(at.to_owned());
        }
        // A checkout with no store of its own is its own project: no store above reaches into it.
        if at.join(".git").exists() {
            return None;
        }
        at = at.parent()?;
    }
}

/// Whether `dir` is a directory nobody in particular owns, and so cannot scope a project.
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
mod tests;
