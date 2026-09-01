//! The binary.
//!
//! Multi-call, as the family's are: `memo`, `memo serve`, `memo api`, `memo lua-api`. It
//! registers what exists and does nothing else — everything a subcommand means lives in the
//! crate that means it.

fn main() -> std::process::ExitCode {
    memo_cli::main()
}
