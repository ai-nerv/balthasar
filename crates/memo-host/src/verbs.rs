//! The exposed surface, spelled out.
//!
//! Written as a list rather than discovered at runtime, so reading this file tells you what a
//! peer can ask of your memory. A surface you have to run something to learn is one nobody
//! audits.

/// Every verb memo answers.
///
/// `verbs` and `status` ship from the first version. One sibling having them and another not is
/// how a family stops being one, and it cannot be retrofitted quietly.
pub const SURFACE: &[Verb] = &[
    Verb {
        name: "verbs",
        writes: false,
        about: "every name this memo will answer",
    },
    Verb {
        name: "status",
        writes: false,
        about: "what this memory holds, and whether a model or an embedder is reachable",
    },
    Verb {
        name: "recall",
        writes: false,
        about: "search: (query, opts) -> [memory]",
    },
    Verb {
        name: "context",
        writes: false,
        about: "what a model would be told: (opts) -> { sections, text, tokens }",
    },
    Verb {
        name: "scroll",
        writes: false,
        about: "part of a scrollback, within a budget: (session, opts) -> { turns, next }",
    },
    Verb {
        name: "model",
        writes: true,
        about: "what a run talks to: (session, opts) -> { model, context }",
    },
    Verb {
        name: "used",
        writes: true,
        about: "a caller acted on what it was given: (injection, opts) -> { action }",
    },
    Verb {
        name: "outcome",
        writes: true,
        about: "how an action went: (action, opts) -> { outcome, kind }",
    },
    Verb {
        name: "trace",
        writes: false,
        about: "a recall, what it considered, and what followed: (recall) -> { .. }",
    },
    Verb {
        name: "utility",
        writes: false,
        about: "attributed outcomes for a memory, beside how often it was retrieved: (id)",
    },
    Verb {
        name: "why",
        writes: false,
        about: "the evidence for a memory: (id) -> { confidence, witnesses }",
    },
    Verb {
        name: "sessions",
        writes: false,
        about: "the runs this project has had",
    },
    Verb {
        name: "observe",
        writes: true,
        about: "stream a turn as it settles: (session, turn) -> ok",
    },
    Verb {
        name: "plan",
        writes: false,
        about: "what to send: (session, window) -> { keep, mask, drop, summarise, why }",
    },
    Verb {
        name: "amend",
        writes: true,
        about: "revise a turn where it stands: (session, turn) -> ok",
    },
    Verb {
        name: "replay",
        writes: false,
        about: "everything a run said, in order: (session) -> [turn]",
    },
    Verb {
        name: "resume",
        writes: false,
        about: "where a restarting harness left off: (session) -> { next, turns }",
    },
    Verb {
        name: "remember",
        writes: true,
        about: "propose something worth keeping: (text, opts) -> landing",
    },
    Verb {
        name: "forget",
        writes: true,
        about: "stop asserting something, or a run: (id, opts) -> ok",
    },
];

/// One verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verb {
    /// What a caller says.
    pub name: &'static str,
    /// Whether answering it changes anything.
    pub writes: bool,
    /// What it does, in one line, for `verbs`.
    pub about: &'static str,
}

/// Whether a name is one memo answers.
#[must_use]
pub fn known(name: &str) -> Option<&'static Verb> {
    SURFACE.iter().find(|verb| verb.name == name)
}

/// Names that will never be verbs, and why.
///
/// Checked by a test rather than merely intended. A socket that runs commands is remote code
/// execution wearing a friendlier name, and the fact that a person would have typed the same
/// thing does not make it the same thing when a process types it.
pub const NEVER: &[&str] = &["prompt", "run", "eval", "exec", "shell", "purge", "sql"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_surface_ships_the_two_verbs_a_family_needs() {
        assert!(known("verbs").is_some());
        assert!(known("status").is_some());
    }

    #[test]
    fn nothing_shaped_like_running_a_command_is_on_the_surface() {
        for name in NEVER {
            assert!(
                known(name).is_none(),
                "'{name}' must never be a verb — a socket that runs commands is remote code \
                 execution wearing a friendlier name"
            );
        }
    }

    #[test]
    fn every_verb_is_named_once() {
        let mut names: Vec<&str> = SURFACE.iter().map(|v| v.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before);
    }

    #[test]
    fn every_verb_says_what_it_does() {
        for verb in SURFACE {
            assert!(!verb.about.is_empty(), "{} says nothing", verb.name);
        }
    }

    #[test]
    fn most_of_the_surface_reads_rather_than_writes() {
        let writes = SURFACE.iter().filter(|v| v.writes).count();
        assert!(
            writes * 2 < SURFACE.len(),
            "a memory layer's socket should mostly answer questions"
        );
    }
}
