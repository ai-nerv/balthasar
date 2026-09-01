//! What `aeon` does when you type it.
//!
//! The CLI is not a wrapper over the socket, and when the socket arrives at M5 it will not be a
//! wrapper over the CLI. Both call the same functions in the same crates, which is the only
//! arrangement in which they cannot drift into describing a memory two different ways.

mod ask;
mod configs;
mod consolidate;
mod context;
mod decay;
mod eval;
mod forget;
mod ingest;
mod init;
mod loaded;
mod promote;
mod recall;
mod reindex;
mod relate;
mod remember;
mod render;
mod replay;
mod serve;
mod sessions;
mod status;
mod trace;
mod transfer;
mod trust;

use clap::{Parser, Subcommand};
use loaded::Loaded;
use std::path::PathBuf;
use std::process::ExitCode;

/// Memory for agents.
#[derive(Debug, Parser)]
#[command(name = "aeon", version, about, disable_help_subcommand = true)]
struct Cli {
    /// Which memory to work in: `global`, `project`, or a path.
    ///
    /// `project` is the repository the working directory is in, so five worktrees of one
    /// project share one memory rather than each starting the others' amnesia.
    #[arg(long, global = true, default_value = "project")]
    scope: String,

    /// Which tool the memory belongs to.
    ///
    /// aeon keeps one store per tool per project, so a harness remembering a decision and a
    /// shell recording every command it ran do not share a decay curve or a ranking. Socket
    /// clients are named by the kernel and need not say; this is for the terminal.
    #[arg(long, global = true, value_name = "NAME")]
    tool: Option<String>,

    /// Work in a store somewhere else. For tests, and for looking at a copy.
    #[arg(long, global = true, value_name = "FILE")]
    store: Option<PathBuf>,

    /// Ignore every configuration file and use the shipped defaults.
    ///
    /// For the suite, which must not behave differently on the machine that runs it, and for
    /// working out whether a problem is aeon's or a config's.
    #[arg(long, global = true)]
    no_config: bool,

    /// Treat this unix time as now.
    ///
    /// Everything aeon decides is a function of when it is asked — what has faded, what is
    /// still asserted, how much a witness is still worth. A clock that cannot be moved is a
    /// design that can only be tested by waiting, so this exists for the suite and for
    /// backfilling transcripts that happened months ago.
    #[arg(long, global = true, value_name = "SECONDS", hide = true)]
    at: Option<i64>,

    #[command(subcommand)]
    what: Option<What>,
}

#[derive(Debug, Subcommand)]
enum What {
    /// Keep something, and say who says so.
    Remember(remember::Args),
    /// Search.
    Recall(recall::Args),
    /// Print the evidence for a memory, and what it adds up to.
    Why(ask::Args),
    /// Carry something out of a session and into the project's memory.
    Promote(promote::Args),
    /// Move a memory out of the live set, or remove it outright.
    Forget(forget::Args),
    /// Show exactly what a model would be told.
    Context(context::Args),
    /// Read a source's existing transcripts into memory.
    Ingest(ingest::Args),
    /// Give memories their vectors. Never on the critical path.
    Reindex(reindex::Args),
    /// Everything a run said, back out again.
    Replay(replay::Args),
    /// Which runs this project has had, and what each left behind.
    Sessions(sessions::Args),
    /// Carry what recurred across sessions into the project's memory. Shows first.
    Consolidate(consolidate::Args),
    /// Fade what has not been needed. Shows first; `--now` applies.
    Decay(decay::Args),
    /// Every memory, one JSON object per line.
    Export(transfer::ExportArgs),
    /// Read back what `export` wrote.
    Import(transfer::ImportArgs),
    /// Write what a learned policy could be trained on. Explicit, and never automatic.
    Dataset(trace::DatasetArgs),
    /// Work out which memories are related.
    Relate(relate::Args),
    /// Follow one search to whatever came of it.
    Trace(trace::TraceArgs),
    /// What a session reported, and how it went.
    Outcomes(trace::OutcomesArgs),
    /// Where a memory's evidence came from, and what that permits.
    Trust(trust::Args),
    /// What using a memory has actually led to.
    Utility(trace::UtilityArgs),
    /// Make this directory the root of its own memory.
    Init(init::Args),
    /// Install the shipped configuration into `$XDG_CONFIG_HOME/aeon`.
    Configs(configs::Args),
    /// Listen for other programs.
    Serve(serve::ServeArgs),
    /// Answer one question, wire-shaped, and exit.
    Api(serve::ApiArgs),
    /// Measure whether memory earns its place: does session k+1 stop rediscovering things.
    Eval(eval::Args),
    /// Print the client library another program loads to talk to aeon.
    #[command(name = "lua-api")]
    LuaApi,
}

