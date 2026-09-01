//! Reaching a model, or doing without one.
//!
//! One trait, several backends, and a list tried in order. The right answer genuinely differs
//! by where memo is standing: running under a harness, the cheapest correct answer is to ask
//! the harness, which already has the credentials; on a laptop with no daemon, the shell-out
//! uses whatever is already authenticated; on a box with neither, an endpoint works.
//!
//! **Nothing here is required.** With no backend reachable, extraction is extractive and
//! consolidation is clustering. Worse, and never absent: an agent whose memory hard-fails
//! without an API key is worse off than one with no memory at all. Every memory records which
//! backend produced it, so `memo why` can say "this came out of the rules, not a model".

use std::io::Write;
use std::process::{Command, Stdio};

/// What went wrong reaching a model.
#[derive(Debug, thiserror::Error)]
pub enum DistilFailure {
    /// The backend is not there.
    #[error("{0} is not reachable")]
    Unreachable(String),
    /// It answered badly, or not at all.
    #[error("{0}: {1}")]
    Refused(String, String),
}

/// How much a single call may spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// How long to wait.
    pub timeout_ms: u64,
    /// How much answer to accept.
    pub max_bytes: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            timeout_ms: 20_000,
            max_bytes: 64 * 1024,
        }
    }
}

/// Something that can answer a prompt.
pub trait Distil {
    /// What to record in `witness.note`, so a distilled memory can say what produced it.
    fn name(&self) -> String;

    /// Whether this can answer right now.
    ///
    /// Checked before use and cheap, so a list falls through without paying a timeout for a
    /// backend that was never going to answer.
    fn reachable(&self) -> bool;

    /// One prompt, one answer.
    ///
    /// No streaming: nothing here is shown to a person as it arrives, and a distillation that
    /// half-arrived is not half-useful.
    fn complete(&self, prompt: &str, budget: Budget) -> Result<String, DistilFailure>;
}

/// Spawn something that is already authenticated.
///
/// Prompt on standard input, answer on standard output, non-zero exit is a failure. No HTTP
/// client, no credential handling in memo at all, and it inherits whatever the person has
/// already set up — `llm`, `ollama run`, a harness's own one-shot mode, or a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spawned {
    /// The command and its arguments.
    pub argv: Vec<String>,
}

impl Spawned {
    /// A backend that runs `argv`.
    #[must_use]
    pub fn new(argv: Vec<String>) -> Self {
        Self { argv }
    }
}

impl Distil for Spawned {
    fn name(&self) -> String {
        format!("command:{}", self.argv.first().map_or("?", String::as_str))
    }

    fn reachable(&self) -> bool {
        let Some(program) = self.argv.first() else {
            return false;
        };
        // Asked of `$PATH` rather than by running it. A reachability check that executed the
        // thing would spend a model call finding out whether it could make one.
        which(program).is_some()
    }

    fn complete(&self, prompt: &str, budget: Budget) -> Result<String, DistilFailure> {
        let Some((program, args)) = self.argv.split_first() else {
            return Err(DistilFailure::Unreachable("an empty command".to_owned()));
        };

        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|why| DistilFailure::Unreachable(format!("{program}: {why}")))?;

        if let Some(mut stdin) = child.stdin.take() {
            // A model that will not read the whole prompt is a model that will answer about
            // half of it, so a broken pipe here is a failure rather than something to ignore.
            stdin
                .write_all(prompt.as_bytes())
                .map_err(|why| DistilFailure::Refused(self.name(), why.to_string()))?;
        }

        let out = child
            .wait_with_output()
            .map_err(|why| DistilFailure::Refused(self.name(), why.to_string()))?;

        if !out.status.success() {
            let said = String::from_utf8_lossy(&out.stderr);
            let said = said.trim();
            return Err(DistilFailure::Refused(
                self.name(),
                if said.is_empty() {
                    format!("exited {}", out.status)
                } else {
                    said.chars().take(200).collect()
                },
            ));
        }

