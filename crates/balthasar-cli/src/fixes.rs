//! Every fix a health check names is a command this binary parses.

use clap::Parser;

/// `text` split the way a shell would, single quotes only.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut held = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '\'' => quoted = !quoted,
            ' ' if !quoted => {
                if !held.is_empty() {
                    out.push(std::mem::take(&mut held));
                }
            }
            _ => held.push(c),
        }
    }
    if !held.is_empty() {
        out.push(held);
    }
    out
}

#[test]
fn every_fix_a_health_check_names_parses() {
    for fix in balthasar_store::FIXES {
        let argv = words(fix);
        assert!(
            super::Cli::try_parse_from(&argv).is_ok(),
            "`{fix}` is not a command balthasar parses"
        );
    }
}