/// Run, and answer with what the shell should exit on.
///
/// Errors are printed here rather than returned to `main`, because `Result` from `main` prints
/// the `Debug` of an error and a person reading a terminal wants the `Display`.
#[must_use]
pub fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("aeon: {why:#}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cli: &Cli) -> anyhow::Result<()> {
    if let Some(at) = cli.at {
        CLOCK.store(at, std::sync::atomic::Ordering::Relaxed);
    }
    let cwd = std::env::current_dir()?;
    let mut loaded = if cli.no_config {
        Loaded::bare()
    } else {
        Loaded::read(&cwd)?
    };
    let scope = scope_of(cli, &mut loaded, &cwd);
    let tool = tool_of(cli, &loaded)?;
    let floors = *loaded.settings().floors();
    let where_ = cli.store.as_deref();

    let outcome = match &cli.what {
        None => status::run(where_, &scope, &tool, floors, &loaded),
        Some(What::Remember(args)) => {
            remember::run(where_, &scope, &tool, args, floors, &mut loaded)
        }
        Some(What::Recall(args)) => recall::run(where_, &scope, &tool, args, floors, &mut loaded),
        Some(What::Why(args)) => ask::run(where_, &scope, &tool, args, floors),
        Some(What::Promote(args)) => promote::run(where_, &scope, &tool, args, &mut loaded),
        Some(What::Forget(args)) => forget::run(where_, &scope, &tool, args, &mut loaded),
        Some(What::Context(args)) => context::run(where_, &scope, &tool, args, &mut loaded),
        Some(What::Ingest(args)) => ingest::run(where_, &scope, &tool, args, &mut loaded),
        Some(What::Reindex(args)) => reindex::run(where_, &scope, &tool, args, &loaded),
        Some(What::Replay(args)) => replay::run(where_, &scope, &tool, args),
        Some(What::Sessions(args)) => sessions::run(where_, &scope, &tool, args),
        Some(What::Consolidate(args)) => consolidate::run(where_, &scope, &tool, args, &mut loaded),
        Some(What::Decay(args)) => decay::run(where_, &scope, &tool, args),
        Some(What::Export(args)) => transfer::export(where_, &scope, &tool, args),
        Some(What::Import(args)) => transfer::import(where_, &scope, &tool, args),
        Some(What::Dataset(args)) => trace::dataset(where_, &scope, &tool, args),
        Some(What::Relate(args)) => relate::run(where_, &scope, &tool, args),
        Some(What::Trace(args)) => trace::trace(where_, &scope, &tool, args),
        Some(What::Utility(args)) => trace::utility(where_, &scope, &tool, args),
        Some(What::Trust(args)) => trust::run(where_, &scope, &tool, args),
        Some(What::Outcomes(args)) => trace::outcomes(where_, &scope, &tool, args),
        Some(What::Init(args)) => init::run(args),
        Some(What::Configs(args)) => configs::run(args),
        Some(What::Serve(args)) => serve::serve(where_, &scope, &tool, args, floors, &mut loaded),
        Some(What::Api(args)) => serve::api(where_, &scope, &tool, args, floors, &mut loaded),
        Some(What::Eval(args)) => eval::run(args),
        Some(What::LuaApi) => {
            serve::lua_api();
            Ok(())
        }
    };

    // Anything a `did.` handler said. Printed after the command rather than as it happens,
    // so a configuration cannot interleave itself into the output a person is reading.
    for line in loaded.log() {
        eprintln!("{}", render::dim(&line));
    }
    outcome
}

/// Which memory the flags name.
fn scope_of(cli: &Cli, loaded: &mut Loaded, cwd: &Path) -> aeon_model::ScopeId {
    match cli.scope.as_str() {
        "global" => aeon_model::ScopeId::global(),
        "project" | "." => loaded.scope_of(cwd),
        path => loaded.scope_of(&PathBuf::from(path)),
    }
}

/// Open the scrollback for a scope.
///
/// Beside the memory store and never inside it: a transcript is orders of magnitude larger than
/// the memories distilled from it, and sharing a file would make every recall walk past it.
///
/// `--store` names a memory file directly, so the scrollback goes beside *that* — which is what
/// makes a test or a copy self-contained rather than reaching into the real data directory.
pub(crate) fn scrollback(
    override_path: Option<&Path>,
    scope: &aeon_model::ScopeId,
    tool: &Which,
) -> anyhow::Result<aeon_store::Transcript> {
    let path = match override_path {
        Some(memory) => {
            let stem = memory
                .file_stem()
                .map_or_else(|| "store".to_owned(), |s| s.to_string_lossy().into_owned());
            memory.with_file_name(format!("{stem}-transcript.db"))
        }
        None => {
            home(scope)?;
            aeon_store::transcript_path(scope, &tool.tool)
        }
    };
    Ok(aeon_store::Transcript::open(&path)?)
}

