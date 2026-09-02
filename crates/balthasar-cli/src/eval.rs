//! `balthasar eval` — whether session k+1 is less annoying than session k.
//!
//! The number nobody else measures. LoCoMo and LongMemEval ask whether a conversation can be
//! recalled; this asks whether a coding agent stopped rediscovering things, which is the only
//! question a person using one actually has.
//!
//! Synthetic and reproducible: no clock, no model, no network, no embedder. A number from here
//! means the same thing on every machine.
//!
//! `--json` emits the full baseline — what scored, what it cost, and what produced it. A result
//! without its schema, configuration and revision cannot be compared against a later one, and a
//! success rate without a token count can be won by injecting everything.

use crate::render;
use balthasar_testkit::{Baseline, Scenario, Score};
use clap::Parser;
use std::path::PathBuf;

/// Measure whether memory earns its place.
#[derive(Debug, Parser)]
pub struct Args {
    /// How many sessions to run.
    #[arg(long, default_value_t = 10)]
    sessions: usize,

    /// Run the long scenario, where the history outruns the window.
    ///
    /// The only shape that separates memory from carrying the text forward: a lesson learned
    /// twice, then not mentioned again until far more has been said than a window can hold. On
    /// anything shorter the two arms tie, which is a real result and not a broken benchmark.
    #[arg(long)]
    long: bool,

    /// Use the several-lessons scenario, which is closer to a real project.
    #[arg(long)]
    varied: bool,

    /// Answer as JSON: the full baseline rather than the headline.
    #[arg(long)]
    json: bool,

    /// Report whether the store can tell a popular memory from a useful one.
    ///
    /// The distinction the ledger exists for. Runs the scenario twice — once crediting every
    /// retrieval, once crediting only attributed outcomes — and reports whether they disagree.
    #[arg(long = "with-utility")]
    with_utility: bool,

    /// Also run any external benchmark found in this directory.
    ///
    /// Nothing is downloaded and nothing is vendored. A dataset that is not there is reported
    /// as skipped, because a suite that breaks on a missing optional file is a suite people
    /// delete.
    #[arg(long, value_name = "DIR")]
    external: Option<PathBuf>,

    /// Report every metric group: correctness, outcomes, efficiency and safety.
    ///
    /// Each of them can be won at the expense of the others, so a claim about memory behaviour
    /// that shows one number is a claim that can be gamed.
    #[arg(long)]
    full: bool,

    /// Also write the baseline into this directory.
    ///
    /// Only ever when asked. An evaluation that wrote its own results into the project tree
    /// would make every run a working-tree change, and a benchmark nobody can run without
    /// dirtying the repository is one nobody runs.
    #[arg(long, value_name = "DIR")]
    artifact: Option<PathBuf>,
}

/// The moment the scenario starts. Fixed, so a run is reproducible.
const START: balthasar_model::Timestamp = 1_756_000_000;

/// Run it, with memory and without.
pub fn run(args: &Args) -> anyhow::Result<()> {
    anyhow::ensure!(args.sessions > 0, "how many sessions?");
    let name = match (args.long, args.varied) {
        (true, _) => "many-lessons",
        (false, true) => "several-lessons",
        (false, false) => "one-lesson",
    };
    let scenario = match (args.long, args.varied) {
        (true, _) => Scenario::many_lessons(args.sessions, START),
        (false, true) => Scenario::several_lessons(args.sessions, START),
        (false, false) => Scenario::one_lesson(args.sessions, START),
    };

    let settings = balthasar_lua::Settings::default();
    let baseline = Baseline::of(&scenario, name, START, &settings);
    let with = balthasar_testkit::run(&scenario, true);
    let without = balthasar_testkit::run(&scenario, false);
    let ceiling = Score::ceiling(&scenario);

    if let Some(into) = &args.artifact {
        let path = into.join(format!("balthasar-baseline-{}.json", baseline.run_id));
        std::fs::create_dir_all(into)?;
        std::fs::write(&path, serde_json::to_string_pretty(&baseline)?)?;
        eprintln!("{}", render::dim(&path.display().to_string()));
    }

    if args.with_utility {
        say_utility_split();
    }

    if let Some(at) = &args.external {
        say_external(at);
    }

    if args.full {
        let held = balthasar_testkit::Full::measure(&scenario, name, START);
        if args.json {
            crate::say!("{}", serde_json::to_string(&held)?);
        } else {
            say_full(&held);
        }
        return Ok(());
    }

    if args.json {
        crate::say!("{}", serde_json::to_string(&baseline)?);
        return Ok(());
    }
    crate::say!(
        "{} session(s) in one project, {}",
        render::bold(&args.sessions.to_string()),
        if args.varied {
            "three lessons met in different orders"
        } else {
            "one lesson"
        }
    );
    crate::say!();
    crate::say!(
        "  {}  {:.0}%  {}",
        render::bold("with memory   "),
        with.hit_rate() * 100.0,
        render::bar(with.hit_rate())
    );
    crate::say!(
        "  {}  {:.0}%  {}",
        render::dim("without memory"),
        without.hit_rate() * 100.0,
        render::bar(without.hit_rate())
    );
    crate::say!(
        "  {}  {:.0}%  {}",
        render::dim("the ceiling   "),
        ceiling * 100.0,
        render::dim("the first session of a project has nothing to know from")
    );
    crate::say!();
    crate::say!(
        "{}",
        render::dim(&format!(
            "{} lesson(s) learned the hard way, {} already known",
            with.rediscoveries, with.recalls
        ))
    );

    // The sessions themselves, so a regression can be seen rather than inferred from one
    // number moving.
    let stumbled: Vec<&str> = with
        .sessions
        .iter()
        .skip(1)
        .filter(|s| !s.rediscovered.is_empty())
        .map(|s| s.session.as_str())
        .collect();
    if !stumbled.is_empty() {
        crate::say!();
        crate::say!(
            "{}",
            render::dim(&format!(
                "still rediscovering after the first: {}",
                stumbled.join(", ")
            ))
        );
    }
    Ok(())
}

