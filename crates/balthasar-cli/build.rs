//! Embedding the shipped configuration.
//!
//! Walked rather than listed: a new file in `config/` needs no list adding to, and no `.rs` file
//! names a harness, which `gate-independent` refuses.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config");
    println!("cargo:rerun-if-changed={}", config.display());

    let mut found = Vec::new();
    walk(&config, &config, &mut found);
    found.sort();

    let mut out = String::from(
        "/// Every file in `config/`, embedded at build time.\n\
         pub const SHIPPED: &[(&str, &str)] = &[\n",
    );
    for (name, path) in &found {
        writeln!(out, "    ({name:?}, include_str!({path:?})),").expect("a string");
        println!("cargo:rerun-if-changed={path}");
    }
    out.push_str("];\n");

    let into = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("shipped.rs");
    std::fs::write(&into, out).expect("write the shipped list");
}

/// Every `.lua` under `root`, as (path relative to root, absolute path).
fn walk(root: &Path, at: &Path, found: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(at) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, found);
        } else if path.extension().is_some_and(|e| e == "lua") {
            let name = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            found.push((name, path.to_string_lossy().into_owned()));
        }
    }
}
