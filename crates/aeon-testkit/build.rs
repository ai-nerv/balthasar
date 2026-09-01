//! Stamp the checkout into the binary, for evaluation artifacts.
//!
//! A benchmark number that cannot name the revision that produced it cannot be compared against
//! a later one. Best effort on purpose: a build from a tarball has no repository, and that is
//! `unknown` rather than a failure.

fn main() {
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-env-changed=AEON_GIT_REV");

    if std::env::var_os("AEON_GIT_REV").is_some() {
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
        println!("cargo:rustc-env=AEON_GIT_REV={revision}");
    }
}
