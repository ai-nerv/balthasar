//! The binary.
//!
//! Multi-call, as the family's are: `aeon`, `aeon serve`, `aeon api`, `aeon lua-api`. It
//! registers what exists and does nothing else — everything a subcommand means lives in the
//! crate that means it.

fn main() -> std::process::ExitCode {
    aeon_cli::main()
}
