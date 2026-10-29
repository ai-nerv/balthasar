//! COMMIT: the repository as a witness.
//!
//! A one-line change that drops a value a memory names closes the memory and records what the
//! line became. A value a commit removed that no tracked file holds any more closes it with
//! nothing in its place. Deleted and renamed files say nothing; a word in more than a few files
//! is vocabulary, not a value, unless it is spelled like a name.

use crate::DistilError;
use balthasar_lua::{Commits, Settings};
use balthasar_model::{
    Body, Channel, Domain, Memory, MemoryId, NoteKind, ScopeId, SessionId, Temporal, Tier,
    Timestamp, Witness, WitnessId, WitnessKind,
};
use balthasar_store::{Recall, Store, mint};
use std::collections::HashMap;
use std::time::Duration;

/// The stamp source a read commit is recorded under.
const SOURCE: &str = "commit";
const VERSION: i64 = 1;

/// Commits and diff lines read on the first pass, and on every pass after it.
const FIRST: (usize, usize) = (500, 40_000);
const AFTER: (usize, usize) = (50, 4_000);

/// The most memories looked at, tree checks one pass makes, and files a value may live in.
const CANDIDATES: usize = 400;
const TREE_CHECKS: usize = 32;
const VALUE_FILES: usize = 3;

/// How long one command may run, and how much of its output is read.
const DEADLINE: Duration = Duration::from_secs(30);
const OUTPUT: usize = 16 * 1024 * 1024;

/// Words a value follows when it is not spelled like a name: "use make", "deploy to fly".
const SLOTS: &[&str] = &["use", "uses", "using", "with", "to", "via", "on"];

/// What one pass read and changed.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Read {
    pub commits: usize,
    pub closed: usize,
    pub recorded: usize,
    /// Whether commits were left unread past this pass's bound.
    pub backlog: bool,
}

/// One commit, oldest first once parsed.
#[derive(Debug, Default)]
struct Commit {
    hash: String,
    at: Timestamp,
    subject: String,
    /// One-for-one changed lines: file, removed, added.
    swaps: Vec<(String, String, String)>,
    removed: Vec<String>,
    added: Vec<String>,
}

/// A memory a commit may close, and the values it names.
struct Held {
    id: MemoryId,
    text: String,
    values: Vec<String>,
}

/// Read what the repository at `scope` has committed since the last pass.
pub fn read_commits(
    store: &mut Store,
    scope: &ScopeId,
    settings: &Settings,
    now: Timestamp,
) -> Result<Read, DistilError> {
    let mut read = Read::default();
    let Some(commands) = settings.commits.as_ref() else {
        return Ok(read);
    };
    let repo = scope.as_str();
    if !std::path::Path::new(repo).join(".git").exists() {
        return Ok(read);
    }
    let (count, lines) = if store.has_read_any(SOURCE)? {
        AFTER
    } else {
        FIRST
    };
    let Some(log) = run(&commands.log, repo, &count.to_string(), "") else {
        return Ok(read);
    };
    let (mut commits, truncated) = parse(&log, lines);
    read.backlog = truncated;
    commits.retain(|c| !store.already_read(SOURCE, &c.hash, VERSION).unwrap_or(true));
    commits.reverse();

    let mut held = candidates(store)?;
    let mut probe = Probe::new(commands, repo);
    for commit in &commits {
        apply(store, scope, commit, &mut held, &mut probe, &mut read, now)?;
        store.stamp(SOURCE, &commit.hash, VERSION, now)?;
        read.commits += 1;
    }
    Ok(read)
}

/// One commit's effect on what is held.
fn apply(
    store: &mut Store,
    scope: &ScopeId,
    commit: &Commit,
    held: &mut Vec<Held>,
    probe: &mut Probe,
    read: &mut Read,
    now: Timestamp,
) -> Result<(), DistilError> {
    if let Some(original) = reverted(&commit.subject) {
        for id in store.committed_with(&format!("({original})"))? {
            store.close(&id, now)?;
            read.closed += 1;
        }
    }
    for (file, removed, added) in &commit.swaps {
        let (gone, came) = (tokens(removed), tokens(added));
        let lost: Vec<&String> = gone.iter().filter(|v| !came.contains(v)).collect();
        let Some(at) = held
            .iter()
            .position(|h| h.values.iter().any(|v| lost.contains(&v) && probe.value(v)))
        else {
            continue;
        };
        let was = held.remove(at);
        let new_values: Vec<&String> = came.iter().filter(|v| !gone.contains(v)).collect();
        let text = match new_values.as_slice() {
            [one] => swapped(&was.text, &was.values, lost.as_slice(), one),
            _ => format!("{file} now reads: {}", clipped(added)),
        };
        let note = format!(
            "commit {} ({}): {file} now reads {}",
            short(&commit.hash),
            commit.subject,
            clipped(added)
        );
        let id = store
            .replace(
                &was.id,
                minted(scope, &text, commit.at, now),
                witness(commit, &note, scope),
                now,
            )?
            .id()
            .clone();
        read.closed += 1;
        read.recorded += 1;
        held.push(Held {
            values: values_in(&text),
            text,
            id,
        });
    }
    for value in &commit.removed {
        if commit.added.contains(value) {
            continue;
        }
        let Some(at) = held.iter().position(|h| h.values.contains(value)) else {
            continue;
        };
        if probe.vanished(value) {
            let was = held.remove(at);
            store.close(&was.id, now)?;
            read.closed += 1;
        }
    }
    Ok(())
}

