//! What happened, and what should be true afterwards.

use memo_model::Timestamp;

/// The axes the published benchmarks separate, plus the ones only a decaying store has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    /// A fact stated once, in one session, and asked about later.
    SingleSession,
    /// An answer that only exists by joining two sessions that never met.
    MultiSession,
    /// The user's state changed and the old answer must stop being given.
    KnowledgeUpdate,
    /// What was true *then*, as distinct from what is true now.
    Temporal,
    /// A preference that should shape an answer rather than be recited.
    Preference,
    /// Nothing here answers the question, and saying so is the correct behaviour.
    Abstention,
    /// A plausible-looking distractor that must not be mistaken for an answer.
    Adversarial,
    /// Time passes and unused things stop being asserted.
    Decay,
    /// What the agent keeps needing resists fading.
    Inertia,
    /// Two runs agreeing outweighs one run insisting.
    Diversity,
    /// A query naming a thing, whatever words it used.
    Entity,
    /// A set of answers, against one current answer.
    Cardinality,
    /// One project's memory does not leak into another's.
    Isolation,
    /// The same failure met and fixed again, in a later run.
    RepeatedRepair,
    /// A project procedure and a global one that disagree.
    ScopeConflict,
    /// What worked on one machine, asked about on another.
    EnvironmentShift,
    /// A person correcting the agent mid-run.
    Correction,
    /// A fact nobody contradicted and nobody has re-seen in a year.
    Staleness,
    /// A memory that shares words with the question and answers nothing.
    LexicalTrap,
    /// Content that arrived from outside, repeated until it looks corroborated.
    Poisoning,
    /// A question whose premise is false.
    FalsePremise,
    /// Something explicitly removed, and every route back to it.
    Purge,
}

impl Category {
    /// How it prints in the scorecard.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SingleSession => "single-session",
            Self::MultiSession => "multi-session",
            Self::KnowledgeUpdate => "knowledge-update",
            Self::Temporal => "temporal",
            Self::Preference => "preference",
            Self::Abstention => "abstention",
            Self::Adversarial => "adversarial",
            Self::Decay => "decay",
            Self::Inertia => "inertia",
            Self::Diversity => "diversity",
            Self::Entity => "entity",
            Self::Cardinality => "cardinality",
            Self::Isolation => "isolation",
            Self::RepeatedRepair => "repeated-repair",
            Self::ScopeConflict => "scope-conflict",
            Self::EnvironmentShift => "environment-shift",
            Self::Correction => "correction",
            Self::Staleness => "staleness",
            Self::LexicalTrap => "lexical-trap",
            Self::Poisoning => "poisoning",
            Self::FalsePremise => "false-premise",
            Self::Purge => "purge",
        }
    }

    /// Every category, for a scorecard that shows the empty ones too.
    #[must_use]
    pub fn all() -> Vec<Self> {
        vec![
            Self::SingleSession,
            Self::MultiSession,
            Self::KnowledgeUpdate,
            Self::Temporal,
            Self::Preference,
            Self::Abstention,
            Self::Adversarial,
            Self::Decay,
            Self::Inertia,
            Self::Diversity,
            Self::Entity,
            Self::Cardinality,
            Self::Isolation,
            Self::RepeatedRepair,
            Self::ScopeConflict,
            Self::EnvironmentShift,
            Self::Correction,
            Self::Staleness,
            Self::LexicalTrap,
            Self::Poisoning,
            Self::FalsePremise,
            Self::Purge,
        ]
    }
}

/// One thing that happened.
#[derive(Debug, Clone)]
pub enum Act {
    /// The person said something, in a session, at a time.
    Said {
        /// Which run.
        session: &'static str,
        /// When.
        at: Timestamp,
        /// What they said.
        text: &'static str,
    },
    /// A tool ran.
    Tool {
        /// Which run.
        session: &'static str,
        /// When.
        at: Timestamp,
        /// The command.
        command: &'static str,
        /// Whether it worked.
        ok: bool,
    },
    /// A consolidation pass ran.
    Consolidate {
        /// When.
        at: Timestamp,
    },
    /// A decay pass ran.
    Decay {
        /// When.
        at: Timestamp,
    },
    /// The agent needed something, which reinforces it.
    Used {
        /// When.
        at: Timestamp,
        /// What it looked for.
        query: &'static str,
    },
    /// Something arrived from outside — a page, a document, an import.
    ///
    /// Carries an origin, so several arrivals of the same material are one source rather than
    /// several confirmations. This is what a poisoning scenario is built out of.
    Read {
        /// Which run read it.
        session: &'static str,
        /// When.
        at: Timestamp,
        /// Where it came from.
        origin: &'static str,
        /// What it said.
        text: &'static str,
    },
    /// Somebody asked for something to be removed.
    Purged {
        /// When.
        at: Timestamp,
        /// Enough of the text to find it by.
        matching: &'static str,
    },
    /// Work happened in a different project.
    Elsewhere {
        /// Which project.
        project: &'static str,
        /// Which run.
        session: &'static str,
        /// When.
        at: Timestamp,
        /// What was said.
        text: &'static str,
    },
}