/// The retrieval weighting a configuration asked for.
///
/// Translated here rather than in the store, because the store must not depend on the Lua
/// crate and the configuration must not have to know the store's field order.
#[must_use]
pub(crate) fn weights_of(settings: &aeon_lua::Settings, vectors: bool) -> aeon_store::Weights {
    let said = settings.weights();
    let asked = aeon_store::Weights {
        semantic: said.semantic,
        lexical: said.lexical,
        entity: said.entity,
        frecency: said.frecency,
        confidence: said.confidence,
        strength: said.strength,
        scope: said.scope,
    };
    // With nothing to compare against, the semantic share goes to the lexical one rather than
    // being lost — otherwise every result on an unembedded store would score lower for no
    // reason anybody could see. See `Weights::without_vectors`.
    if vectors {
        asked
    } else {
        asked.without_vectors()
    }
}

/// What `--at` said, or zero for "ask the real clock".
static CLOCK: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// Seconds since the epoch, as everything here counts time.
#[must_use]
pub(crate) fn now() -> aeon_model::Timestamp {
    match CLOCK.load(std::sync::atomic::Ordering::Relaxed) {
        0 => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64),
        overridden => overridden,
    }
}

/// Which tool a command works in, and whether anybody said so.
///
/// The second half is what makes a read different from a write. A write always names one tool,
/// because provenance is the point and "some tool" is not an answer. A read with nothing named
/// searches every tool in the project, because the question is what is known here rather than
/// what one program happened to file.
#[derive(Debug, Clone)]
pub(crate) struct Which {
    /// The tool to write as, and to read from when one was named.
    pub tool: aeon_store::Tool,
    /// Whether a flag or a configuration said which, rather than this being the default.
    pub named: bool,
}

/// Open the store a command should work in.
pub(crate) fn open(
    override_path: Option<&std::path::Path>,
    scope: &aeon_model::ScopeId,
    tool: &Which,
) -> anyhow::Result<aeon_store::Store> {
    let path =
        override_path.map_or_else(|| aeon_store::scope_path(scope, &tool.tool), Path::to_owned);
    if override_path.is_none() {
        home(scope)?;
    }
    Ok(aeon_store::Store::open(&path)?)
}

use std::path::Path;

/// Where a tool's runs keep their own memories.
///
/// Beneath the tool's home, so a run's scratch sits beside the project store it promotes into.
/// `--store` names a file directly and takes its runs with it, which is what makes a test or a
/// copy self-contained rather than reaching into the real data directory.
pub(crate) fn runs_under(
    override_path: Option<&Path>,
    scope: &aeon_model::ScopeId,
    tool: &Which,
) -> PathBuf {
    match override_path {
        Some(memory) => memory.with_extension("runs"),
        None => aeon_store::home_of(scope).join(tool.tool.as_str()),
    }
}

/// Make sure a scope has somewhere to keep its memory.
///
/// Creating the home is a side effect of opening a store rather than a command somebody has to
/// remember to run, and it is idempotent. Scopes with no project — the global one, and any
/// directory that is not a checkout — have nothing to create: the data directory needs no
/// marker and no ignore file.
fn home(scope: &aeon_model::ScopeId) -> anyhow::Result<()> {
    if let Some(at) = aeon_store::project_home(scope) {
        aeon_store::make_home(&at)?;
    }
    Ok(())
}

/// Which tool's memory the flags name.
///
/// Strict about what `--tool` accepts, because a name that had to be rewritten to be usable
/// would put memories somewhere nobody asked for. Socket clients do not come through here —
/// the kernel names them, and `Tool::from_program` salvages what it can from an executable's
/// name because nobody typed that.
fn tool_of(cli: &Cli, loaded: &Loaded) -> anyhow::Result<Which> {
    let said = cli
        .tool
        .as_deref()
        .or_else(|| loaded.settings().tool())
        .map(str::to_owned);
    match said {
        None => Ok(Which {
            tool: aeon_store::Tool::default(),
            named: false,
        }),
        Some(name) => aeon_store::Tool::new(&name)
            .map(|tool| Which { tool, named: true })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "`{name}` cannot name a tool: lowercase letters, digits, `-` and `_`, up to 32"
                )
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_line_is_well_formed() {
        // clap's own audit: duplicate flags, bad defaults, an argument that can never be given.
        Cli::command().debug_assert();
    }

    #[test]
    fn typing_aeon_alone_says_what_is_remembered() {
        let cli = Cli::try_parse_from(["aeon"]).expect("parse");
        assert!(cli.what.is_none());
    }

    #[test]
    fn the_default_scope_is_the_project() {
        // A wrong global fact contaminates every project; a wrong project fact contaminates
        // one. The default goes to the smaller blast radius.
        let cli = Cli::try_parse_from(["aeon"]).expect("parse");
        assert_eq!(cli.scope, "project");
    }
}
