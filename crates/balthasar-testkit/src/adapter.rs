//! Reading somebody else's benchmark.

use crate::{Act, Case, Category, Expect, Probe};
use std::path::Path;

/// A benchmark balthasar knows how to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Long conversational histories with question sets.
    LongMemEval,
    /// Multi-session dialogue with temporal questions.
    LoCoMo,
    /// Agentic memory tasks.
    MemoryAgentBench,
}

impl Family {
    /// What it is called.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LongMemEval => "longmemeval",
            Self::LoCoMo => "locomo",
            Self::MemoryAgentBench => "memoryagentbench",
        }
    }

    /// Which of balthasar's categories its questions map onto.
    #[must_use]
    pub fn covers(self) -> &'static [Category] {
        match self {
            Self::LongMemEval => &[
                Category::SingleSession,
                Category::MultiSession,
                Category::KnowledgeUpdate,
                Category::Temporal,
                Category::Abstention,
            ],
            Self::LoCoMo => &[
                Category::MultiSession,
                Category::Temporal,
                Category::Preference,
            ],
            Self::MemoryAgentBench => &[Category::MultiSession, Category::Preference],
        }
    }

    /// What it cannot tell you about, however well balthasar scores.
    #[must_use]
    pub fn blind_to(self) -> &'static [Category] {
        &[
            Category::RepeatedRepair,
            Category::EnvironmentShift,
            Category::Poisoning,
            Category::Purge,
            Category::ScopeConflict,
        ]
    }
}

/// One dataset, if it is there.
#[derive(Debug, Clone)]
pub struct Dataset {
    /// Which benchmark.
    pub family: Family,
    /// What was read.
    pub cases: Vec<Case>,
}

/// What happened when a dataset was looked for.
#[derive(Debug, Clone)]
pub enum Found {
    /// It was there and read.
    Read(Dataset),
    /// It was not there, which is not an error.
    Absent(&'static str),
    /// It was there and could not be understood.
    Unreadable(String),
}

impl Found {
    /// Whether anything can be run.
    #[must_use]
    pub fn is_runnable(&self) -> bool {
        matches!(self, Self::Read(held) if !held.cases.is_empty())
    }

    /// A sentence for a report.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Read(held) => format!("{}: {} case(s)", held.family.as_str(), held.cases.len()),
            Self::Absent(family) => format!("{family}: not present — skipped"),
            Self::Unreadable(why) => format!("unreadable: {why}"),
        }
    }
}

/// Look for a dataset, and read it if it is there.
///
/// Nothing here searches the filesystem, downloads anything, or caches anything.
pub fn load(family: Family, at: &Path) -> Found {
    let path = at.join(format!("{}.jsonl", family.as_str()));
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Found::Absent(family.as_str());
    };
    match parse(family, &text) {
        Ok(cases) => Found::Read(Dataset { family, cases }),
        Err(why) => Found::Unreadable(why),
    }
}

/// Turn a benchmark's own shape into scenarios.
///
/// One object per line: a list of turns and a list of questions. The mapping is shallow — every
/// turn becomes something said, every question becomes a probe.
fn parse(family: Family, text: &str) -> Result<Vec<Case>, String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let held: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("line {}: {e}", n + 1))?;

        let turns = held
            .get("turns")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| format!("line {}: no turns", n + 1))?;
        let questions = held
            .get("questions")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| format!("line {}: no questions", n + 1))?;

        let script: Vec<Act> = turns
            .iter()
            .enumerate()
            .filter_map(|(at, turn)| {
                let text = turn.get("text").and_then(serde_json::Value::as_str)?;
                Some(Act::Said {
                    session: leak(format!("bench-{n}")),
                    at: 1_700_000_000 + at as i64 * 3600,
                    text: leak(text.to_owned()),
                })
            })
            .collect();

        let probes: Vec<Probe> = questions
            .iter()
            .filter_map(|question| {
                let asks = question.get("asks").and_then(serde_json::Value::as_str)?;
                let expect = match question.get("answer").and_then(serde_json::Value::as_str) {
                    Some(answer) => Expect::Asserted(leak(answer.to_owned())),
                    // A question with no answer is an abstention question, not an omission.
                    None => Expect::Silent,
                };
                Some(Probe {
                    asks: leak(asks.to_owned()),
                    at: 1_700_000_000 + turns.len() as i64 * 3600 + 60,
                    expect,
                    why: "an external benchmark question",
                })
            })
            .collect();

        if script.is_empty() || probes.is_empty() {
            continue;
        }
        out.push(Case {
            name: leak(format!("{}-{n}", family.as_str())),
            category: Category::MultiSession,
            project: "/bench",
            script,
            probes,
        });
    }
    Ok(out)
}

