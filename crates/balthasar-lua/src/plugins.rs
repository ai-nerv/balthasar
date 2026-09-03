//! Where configuration comes from, and in what order.
//!
//! neovim's model, unchanged: a runtimepath of roots, `plugin/` run at startup, `lua/` required
//! on demand, `after/` last. Twenty years of real plugins have been written against it and most
//! people arriving already know it. Deviating buys nothing and costs everyone the transfer.

use std::path::{Path, PathBuf};

/// Whether a file may declare as well as choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// The owner's own configuration. May declare anything.
    Owner,
    /// A file that arrived with the project. May set a floor or add a section; may not name a
    /// command to run, an endpoint to send text to, or how somebody's transcripts are read.
    Project,
}

impl Trust {
    /// Whether a file at this level may declare.
    #[must_use]
    pub fn may_declare(self) -> bool {
        matches!(self, Self::Owner)
    }
}

/// The roots a configuration is read from.
#[derive(Debug, Clone, Default)]
pub struct Roots {
    /// `$XDG_CONFIG_HOME/balthasar`, the owner's own.
    pub config: Option<PathBuf>,
    /// `$XDG_DATA_HOME/balthasar/site`, where installed packages live.
    pub site: Option<PathBuf>,
    /// The working directory, whose `.balthasar.lua` may choose but not declare.
    pub project: Option<PathBuf>,
    /// What a coordinator said, read last of all.
    ///
    /// A root like the others rather than a path this module goes and looks up, so
    /// [`runtimepath`] is a function of what it is handed. Reading it inside meant the answer
    /// depended on whether *this machine* happened to have a coordinator running — which made
    /// every test of the order pass alone and fail beside a real session.
    pub given: Option<PathBuf>,
}

impl Roots {
    /// The usual roots for a machine.
    #[must_use]
    pub fn discovered(cwd: &Path) -> Self {
        Self {
            config: Some(PathBuf::from(crate::helpers::config_home()).join("balthasar")),
            site: Some(PathBuf::from(crate::helpers::data_home()).join("balthasar/site")),
            project: Some(cwd.to_owned()),
            given: Some(crate::setup::given()),
        }
    }
}

/// Every file to read, in the order to read it, with whether each may declare.
///
/// ```text
///   <config>/init.lua                 the entry point; what it does not name does not run
///   <config>/plugin/*.lua             alphabetical, each in its own pcall
///   <site>/pack/*/start/*/plugin/*.lua
///   <config>/after/plugin/*.lua       the last word
///   ./.balthasar.lua                       may choose, may not declare
/// ```
#[must_use]
pub fn runtimepath(roots: &Roots) -> Vec<(PathBuf, bool)> {
    let mut out = Vec::new();

    if let Some(config) = &roots.config {
        let init = config.join("init.lua");
        if init.is_file() {
            out.push((init, true));
        }
        out.extend(
            lua_files(&config.join("plugin"))
                .into_iter()
                .map(|p| (p, true)),
        );
    }

    if let Some(site) = &roots.site {
        for package in packages(&site.join("pack")) {
            out.extend(
                lua_files(&package.join("plugin"))
                    .into_iter()
                    .map(|p| (p, true)),
            );
        }
    }

    // `after/` runs last, which is what lets it win against keyed registrars: registering the
    // same identity twice replaces, so whoever registers last decides.
    if let Some(config) = &roots.config {
        out.extend(
            lua_files(&config.join("after/plugin"))
                .into_iter()
                .map(|p| (p, true)),
        );
    }

    if let Some(project) = &roots.project {
        let local = project.join(".balthasar.lua");
        if local.is_file() {
            out.push((local, false));
        }
    }

    // What a coordinator said, last of all and trusted like the owner's own: whoever starts this
    // process is deciding what it should be, and a file on disk that quietly won would be the
    // disagreement the arrangement exists to end. Absent is the ordinary case — a balthasar
    // nobody is coordinating reads its own files exactly as before.
    if let Some(given) = &roots.given
        && given.is_file()
    {
        out.push((given.clone(), true));
    }
    out
}

/// Whether a project directory is one the owner vouched for.
///
/// `balthasar.trusted = { "/home/you/work" }` in the owner's own configuration. A directory under a
/// vouched-for one counts, so vouching for a workspace does not mean listing every repository
/// in it.
#[must_use]
pub fn vouched_for(trusted: &[String], project: &Path) -> bool {
    trusted
        .iter()
        .any(|root| !root.is_empty() && project.starts_with(root))
}

/// Every `.lua` directly in a directory, alphabetically.
///
/// Alphabetical rather than by whatever the filesystem answers: a load order that changes
/// between machines is a configuration that behaves differently on each of them.
fn lua_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "lua"))
        .collect();
    found.sort();
    found
}

