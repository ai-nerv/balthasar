//! Stamp the checkout into the binary, for evaluation artifacts.
//!
//! Best effort: a build with no repository stamps `unknown` rather than failing.

fn main() {
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-env-changed=MAGI_BALTHASAR_GIT_REV");

    if std::env::var_os("MAGI_BALTHASAR_GIT_REV").is_some() {
        return;
    }
    let revision = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty());

    if let Some(revision) = revision {
        println!("cargo:rustc-env=MAGI_BALTHASAR_GIT_REV={revision}");
    }
}