/// What should be true when the question is asked.
#[derive(Debug, Clone)]
pub enum Expect {
    /// A memory containing this is asserted — the model would be told.
    Asserted(&'static str),
    /// A memory containing this is found, and is *not* asserted.
    ///
    /// The band nothing else on this shelf has, and the reason abstention is possible.
    Findable(&'static str),
    /// Nothing is asserted at all. The correct answer is "I do not know".
    Silent,
    /// A memory containing this is not asserted, whatever else is.
    NotAsserted(&'static str),
    /// A memory containing this is nowhere in the store.
    Absent(&'static str),
}

/// One question, and what should come back.
#[derive(Debug, Clone)]
pub struct Probe {
    /// What is asked.
    pub asks: &'static str,
    /// When.
    pub at: Timestamp,
    /// What should be true.
    pub expect: Expect,
    /// What this is really testing, printed when it fails.
    pub why: &'static str,
}

/// One scenario.
#[derive(Debug, Clone)]
pub struct Case {
    /// Its name in the scorecard.
    pub name: &'static str,
    /// Which axis it exercises.
    pub category: Category,
    /// Which project it happens in.
    pub project: &'static str,
    /// What happened, in order.
    pub script: Vec<Act>,
    /// What should be true afterwards.
    pub probes: Vec<Probe>,
}

const DAY: Timestamp = 86_400;
const START: Timestamp = 1_700_000_000;

/// A moment, `n` days in.
const fn day(n: i64) -> Timestamp {
    START + n * DAY
}

/// Every scenario.
///
/// Deliberately hostile in places. A suite that only asks what a system is good at measures
/// nothing, and the abstention and adversarial cases are here to fail if the floors are ever
/// loosened.
#[must_use]
pub fn corpus() -> Vec<Case> {
    vec![
        // ── single session ──────────────────────────────────────────────────
        Case {
            name: "a thing said once, plainly",
            category: Category::SingleSession,
            project: "/w/thing",
            script: vec![Act::Said {
                session: "s1",
                at: day(0),
                text: "remember: the staging box is at 10.0.0.7",
            }],
            probes: vec![Probe {
                asks: "staging box",
                at: day(1),
                expect: Expect::Asserted("10.0.0.7"),
                why: "an instruction crosses alone and is asserted the next day",
            }],
        },
        Case {
            name: "a thing said once, in passing",
            category: Category::SingleSession,
            project: "/w/thing",
            script: vec![Act::Said {
                session: "s1",
                at: day(0),
                text: "I think the cache is probably in redis somewhere",
            }],
            probes: vec![Probe {
                asks: "cache",
                at: day(1),
                expect: Expect::Silent,
                why: "a passing guess is not an instruction and must not be asserted",
            }],
        },
        // ── multi session ───────────────────────────────────────────────────
        Case {
            name: "two runs agreeing",
            category: Category::MultiSession,
            project: "/w/thing",
            script: vec![
                Act::Said {
                    session: "s1",
                    at: day(0),
                    text: "the database is postgres",
                },
                Act::Said {
                    session: "s2",
                    at: day(3),
                    text: "the database is postgres",
                },
                Act::Consolidate { at: day(4) },
            ],
            probes: vec![Probe {
                asks: "database",
                at: day(4),
                expect: Expect::Asserted("postgres"),
                why: "two unrelated runs corroborating carries a claim into the project",
            }],
        },
        Case {
            name: "one run repeating itself",
            category: Category::Diversity,
            project: "/w/thing",
            script: vec![
                Act::Said {
                    session: "s1",
                    at: day(0),
                    text: "the queue is rabbitmq",
                },
                Act::Said {
                    session: "s1",
                    at: day(0) + 600,
                    text: "the queue is rabbitmq",
                },
                Act::Said {
                    session: "s1",
                    at: day(0) + 1200,
                    text: "the queue is rabbitmq",
                },
                Act::Consolidate { at: day(1) },
            ],
            probes: vec![Probe {
                asks: "queue",
                at: day(1),
                expect: Expect::Silent,
                why: "one run being emphatic is not corroboration, however often it repeats",
            }],
        },
        // ── knowledge update ────────────────────────────────────────────────
        Case {
            name: "the answer changed",
            category: Category::KnowledgeUpdate,
            project: "/w/thing",
            script: vec![
                Act::Said {
                    session: "s1",
                    at: day(0),
                    text: "remember: we deploy to heroku",
                },
                Act::Said {
                    session: "s2",
                    at: day(10),
                    text: "remember: we deploy to fly.io",
                },
            ],
            probes: vec![
                Probe {
                    asks: "deploy",
                    at: day(11),
                    expect: Expect::Asserted("fly.io"),
                    why: "the current answer is the one that is asserted",
                },
                Probe {
                    asks: "deploy",
                    at: day(11),
                    expect: Expect::NotAsserted("heroku"),
                    why: "the superseded answer must stop being stated",
                },
                Probe {
                    asks: "heroku",
                    at: day(11),
                    expect: Expect::Findable("heroku"),
                    why: "and must still be findable — nothing is deleted",
                },
            ],
        },
        // ── temporal ────────────────────────────────────────────────────────
        Case {
            name: "what was true then",
            category: Category::Temporal,
            project: "/w/thing",
            script: vec![
                Act::Said {
                    session: "s1",
                    at: day(0),
                    text: "remember: the version is 1.2",
                },
                Act::Said {
                    session: "s2",
                    at: day(30),
                    text: "remember: the version is 2.0",
                },
            ],
            probes: vec![
                Probe {
                    asks: "version",
                    at: day(31),
                    expect: Expect::Asserted("2.0"),
                    why: "asked today, the answer is what is true today",
                },
                Probe {
                    asks: "version",
                    at: day(31),
                    expect: Expect::NotAsserted("1.2"),
                    why: "and what was true a month ago is not stated as if it still were",
                },
            ],
        },
        // ── preference ──────────────────────────────────────────────────────
        Case {
            name: "a stated preference",
            category: Category::Preference,
            project: "/w/thing",
            script: vec![Act::Said {
                session: "s1",
                at: day(0),
                text: "always use tabs, never spaces",
            }],
            probes: vec![Probe {
                asks: "tabs",
                at: day(2),
                expect: Expect::Asserted("tabs"),
                why: "a preference is an instruction and is carried",
            }],
        },
        // ── abstention ──────────────────────────────────────────────────────
        Case {
            name: "nothing was ever said about it",
            category: Category::Abstention,
            project: "/w/thing",
            script: vec![Act::Said {
                session: "s1",
                at: day(0),
                text: "remember: the tests are run with make test",
            }],
            probes: vec![Probe {
                asks: "what is the production database password",
                at: day(1),
                expect: Expect::Silent,
                why: "a store that answers this has invented something",
            }],
        },
        Case {
            name: "the only thing said is too weak to state",
            category: Category::Abstention,
            project: "/w/thing",
            script: vec![Act::Said {
                session: "s1",
                at: day(0),
                text: "maybe the timeout is thirty seconds, not sure",
            }],
            probes: vec![Probe {
                asks: "timeout",
                at: day(1),
                expect: Expect::Silent,
                why: "the two floors exist so a guess can be kept without being asserted",
            }],
        },
        // ── adversarial ─────────────────────────────────────────────────────
        Case {
            name: "a distractor that shares every word",
            category: Category::Adversarial,
            project: "/w/thing",
            script: vec![Act::Said {
                session: "s1",
                at: day(0),
                text: "remember: the staging box is at 10.0.0.7",
            }],
            probes: vec![Probe {
                asks: "production box",
                at: day(1),
                expect: Expect::NotAsserted("10.0.0.7"),
                why: "staging is not production, however close the words are",
            }],
        },
        // ── decay ───────────────────────────────────────────────────────────
        Case {
            name: "a passing remark, a year later",
            category: Category::Decay,
            project: "/w/thing",
            script: vec![
                Act::Said {
                    session: "s1",
                    at: day(0),
                    text: "note that the intern is called Sam",
                },
                Act::Decay { at: day(400) },
            ],
            probes: vec![Probe {
                asks: "intern",
                at: day(400),
                expect: Expect::NotAsserted("Sam"),
                why: "an unpinned normal-importance fact fades out of assertion in a year",
            }],
        },
        Case {
            name: "something shouted, a year later",
            category: Category::Decay,
            project: "/w/thing",
            script: vec![
                Act::Said {
                    session: "s1",
                    at: day(0),
                    text: "DUDE REMEMBER we deploy with fly!!",
                },
                Act::Decay { at: day(400) },
            ],
            probes: vec![Probe {
                asks: "deploy",
                at: day(400),
                expect: Expect::Asserted("fly"),
                why: "insisting pins, and a pinned memory does not fade",
            }],
        },
        // ── inertia ─────────────────────────────────────────────────────────
        Case {
            name: "what the agent keeps needing",
            category: Category::Inertia,
            project: "/w/thing",
            script: vec![
                Act::Said {
                    session: "s1",
                    at: day(0),
                    text: "note that the build takes forty seconds",
                },
                Act::Used {
                    at: day(30),
                    query: "build",
                },
                Act::Used {
                    at: day(60),
                    query: "build",
                },
                Act::Used {
                    at: day(90),
                    query: "build",
                },
                Act::Decay { at: day(120) },
            ],
            probes: vec![Probe {
                asks: "build",
                at: day(120),
                expect: Expect::Asserted("forty seconds"),
                why: "reaching for something resets its strength, so use resists decay",
            }],
        },
        // ── entity ──────────────────────────────────────────────────────────
        Case {
            name: "a query naming the thing",
            category: Category::Entity,
            project: "/w/thing",
            script: vec![Act::Said {
                session: "s1",
                at: day(0),
                text: "remember: `make test` is how the suite runs",
            }],
            probes: vec![Probe {
                asks: "`make test`",
                at: day(1),
                expect: Expect::Asserted("make test"),
                why: "a backticked command is one entity, not two words",
            }],
        },
        // ── cardinality ─────────────────────────────────────────────────────
        Case {
            name: "a repaired command",
            category: Category::Cardinality,
            project: "/w/thing",
            script: vec![
                Act::Tool {
                    session: "s1",
                    at: day(0),
                    command: "cargo test",
                    ok: false,
                },
                Act::Tool {
                    session: "s1",
                    at: day(0) + 60,
                    command: "make test",
                    ok: true,
                },
            ],
            probes: vec![Probe {
                asks: "tests",
                at: day(1),
                expect: Expect::Asserted("make test"),
                why: "a repair is worth keeping on its own — the cost path",
            }],
        },
        // ── isolation ───────────────────────────────────────────────────────
        Case {
            name: "another project's memory",
            category: Category::Isolation,
            project: "/w/thing",
            script: vec![Act::Elsewhere {
                project: "/w/other",
                session: "s9",
                at: day(0),
                text: "remember: we deploy to vercel",
            }],
            probes: vec![Probe {
                asks: "deploy",
                at: day(1),
                expect: Expect::Silent,
                why: "durable memory is one project's, and does not leak into another's",
            }],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_case_asks_something() {
        for case in corpus() {
            assert!(!case.probes.is_empty(), "{} probes nothing", case.name);
            assert!(!case.script.is_empty(), "{} does nothing", case.name);
        }
    }

    #[test]
    fn every_probe_says_what_it_is_testing() {
        // A failing probe with no `why` is a puzzle rather than a report.
        for case in corpus() {
            for probe in &case.probes {
                assert!(probe.why.len() > 12, "{}: {}", case.name, probe.asks);
            }
        }
    }

    #[test]
    fn the_hard_categories_are_actually_covered() {
        // A suite that only asks what a system is good at measures nothing.
        let covered: std::collections::BTreeSet<Category> =
            corpus().iter().map(|c| c.category).collect();
        for must in [
            Category::Abstention,
            Category::Adversarial,
            Category::Decay,
            Category::KnowledgeUpdate,
        ] {
            assert!(covered.contains(&must), "{} is untested", must.as_str());
        }
    }

    #[test]
    fn abstention_is_taken_seriously() {
        // The behaviour nothing else on this shelf has. If it were ever down to one case,
        // loosening a floor would stop being visible.
        let silent = corpus()
            .iter()
            .flat_map(|c| c.probes.iter())
            .filter(|p| matches!(p.expect, Expect::Silent))
            .count();
        assert!(silent >= 4, "only {silent} probes expect silence");
    }
}