/// The open facts a commit may speak to: ones the person stated, and ones a commit recorded.
fn candidates(store: &Store) -> Result<Vec<Held>, DistilError> {
    let mut ask = Recall::of("", 0);
    ask.limit = CANDIDATES;
    ask.tiers = vec![Tier::Fact];
    ask.floor = 0.0;
    let mut out = Vec::new();
    for hit in store.recall_current(&ask)? {
        let memory = hit.memory();
        let vouched = store.witnesses_of(&memory.id)?.iter().any(|w| {
            matches!(
                w.kind,
                WitnessKind::Imperative
                    | WitnessKind::Correction
                    | WitnessKind::Choice
                    | WitnessKind::Commit
            )
        });
        let text = memory.text();
        let values = values_in(&text);
        if vouched && !values.is_empty() {
            out.push(Held {
                id: memory.id.clone(),
                text,
                values,
            });
        }
    }
    Ok(out)
}

/// The values a line names: words spelled like names, and words a slot word introduces.
fn values_in(text: &str) -> Vec<String> {
    let words: Vec<&str> = text
        .split(|c: char| !(c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '/')))
        .map(|w| w.trim_matches(['.', '-', '/']))
        .filter(|w| w.len() >= 2)
        .collect();
    let mut out: Vec<String> = Vec::new();
    for (i, word) in words.iter().enumerate() {
        let after_slot = i
            .checked_sub(1)
            .and_then(|k| words.get(k))
            .is_some_and(|w| SLOTS.contains(&w.to_lowercase().as_str()));
        let lower = word.to_lowercase();
        if (name_like(word) || (after_slot && word.len() >= 3)) && !out.contains(&lower) {
            out.push(lower);
        }
    }
    out
}

/// A changed line's words, lowercased: what it held or holds, valued or not.
fn tokens(line: &str) -> Vec<String> {
    line.split(|c: char| !(c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | '/')))
        .map(|w| w.trim_matches(['.', '-', '/']).to_lowercase())
        .filter(|w| w.len() >= 2)
        .collect()
}

/// Whether a word is spelled like a name: an inner capital, a digit beside letters, or a
/// `.`, `-` or `_` inside it.
fn name_like(word: &str) -> bool {
    let inner = word.chars().skip(1).any(char::is_uppercase);
    let digits = word.chars().any(|c| c.is_ascii_digit()) && word.chars().any(char::is_alphabetic);
    let joined = word.trim_matches(['.', '-', '_']).contains(['.', '-', '_']);
    inner || digits || joined
}

