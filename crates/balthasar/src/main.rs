//! The binary.
//!
//! Multi-call, as the family's are: `balthasar`, `balthasar serve`, `balthasar api`, `balthasar lua-api`. It
//! registers what exists and does nothing else — everything a subcommand means lives in the
//! crate that means it.

fn main() -> std::process::ExitCode {
    balthasar_cli::main()
}