        let mut answer = String::from_utf8_lossy(&out.stdout).into_owned();
        answer.truncate(budget.max_bytes);
        let answer = answer.trim().to_owned();
        if answer.is_empty() {
            return Err(DistilFailure::Refused(
                self.name(),
                "said nothing".to_owned(),
            ));
        }
        Ok(answer)
    }
}

/// Where a program is, if it is anywhere on `$PATH`.
fn which(program: &str) -> Option<std::path::PathBuf> {
    if program.contains('/') {
        let path = std::path::PathBuf::from(program);
        return path.is_file().then_some(path);
    }
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| std::path::Path::new(dir).join(program))
        .find(|candidate| candidate.is_file())
}

/// Try each backend in turn, and answer with what the first reachable one said.
///
/// A backend that fails is skipped and the next tried. Falling all the way through is not an
/// error: it means the extractive path runs, which is a supported state and the one every
/// milestone before this was built on.
#[must_use]
pub fn first_answer(
    backends: &[Box<dyn Distil>],
    prompt: &str,
    budget: Budget,
) -> Option<(String, String)> {
    for backend in backends {
        if !backend.reachable() {
            continue;
        }
        if let Ok(answer) = backend.complete(prompt, budget) {
            return Some((answer, backend.name()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawned(argv: &[&str]) -> Spawned {
        Spawned::new(argv.iter().map(|s| (*s).to_owned()).collect())
    }

    #[test]
    fn something_that_is_not_installed_is_not_reachable() {
        // Checked against `$PATH` rather than by running it: a reachability test that executed
        // the thing would spend a model call finding out whether it could make one.
        assert!(!spawned(&["no-such-program-9f3a"]).reachable());
    }

    #[test]
    fn something_that_is_installed_is() {
        assert!(spawned(&["sh"]).reachable());
    }

    #[test]
    fn an_empty_command_is_never_reachable() {
        assert!(!Spawned::new(Vec::new()).reachable());
    }

    #[test]
    fn the_prompt_goes_in_and_the_answer_comes_out() {
        let backend = spawned(&["cat"]);
        let answer = backend
            .complete("summarise this", Budget::default())
            .expect("cat answers");
        assert_eq!(answer, "summarise this");
    }

    #[test]
    fn a_command_that_fails_says_what_it_said() {
        let backend = spawned(&["sh", "-c", "echo 'no api key' >&2; exit 1"]);
        let why = backend
            .complete("anything", Budget::default())
            .expect_err("it failed");
        assert!(why.to_string().contains("no api key"), "{why}");
    }

    #[test]
    fn a_command_that_says_nothing_is_a_failure() {
        // An empty answer read as a distillation would produce a memory with no content and a
        // witness saying a model made it.
        let backend = spawned(&["true"]);
        assert!(backend.complete("anything", Budget::default()).is_err());
    }

    #[test]
    fn an_answer_is_bounded() {
        let backend = spawned(&["sh", "-c", "yes x | head -c 100000"]);
        let budget = Budget {
            max_bytes: 1024,
            ..Budget::default()
        };
        let answer = backend.complete("anything", budget).expect("answers");
        assert!(answer.len() <= 1024, "{}", answer.len());
    }

    #[test]
    fn a_backend_names_itself_for_the_witness() {
        // Distilled output that came out of the rules must not be indistinguishable from
        // output that came out of a model.
        assert_eq!(spawned(&["llm", "-m", "x"]).name(), "command:llm");
    }

    #[test]
    fn a_list_falls_through_to_the_first_that_answers() {
        let backends: Vec<Box<dyn Distil>> = vec![
            Box::new(spawned(&["no-such-program-9f3a"])),
            Box::new(spawned(&["sh", "-c", "exit 1"])),
            Box::new(spawned(&["cat"])),
        ];
        let (answer, from) =
            first_answer(&backends, "a prompt", Budget::default()).expect("something answered");
        assert_eq!(answer, "a prompt");
        assert_eq!(from, "command:cat");
    }

    #[test]
    fn a_list_with_nothing_reachable_answers_nothing() {
        // Not an error. It means the extractive path runs, which every milestone before this
        // was built on.
        let backends: Vec<Box<dyn Distil>> = vec![Box::new(spawned(&["no-such-program-9f3a"]))];
        assert!(first_answer(&backends, "a prompt", Budget::default()).is_none());
    }
}

/// Read a configuration's `memo.distiller` into backends, in the order it listed them.
///
/// One spec or a list; `kind` is always explicit, never inferred from which keys are present.
/// A config whose meaning changes because a field was added is a config nobody can read.
///
/// Backends this build does not carry are reported rather than silently dropped — a setting
/// that is quietly ignored is worse than one that is refused, because nobody ever finds out.
#[must_use]
pub fn backends(said: Option<&serde_json::Value>) -> (Vec<Box<dyn Distil>>, Vec<String>) {
    let mut out: Vec<Box<dyn Distil>> = Vec::new();
    let mut unavailable = Vec::new();

    let specs: Vec<&serde_json::Value> = match said {
        None => Vec::new(),
        Some(serde_json::Value::Array(items)) => items.iter().collect(),
        Some(one) => vec![one],
    };

    for spec in specs {
        match spec.get("kind").and_then(serde_json::Value::as_str) {
            Some("command") => {
                let argv: Vec<String> = spec
                    .get("argv")
                    .and_then(serde_json::Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                if argv.is_empty() {
                    unavailable.push("a command backend with no argv".to_owned());
                } else {
                    out.push(Box::new(Spawned::new(argv)));
                }
            }
            Some(kind @ ("endpoint" | "peer")) => {
                unavailable.push(format!(
                    "'{kind}' is declared but this build carries only 'command'"
                ));
            }
            Some(other) => unavailable.push(format!("'{other}' is not a backend memo knows")),
            None => unavailable.push("a distiller with no kind".to_owned()),
        }
    }
    (out, unavailable)
}

#[cfg(test)]
mod reading {
    use super::*;

    #[test]
    fn a_list_is_read_in_the_order_it_was_written() {
        let said = serde_json::json!([
            { "kind": "command", "argv": ["first"] },
            { "kind": "command", "argv": ["second"] },
        ]);
        let (backends, _) = backends(Some(&said));
        assert_eq!(backends.len(), 2);
        assert_eq!(backends[0].name(), "command:first");
    }

    #[test]
    fn one_spec_is_a_list_of_one() {
        let said = serde_json::json!({ "kind": "command", "argv": ["llm"] });
        let (backends, _) = backends(Some(&said));
        assert_eq!(backends.len(), 1);
    }

    #[test]
    fn saying_nothing_configures_nothing() {
        let (backends, unavailable) = backends(None);
        assert!(backends.is_empty() && unavailable.is_empty());
    }

    #[test]
    fn a_backend_this_build_does_not_carry_is_reported_rather_than_dropped() {
        // A setting that is quietly ignored is worse than one that is refused: nobody ever
        // finds out why their distiller never ran.
        let said = serde_json::json!({ "kind": "endpoint", "base_url": "https://…" });
        let (backends, unavailable) = backends(Some(&said));
        assert!(backends.is_empty());
        assert_eq!(unavailable.len(), 1);
        assert!(unavailable[0].contains("endpoint"), "{}", unavailable[0]);
    }

    #[test]
    fn a_kind_is_never_inferred_from_which_keys_are_present() {
        // A config whose meaning changes because a field was added is a config nobody can
        // read. It says what it is or it is refused.
        let said = serde_json::json!({ "argv": ["llm"] });
        let (backends, unavailable) = backends(Some(&said));
        assert!(backends.is_empty());
        assert!(unavailable[0].contains("no kind"), "{}", unavailable[0]);
    }
}