/// A string that outlives the parse.
///
/// [`Case`] holds `&'static str`, so a dataset read at runtime has its strings leaked
/// deliberately: a benchmark process runs once and exits.
fn leak(held: String) -> &'static str {
    Box::leak(held.into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use balthasar_model::scratch::Scratch;

    fn scratch(name: &str) -> Scratch {
        Scratch::new("balthasar-adapter", name)
    }

    #[test]
    fn a_missing_dataset_is_not_an_error() {
        // The property that keeps `verify` green on a machine that has never heard of these.
        let held = load(Family::LongMemEval, &scratch("absent"));
        assert!(matches!(held, Found::Absent(_)));
        assert!(!held.is_runnable());
        assert!(held.describe().contains("skipped"));
    }

    #[test]
    fn a_dataset_that_is_there_is_read() {
        let at = scratch("present");
        std::fs::write(
            at.join("locomo.jsonl"),
            r#"{"turns":[{"text":"my sister lives in Lisbon"}],"questions":[{"asks":"where does my sister live","answer":"Lisbon"}]}
"#,
        )
        .expect("write");

        let held = load(Family::LoCoMo, &at);
        assert!(held.is_runnable(), "{}", held.describe());
        let Found::Read(dataset) = held else {
            panic!("not read")
        };
        assert_eq!(dataset.cases.len(), 1);
        assert_eq!(dataset.cases[0].script.len(), 1);
        assert_eq!(dataset.cases[0].probes.len(), 1);
    }

    #[test]
    fn a_question_with_no_answer_becomes_an_abstention() {
        // Unanswerable questions are the hardest cases; dropping them would flatter every score.
        let at = scratch("abstain");
        std::fs::write(
            at.join("longmemeval.jsonl"),
            r#"{"turns":[{"text":"we talked about the weather"}],"questions":[{"asks":"what is my bank balance"}]}
"#,
        )
        .expect("write");

        let Found::Read(dataset) = load(Family::LongMemEval, &at) else {
            panic!("not read")
        };
        assert!(matches!(dataset.cases[0].probes[0].expect, Expect::Silent));
    }

    #[test]
    fn a_malformed_dataset_says_where_it_broke() {
        let at = scratch("broken");
        std::fs::write(at.join("locomo.jsonl"), "{not json}\n").expect("write");
        let held = load(Family::LoCoMo, &at);
        assert!(matches!(held, Found::Unreadable(_)));
        assert!(held.describe().contains("line 1"), "{}", held.describe());
    }

    #[test]
    fn every_family_says_what_it_cannot_measure() {
        // A conversational benchmark says nothing about a procedure that stopped working.
        for family in [
            Family::LongMemEval,
            Family::LoCoMo,
            Family::MemoryAgentBench,
        ] {
            assert!(!family.covers().is_empty(), "{}", family.as_str());
            assert!(!family.blind_to().is_empty(), "{}", family.as_str());
            for blind in family.blind_to() {
                assert!(
                    !family.covers().contains(blind),
                    "{} claims to cover {}",
                    family.as_str(),
                    blind.as_str()
                );
            }
        }
    }

    #[test]
    fn nothing_here_reaches_the_network_or_the_filesystem_at_large() {
        // `load` opens exactly one path, derived from what the caller passed.
        let at = scratch("scoped");
        let held = load(Family::MemoryAgentBench, &at);
        assert!(matches!(held, Found::Absent(_)));
        assert!(
            std::fs::read_dir(&at).expect("read").next().is_none(),
            "looking left something behind"
        );
    }
}
