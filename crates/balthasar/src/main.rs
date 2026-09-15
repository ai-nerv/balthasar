//! The binary: multi-call, as the family's are. It registers what exists and does nothing else —
//! everything a subcommand means lives in the crate that means it.

fn main() -> std::process::ExitCode {
    balthasar_cli::main()
}
