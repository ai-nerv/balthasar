//! What `balthasar` does when you type it.

mod ask;
mod configs;
mod consolidate;
mod context;
mod coordinated;
mod decay;
pub(crate) mod distil;
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
mod train;
mod transfer;
mod trust;

use clap::{Parser, Subcommand};
use loaded::Loaded;
use std::path::PathBuf;
use std::process::ExitCode;

/// Memory for agents.
#[derive(Debug, Parser)]
#[command(name = "balthasar", version, about, disable_help_subcommand = true)]
struct Cli {
    /// Which memory to work in: `global`, `project`, or a path.
    #[arg(long, global = true, default_value = "project")]
    scope: String,

    /// Which tool the memory belongs to.
    #[arg(long, global = true, value_name = "NAME")]
    tool: Option<String>,

    /// Work in a store somewhere else. For tests, and for looking at a copy.
    #[arg(long, global = true, value_name = "FILE")]
    store: Option<PathBuf>,

    /// Ignore every configuration file and use the shipped defaults.
    #[arg(long, global = true)]
    no_config: bool,

    /// Treat this unix time as now.
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

    /// Run the extractors over what this project's own runs said.
    Distil(distil::Args),
    /// Give memories their vectors. Never on the critical path.
    Reindex(reindex::Args),
    /// Everything a run said, back out again.
    Replay(replay::Args),
    /// Which runs this project has had, and what each left behind.
    Sessions(sessions::Args),
    /// What a coordinator may tell this balthasar.
    Needs(coordinated::NeedsArgs),
    /// Take configuration from a coordinator, as Lua on stdin.
    Configure(coordinated::ConfigureArgs),
    /// Acknowledge the installed packages, so their declarations may run.
    Acknowledge(coordinated::AcknowledgeArgs),
    /// Carry what recurred across sessions into the project's memory. Shows first.
    Consolidate(consolidate::Args),
    /// Fade what has not been needed. Shows first; `--now` applies.
    Decay(decay::Args),
    /// Every memory, one JSON object per line.
    Export(transfer::ExportArgs),
    /// Read back what `export` wrote.
    Import(transfer::ImportArgs),
    /// Fit the ranking policy from this store's ledger, and say whether to believe it.
    Train(train::Args),
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
    /// Install the shipped configuration into `$XDG_CONFIG_HOME/balthasar`.
    Configs(configs::Args),
    /// Listen for other programs.
    Serve(serve::ServeArgs),
    /// Answer one question, wire-shaped, and exit.
    Api(serve::ApiArgs),
    /// Measure whether memory earns its place: does session k+1 stop rediscovering things.
    Eval(eval::Args),
    /// Print the client library another program loads to talk to balthasar.
    #[command(name = "lua-api", alias = "client")]
    LuaApi(serve::ClientArgs),
    /// Every verb this program answers, on each of its doors.
    Verbs(coordinated::VerbsArgs),
}

/// Run, and answer with what the shell should exit on.
#[must_use]
pub fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        // A verb balthasar does not have is a refusal like any other: the reply shape, on stdout,
        // at exit zero. A parser's usage on stderr cannot be told from a binary that is not there.
        Err(why) if why.kind() == clap::error::ErrorKind::InvalidSubcommand => {
            return no_such_call(&why);
        }
        Err(why) => why.exit(),
    };
    match dispatch(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("balthasar: {why:#}");
            ExitCode::FAILURE
        }
    }
}

/// Refuse a verb that does not exist, naming what was asked for.
fn no_such_call(why: &clap::Error) -> ExitCode {
    let asked = match why.get(clap::error::ContextKind::InvalidSubcommand) {
        Some(clap::error::ContextValue::String(name)) => name.clone(),
        _ => String::new(),
    };
    let mut out = std::io::stdout().lock();
    coordinated::emit(
        &mut out,
        render::How::default(),
        &balthasar_ipc::Reply::refused(format!("no such call: {asked}")),
    );
    ExitCode::SUCCESS
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
        Some(What::Distil(args)) => distil::run(where_, &scope, &tool, args, &mut loaded),
        Some(What::Context(args)) => context::run(where_, &scope, &tool, args, &mut loaded),
        Some(What::Ingest(args)) => ingest::run(where_, &scope, &tool, args, &mut loaded),
        Some(What::Reindex(args)) => reindex::run(where_, &scope, &tool, args, &loaded),
        Some(What::Replay(args)) => replay::run(where_, &scope, &tool, args),
        Some(What::Sessions(args)) => sessions::run(where_, &scope, &tool, args),
        Some(What::Needs(args)) => coordinated::needs(args),
        Some(What::Configure(args)) => coordinated::configure(args),
        Some(What::Acknowledge(args)) => coordinated::acknowledge(args),
        Some(What::Consolidate(args)) => consolidate::run(where_, &scope, &tool, args, &mut loaded),
        Some(What::Decay(args)) => decay::run(where_, &scope, &tool, args),
        Some(What::Export(args)) => transfer::export(where_, &scope, &tool, args),
        Some(What::Import(args)) => transfer::import(where_, &scope, &tool, args),
        Some(What::Train(args)) => train::run(where_, &scope, &tool, args),
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
        Some(What::Verbs(args)) => coordinated::verbs(args),
        Some(What::LuaApi(args)) => {
            serve::lua_api(args);
            Ok(())
        }
    };

    for line in loaded.log() {
        eprintln!("{}", render::dim(&line));
    }
    outcome
}

