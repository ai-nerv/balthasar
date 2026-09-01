//! `aeon eval` — whether session k+1 is less annoying than session k.
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
use aeon_testkit::{Baseline, Scenario, Score};
use clap::Parser;
use std::path::PathBuf;

/// Measure whether memory earns its place.
#[derive(Debug, Parser)]
pub struct Args {
    /// How many sessions to run.
    #[arg(long, default_value_t = 10)]
    sessions: usize,

    /// Use the several-lessons scenario, which is closer to a real project.
    #[arg(long)]
    varied: bool,

    /// Answer as JSON: the full baseline rather than the headline.
    #[arg(long)]
    json: bool,

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
const START: aeon_model::Timestamp = 1_756_000_000;

/// Run it, with memory and without.
pub fn run(args: &Args) -> anyhow::Result<()> {
    anyhow::ensure!(args.sessions > 0, "how many sessions?");
    let name = if args.varied {
        "several-lessons"
    } else {
        "one-lesson"
    };
    let scenario = if args.varied {
        Scenario::several_lessons(args.sessions, START)
    } else {
        Scenario::one_lesson(args.sessions, START)
    };

    let settings = aeon_lua::Settings::default();
    let baseline = Baseline::of(&scenario, name, START, &settings);
    let with = aeon_testkit::run(&scenario, true);
    let without = aeon_testkit::run(&scenario, false);
    let ceiling = Score::ceiling(&scenario);

    if let Some(into) = &args.artifact {
        let path = into.join(format!("aeon-baseline-{}.json", baseline.run_id));
        std::fs::create_dir_all(into)?;
        std::fs::write(&path, serde_json::to_string_pretty(&baseline)?)?;
        eprintln!("{}", render::dim(&path.display().to_string()));
    }

    if args.full {
        let held = aeon_testkit::Full::measure(&scenario, name, START);
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
fn say_full(held: &aeon_testkit::Full) {
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
