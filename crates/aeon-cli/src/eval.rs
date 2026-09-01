//! `aeon eval` — whether session k+1 is less annoying than session k.
//!
//! The number nobody else measures. LoCoMo and LongMemEval ask whether a conversation can be
//! recalled; this asks whether a coding agent stopped rediscovering things, which is the only
//! question a person using one actually has.
//!
//! Synthetic and reproducible: no clock, no model, no network, no embedder. A number from here
//! means the same thing on every machine.

use crate::render;
use aeon_testkit::{Scenario, Score};
use clap::Parser;

/// Measure whether memory earns its place.
#[derive(Debug, Parser)]
pub struct Args {
    /// How many sessions to run.
    #[arg(long, default_value_t = 10)]
    sessions: usize,

    /// Use the several-lessons scenario, which is closer to a real project.
    #[arg(long)]
    varied: bool,

    /// Answer as JSON.
    #[arg(long)]
    json: bool,
}

/// The moment the scenario starts. Fixed, so a run is reproducible.
const START: aeon_model::Timestamp = 1_756_000_000;

/// Run it, with memory and without.
pub fn run(args: &Args) -> anyhow::Result<()> {
    anyhow::ensure!(args.sessions > 0, "how many sessions?");
    let scenario = if args.varied {
        Scenario::several_lessons(args.sessions, START)
    } else {
        Scenario::one_lesson(args.sessions, START)
    };

    let with = aeon_testkit::run(&scenario, true);
    let without = aeon_testkit::run(&scenario, false);
    let ceiling = Score::ceiling(&scenario);

    if args.json {
        crate::say!(
            "{}",
            serde_json::json!({
                "sessions": args.sessions,
                "scenario": if args.varied { "several-lessons" } else { "one-lesson" },
                "with_memory": with.hit_rate(),
                "without_memory": without.hit_rate(),
                "ceiling": ceiling,
                "rediscoveries": with.rediscoveries,
                "recalls": with.recalls,
            })
        );
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