/// The held claim with its lost value written as the one that came in.
fn swapped(text: &str, values: &[String], lost: &[&String], new: &str) -> String {
    let Some(old) = values.iter().find(|v| lost.contains(v)) else {
        return text.to_owned();
    };
    text.split(' ')
        .map(|word| {
            let bare =
                word.trim_matches(|c: char| !c.is_alphanumeric() && !matches!(c, '.' | '-' | '_'));
            if bare.eq_ignore_ascii_case(old) {
                word.replacen(bare, new, 1)
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn minted(scope: &ScopeId, text: &str, when: Timestamp, now: Timestamp) -> Memory {
    let mut memory = Memory::new(
        mint(now),
        Tier::Fact,
        scope.clone(),
        Body::note(text, NoteKind::Claim),
        now,
    );
    memory.temporal = Temporal::recalled(now, when);
    memory
}

fn witness(commit: &Commit, note: &str, scope: &ScopeId) -> Witness {
    Witness::new(
        WitnessId::new(format!(
            "commit-{}-{}",
            short(&commit.hash),
            mint(commit.at)
        )),
        WitnessKind::Commit,
        SessionId::new(format!("commit:{}", short(&commit.hash))),
        scope.clone(),
        commit.at,
    )
    .through(Channel::Repository, Some(Domain::new("repo")))
    .noted(note.to_owned())
}

/// What a `Revert "…"` commit reverted, by its subject.
fn reverted(subject: &str) -> Option<&str> {
    subject.strip_prefix("Revert \"")?.strip_suffix('"')
}

fn short(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}

fn clipped(line: &str) -> String {
    line.trim().chars().take(160).collect()
}

/// `git log` output as commits, and whether the line bound cut it short.
fn parse(log: &str, most: usize) -> (Vec<Commit>, bool) {
    let mut out = Vec::new();
    let mut seen = 0_usize;
    for record in log.split('\u{1e}').filter(|r| !r.trim().is_empty()) {
        let mut lines = record.lines();
        let mut head = lines.next().unwrap_or_default().split('\u{1f}');
        let mut commit = Commit {
            hash: head.next().unwrap_or_default().to_owned(),
            at: head.next().and_then(|t| t.parse().ok()).unwrap_or(0),
            subject: head.next().unwrap_or_default().to_owned(),
            ..Commit::default()
        };
        let mut file: Option<String> = None;
        let mut hunk: (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
        for line in lines {
            seen += 1;
            if seen > most {
                close(&mut commit, file.as_deref(), &mut hunk);
                out.push(commit);
                return (out, true);
            }
            if line.starts_with("diff --git") {
                close(&mut commit, file.as_deref(), &mut hunk);
                file = None;
            } else if let Some(path) = line.strip_prefix("+++ b/") {
                file = Some(path.to_owned());
            } else if line.starts_with("+++ ") || line.starts_with("rename ") {
                file = None;
            } else if line.starts_with("@@") {
                close(&mut commit, file.as_deref(), &mut hunk);
            } else if file.is_some() && !line.starts_with("---") {
                if let Some(gone) = line.strip_prefix('-') {
                    hunk.0.push(gone.to_owned());
                } else if let Some(came) = line.strip_prefix('+') {
                    hunk.1.push(came.to_owned());
                }
            }
        }
        close(&mut commit, file.as_deref(), &mut hunk);
        out.push(commit);
    }
    (out, false)
}

/// End a hunk: a one-for-one change is a swap; every line's values count as removed or added.
fn close(commit: &mut Commit, file: Option<&str>, hunk: &mut (Vec<String>, Vec<String>)) {
    let (gone, came) = std::mem::take(hunk);
    let Some(file) = file else {
        return;
    };
    for line in &gone {
        commit.removed.extend(tokens(line));
    }
    for line in &came {
        commit.added.extend(tokens(line));
    }
    if let ([removed], [added]) = (gone.as_slice(), came.as_slice()) {
        commit
            .swaps
            .push((file.to_owned(), removed.clone(), added.clone()));
    }
}

/// The configured `argv` with its placeholders filled, run; its output, when it succeeded.
fn run(argv: &[String], repo: &str, count: &str, word: &str) -> Option<String> {
    let argv: Vec<String> = argv
        .iter()
        .map(|arg| match arg.as_str() {
            "{repo}" => repo.to_owned(),
            "{count}" => count.to_owned(),
            "{word}" => word.to_owned(),
            other => other.to_owned(),
        })
        .collect();
    let ran = crate::child::run(&argv, &[], DEADLINE, OUTPUT).ok()?;
    ran.status
        .success()
        .then(|| String::from_utf8_lossy(&ran.stdout).into_owned())
}

/// Questions asked of the tracked files, cached and bounded.
struct Probe<'a> {
    commands: &'a Commits,
    repo: &'a str,
    files: HashMap<String, usize>,
    checks: usize,
}

impl<'a> Probe<'a> {
    fn new(commands: &'a Commits, repo: &'a str) -> Self {
        Self {
            commands,
            repo,
            files: HashMap::new(),
            checks: 0,
        }
    }

    /// How many tracked files hold `word`, or `None` past this pass's bound.
    fn holding(&mut self, word: &str) -> Option<usize> {
        if let Some(n) = self.files.get(word) {
            return Some(*n);
        }
        if self.checks >= TREE_CHECKS {
            return None;
        }
        self.checks += 1;
        let n = run(&self.commands.grep, self.repo, "", word).map_or(0, |out| {
            out.lines().filter(|l| !l.trim().is_empty()).count()
        });
        self.files.insert(word.to_owned(), n);
        Some(n)
    }

    /// Whether `word` is a value rather than the repository's vocabulary.
    fn value(&mut self, word: &str) -> bool {
        name_like(word) || self.holding(word).is_some_and(|n| n <= VALUE_FILES)
    }

    /// Whether no tracked file holds `word` any more.
    fn vanished(&mut self, word: &str) -> bool {
        self.holding(word) == Some(0)
    }
}