/// Every metric group, side by side.
///
/// Deliberately four blocks rather than one score. A system that asserts nothing is perfectly
/// safe, one that asserts everything recalls perfectly, and one that injects the whole store
/// has the best task success anybody has measured — averaging them would hide all three.
fn say_full(held: &balthasar_testkit::Full) {
    let b = &held.baseline;

    crate::say!("{}", render::bold("correctness"));
    crate::say!(
        "  {:>7}  {}",
        format!("{}/{}", held.scenarios_passed, held.scenarios),
        render::dim("scenario probes")
    );
    crate::say!(
        "  {:>7}  {}",
        format!("{:.2}", b.recall_precision),
        render::dim("recall precision — of what was asserted, how much was right")
    );
    crate::say!(
        "  {:>7}  {}",
        format!("{:.2}", b.assertion_accuracy),
        render::dim("assertion accuracy — nothing superseded was stated as current")
    );

    crate::say!();
    crate::say!("{}", render::bold("agent outcomes"));
    crate::say!(
        "  {:>7}  {}",
        format!("{:.0}%", b.task_success * 100.0),
        render::dim(&format!(
            "task success against a {:.0}% ceiling, {:.0}% without memory",
            b.ceiling * 100.0,
            b.without_memory * 100.0
        ))
    );
    // The arm that can lose, printed beside the one that cannot. A report without it can be won
    // by a system that is worse than doing nothing clever at all, which is the failure this
    // exists to make visible.
    crate::say!(
        "  {:>7}  {}",
        format!("{:.0}%", b.in_window * 100.0),
        render::dim(&format!(
            "the same history in the window, no memory at all — balthasar is {}",
            if b.task_success > b.in_window {
                format!(
                    "ahead by {:.0} points",
                    (b.task_success - b.in_window) * 100.0
                )
            } else if b.task_success < b.in_window {
                format!(
                    "BEHIND by {:.0} points",
                    (b.in_window - b.task_success) * 100.0
                )
            } else {
                "level".to_owned()
            }
        ))
    );
    crate::say!(
        "  {:>7}  {}",
        b.crossover
            .map_or_else(|| "—".to_owned(), |n| n.to_string()),
        render::dim(match b.crossover {
            Some(_) => "sessions before memory catches the window",
            None => "the window was still ahead at the end of this scenario",
        })
    );
    crate::say!(
        "  {:>7}  {}",
        b.avoidable_failures,
        render::dim("avoidable failures — rediscoveries memory could have prevented")
    );

    crate::say!();
    crate::say!("{}", render::bold("efficiency"));
    crate::say!(
        "  {:>7}  {}",
        b.injected_tokens,
        render::dim("injected tokens — what the success cost")
    );
    crate::say!(
        "  {:>7}  {}",
        format!("{}k", b.store_bytes / 1024),
        render::dim("store bytes")
    );
    crate::say!(
        "  {:>7}  {}",
        format!("{:.2}ms", b.recall_p95_ms),
        render::dim(&format!("recall p95, p50 {:.2}ms", b.recall_p50_ms))
    );

    crate::say!();
    crate::say!("{}", render::bold("safety"));
    crate::say!(
        "  {:>7}  {}",
        format!("{:.0}%", held.attack_success_rate * 100.0),
        render::dim(&format!(
            "attack success rate over {} deterministic attacks",
            held.attacks
        ))
    );

    crate::say!();
    crate::say!(
        "{}",
        render::dim(&format!(
            "schema v{} · {} · config {}",
            b.schema_version, b.git_revision, b.config_fingerprint
        ))
    );
    if !held.is_clean() {
        crate::say!(
            "{}",
            render::bold("something is not where it should be — see the groups above")
        );
    }
}