/// Every installed package under `pack/*/start/*`.
fn packages(pack: &Path) -> Vec<PathBuf> {
    let Ok(groups) = std::fs::read_dir(pack) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for group in groups.flatten() {
        let Ok(entries) = std::fs::read_dir(group.path().join("start")) else {
            continue;
        };
        found.extend(entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("balthasar-rtp-{name}"));
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).expect("mkdir");
        at
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, "-- nothing\n").expect("write");
    }

    #[test]
    fn init_runs_before_any_plugin() {
        let root = scratch("order");
        let config = root.join("config");
        touch(&config.join("init.lua"));
        touch(&config.join("plugin/zzz.lua"));

        let files = runtimepath(&Roots {
            config: Some(config),
            site: None,
            project: None,
            given: None,
        });
        assert!(files[0].0.ends_with("init.lua"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn after_gets_the_last_word() {
        // Registrars are keyed, so whoever registers last decides. That is what makes
        // `after/` mean anything at all.
        let root = scratch("after");
        let config = root.join("config");
        touch(&config.join("plugin/aaa.lua"));
        touch(&config.join("after/plugin/aaa.lua"));

        let files = runtimepath(&Roots {
            config: Some(config),
            site: None,
            project: None,
            given: None,
        });
        let last = &files.last().expect("something").0;
        assert!(
            last.to_string_lossy().contains("after"),
            "{}",
            last.display()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plugins_load_alphabetically_rather_than_however_the_disk_answers() {
        let root = scratch("alpha");
        let config = root.join("config");
        for name in ["ccc.lua", "aaa.lua", "bbb.lua"] {
            touch(&config.join("plugin").join(name));
        }
        let files = runtimepath(&Roots {
            config: Some(config),
            site: None,
            project: None,
            given: None,
        });
        let names: Vec<String> = files
            .iter()
            .map(|(p, _)| p.file_name().unwrap_or_default().to_string_lossy().into())
            .collect();
        assert_eq!(names, ["aaa.lua", "bbb.lua", "ccc.lua"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn what_a_coordinator_said_comes_after_everything_on_disk() {
        // The point of the arrangement: whoever started this process is deciding what it should
        // be, and registrars are keyed, so the last word has to be theirs. A file on disk that
        // quietly won would be the disagreement this exists to end.
        let root = scratch("coordinated");
        let config = root.join("config");
        touch(&config.join("init.lua"));
        touch(&config.join("after/plugin/zzz.lua"));
        let given = root.join("given.lua");
        touch(&given);
        touch(&root.join(".balthasar.lua"));

        let files = runtimepath(&Roots {
            config: Some(config),
            site: None,
            project: Some(root.clone()),
            given: Some(given.clone()),
        });
        assert_eq!(files.last().expect("something").0, given, "{files:?}");
        assert!(files.last().expect("something").1, "and trusted to declare");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_coordinator_that_said_nothing_adds_nothing() {
        // The ordinary case: a balthasar nobody is coordinating reads its own files exactly as
        // it did before any of this existed.
        let root = scratch("uncoordinated");
        let config = root.join("config");
        touch(&config.join("init.lua"));

        let files = runtimepath(&Roots {
            config: Some(config),
            site: None,
            project: None,
            given: Some(root.join("nothing-was-written-here.lua")),
        });
        assert_eq!(files.len(), 1, "{files:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_project_file_comes_last_and_is_not_trusted() {
        let root = scratch("project");
        touch(&root.join(".balthasar.lua"));
        let files = runtimepath(&Roots {
            config: None,
            site: None,
            project: Some(root.clone()),
            given: None,
        });
        assert_eq!(files.len(), 1);
        assert!(
            !files[0].1,
            "a file that arrived with git clone may not declare"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn vouching_for_a_workspace_covers_what_is_in_it() {
        let trusted = vec!["/home/you/work".to_owned()];
        assert!(vouched_for(&trusted, Path::new("/home/you/work/thing")));
        assert!(!vouched_for(
            &trusted,
            Path::new("/home/you/downloads/thing")
        ));
    }

    #[test]
    fn an_empty_trust_entry_vouches_for_nothing() {
        // `starts_with("")` is true for every path. An empty string in the list would quietly
        // trust the whole filesystem.
        assert!(!vouched_for(&[String::new()], Path::new("/anywhere")));
    }

    #[test]
    fn a_missing_root_is_not_an_error() {
        let files = runtimepath(&Roots {
            config: Some(PathBuf::from("/no/such/place")),
            site: Some(PathBuf::from("/nor/this")),
            project: None,
            given: None,
        });
        assert!(files.is_empty());
    }
}