fn scope_of(cli: &Cli, loaded: &mut Loaded, cwd: &Path) -> balthasar_model::ScopeId {
    match cli.scope.as_str() {
        "global" => balthasar_model::ScopeId::global(),
        "project" | "." => loaded.scope_of(cwd),
        path => loaded.scope_of(&PathBuf::from(path)),
    }
}

/// Open the scrollback for a scope.
///
/// Beside the memory store and never inside it; `--store` puts it beside that file instead.
pub(crate) fn scrollback(
    override_path: Option<&Path>,
    scope: &balthasar_model::ScopeId,
    tool: &Which,
) -> anyhow::Result<balthasar_store::Transcript> {
    let path = match override_path {
        Some(memory) => {
            let stem = memory
                .file_stem()
                .map_or_else(|| "store".to_owned(), |s| s.to_string_lossy().into_owned());
            memory.with_file_name(format!("{stem}-transcript.db"))
        }
        None => {
            home(scope)?;
            balthasar_store::transcript_path(scope, &tool.tool)
        }
    };
    Ok(balthasar_store::Transcript::open(&path)?)
}

/// The retrieval weighting a configuration asked for.
#[must_use]
pub(crate) fn weights_of(
    settings: &balthasar_lua::Settings,
    vectors: bool,
) -> balthasar_store::Weights {
    let said = settings.weights();
    let asked = balthasar_store::Weights {
        semantic: said.semantic,
        lexical: said.lexical,
        entity: said.entity,
        frecency: said.frecency,
        confidence: said.confidence,
        strength: said.strength,
        scope: said.scope,
    };
    // With nothing to compare against, the semantic share goes to the lexical one rather than
    // being lost. See `Weights::without_vectors`.
    if vectors {
        asked
    } else {
        asked.without_vectors()
    }
}

/// What `--at` said, or zero for "ask the real clock".
static CLOCK: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

#[must_use]
pub(crate) fn now() -> balthasar_model::Timestamp {
    match CLOCK.load(std::sync::atomic::Ordering::Relaxed) {
        0 => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64),
        overridden => overridden,
    }
}

/// Which tool a command works in, and whether anybody said so.
///
/// A read with nothing named searches every tool in the project; a write always names one.
#[derive(Debug, Clone)]
pub(crate) struct Which {
    /// The tool to write as, and to read from when one was named.
    pub tool: balthasar_store::Tool,
    /// Whether a flag or a configuration said which, rather than this being the default.
    pub named: bool,
}

/// Open the store a command should work in.
pub(crate) fn open(
    override_path: Option<&std::path::Path>,
    scope: &balthasar_model::ScopeId,
    tool: &Which,
) -> anyhow::Result<balthasar_store::Store> {
    let path = override_path.map_or_else(
        || balthasar_store::scope_path(scope, &tool.tool),
        Path::to_owned,
    );
    if override_path.is_none() {
        home(scope)?;
    }
    Ok(balthasar_store::Store::open(&path)?)
}

use std::path::Path;

/// Where a tool's runs keep their own memories.
pub(crate) fn runs_under(
    override_path: Option<&Path>,
    scope: &balthasar_model::ScopeId,
    tool: &Which,
) -> PathBuf {
    match override_path {
        Some(memory) => memory.with_extension("runs"),
        None => balthasar_store::home_of(scope).join(tool.tool.as_str()),
    }
}

/// Make sure a scope has somewhere to keep its memory.
fn home(scope: &balthasar_model::ScopeId) -> anyhow::Result<()> {
    if let Some(at) = balthasar_store::project_home(scope) {
        balthasar_store::make_home(&at)?;
    }
    Ok(())
}

/// Which tool's memory the flags name.
fn tool_of(cli: &Cli, loaded: &Loaded) -> anyhow::Result<Which> {
    let said = cli
        .tool
        .as_deref()
        .or_else(|| loaded.settings().tool())
        .map(str::to_owned);
    match said {
        None => Ok(Which {
            tool: balthasar_store::Tool::default(),
            named: false,
        }),
        Some(name) => balthasar_store::Tool::new(&name)
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
        Cli::command().debug_assert();
    }

    #[test]
    fn typing_balthasar_alone_says_what_is_remembered() {
        let cli = Cli::try_parse_from(["balthasar"]).expect("parse");
        assert!(cli.what.is_none());
    }

    #[test]
    fn the_default_scope_is_the_project() {
        let cli = Cli::try_parse_from(["balthasar"]).expect("parse");
        assert_eq!(cli.scope, "project");
    }
}
