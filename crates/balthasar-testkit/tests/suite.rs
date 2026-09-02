//! The scenario suite, run.

use balthasar_testkit::{Category, run_suite};

#[test]
fn the_suite_runs_and_reports() {
    let report = run_suite();
    println!(
        "\n  probes {}  rate {:.0}%\n",
        report.probes,
        report.rate() * 100.0
    );
    for category in Category::all() {
        let (held, broke) = report.of(category);
        if held + broke == 0 {
            continue;
        }
        println!(
            "  {:<18} {}/{}{}",
            category.as_str(),
            held,
            held + broke,
            if broke > 0 { "  <-" } else { "" }
        );
    }
    if !report.failures.is_empty() {
        println!("\n  failures:");
        for f in &report.failures {
            println!("    [{}] {}", f.category.as_str(), f.case);
            println!("      asked   {}", f.asks);
            println!("      wanted  {}", f.why);
            println!("      got     {}", f.got);
        }
    }
    println!();
    assert!(report.probes > 0);
}