/// What each external benchmark had to say, and what it could not say.
fn say_external(at: &std::path::Path) {
    use balthasar_testkit::Family;

    crate::say!("{}", render::bold("external benchmarks"));
    for family in [
        Family::LongMemEval,
        Family::LoCoMo,
        Family::MemoryAgentBench,
    ] {
        let found = balthasar_testkit::load(family, at);
        crate::say!("  {}", render::dim(&found.describe()));
    }
    crate::say!();
    crate::say!(
        "{}",
        render::dim(
            "these ask whether a conversation can be recalled. None of them asks whether a \
             procedure stopped working, or whether an untrusted page got through — the local \
             suite is still the gate."
        )
    );
    crate::say!();
}

/// Whether retrieval count and attributed utility can actually diverge.
///
/// Not a benchmark of balthasar against something else — a demonstration that the two numbers are
/// separable at all. A store that could not tell them apart would report identical figures here
/// however much ledger machinery it carried.
fn say_utility_split() {
    use balthasar_model::{
        Attribution, Body, Memory, NoteKind, OutcomeKind, Presentation, ScopeId, SessionId, Tier,
        Witness, WitnessId, WitnessKind,
    };
    use balthasar_store::{Candidate, Injection, RecallRun, Signals, Store, Use, Verdict, mint};

    let at = START;
    let scope = ScopeId::new("/w/bench");
    let run = SessionId::new("01BENCH");
    let mut store = Store::ephemeral().expect("store");

    let mut kept = |text: &str, n: usize| {
        let held = Memory::new(
            mint(at + n as i64),
            Tier::Fact,
            scope.clone(),
            Body::note(text, NoteKind::Claim),
            at,
        );
        let id = held.id.clone();
        store
            .remember(
                held,
                Witness::new(
                    WitnessId::new(format!("w{n}")),
                    WitnessKind::Imperative,
                    run.clone(),
                    scope.clone(),
                    at,
                ),
                at,
            )
            .expect("remember");
        id
    };
    let popular = kept("a memory every query happens to match", 0);
    let useful = kept("the memory somebody actually followed", 1);

    // Twenty retrievals for one, a single attributed success for the other.
    for n in 0..20 {
        store
            .note_recall(
                &RecallRun {
                    id: format!("r{n}"),
                    scope: scope.clone(),
                    session: Some(run.clone()),
                    query_hash: "hash".to_owned(),
                    requested_at: at + n,
                    config_fingerprint: "cfg".to_owned(),
                    vector_available: false,
                    result_limit: 10,
                    latency_us: 100,
                },
                &[Candidate {
                    memory: popular.clone(),
                    rank: 0,
                    selected: true,
                    score: 0.9,
                    signals: Signals::default(),
                }],
            )
            .expect("recall");
    }
    store
        .note_injection(
            &Injection {
                id: "i1".to_owned(),
                recall: None,
                session: Some(run.clone()),
                created_at: at,
                token_count: 10,
                remote: false,
                policy: "balanced".to_owned(),
            },
            &[(useful.clone(), Presentation::Asserted)],
        )
        .expect("inject");
    store
        .note_use(&Use {
            id: "a1".to_owned(),
            injection: Some("i1".to_owned()),
            session: Some(run),
            reported_at: at,
            tool: Some("shell".to_owned()),
            action_hash: "h".to_owned(),
            attribution: Attribution::Explicit,
            memories: vec![useful.clone()],
        })
        .expect("use");
    store
        .note_outcome(&Verdict {
            id: "o1".to_owned(),
            action: "a1".to_owned(),
            observed_at: at,
            kind: OutcomeKind::Succeeded,
            score: None,
            evidence_cursor: None,
            evaluator: "caller".to_owned(),
            note: None,
        })
        .expect("outcome");

    let (seen_popular, _) = store.times_retrieved(&popular).unwrap_or((0, 0));
    let (seen_useful, _) = store.times_retrieved(&useful).unwrap_or((0, 0));
    let up = store.utility_of(&popular).expect("utility");
    let uu = store.utility_of(&useful).expect("utility");

    crate::say!("{}", render::bold("popularity against usefulness"));
    crate::say!();
    crate::say!(
        "  {:<24}  {:>3} retrieved   {:>2} verified helpful",
        render::dim("matched everything"),
        seen_popular,
        up.verified_helpful
    );
    crate::say!(
        "  {:<24}  {:>3} retrieved   {:>2} verified helpful",
        render::dim("actually followed"),
        seen_useful,
        uu.verified_helpful
    );
    crate::say!();
    if up.is_verified() || !uu.is_verified() {
        crate::say!(
            "{}",
            render::bold("the two are not separable — the ledger is not doing its job")
        );
    } else {
        crate::say!(
            "{}",
            render::dim(
                "twenty retrievals bought no evidence; one attributed outcome did. Access count \
                 is not utility."
            )
        );
    }
    crate::say!();
}
