//! What an amendment costs on disk, since a streaming message is amended as it grows.
//!
//! Run it deliberately: `cargo test -p balthasar-store --test amendment_cost -- --ignored
//! --nocapture`. It measures rather than asserts, because the number is hardware and the
//! decision it informs — how often a harness may amend — is not balthasar's alone to make.

use balthasar_model::scratch::Scratch;

use balthasar_model::SessionId;
use balthasar_store::{Transcript, Turn};

const NOW: balthasar_model::Timestamp = 1_756_000_000;

#[test]
#[ignore = "takes ~14s: an fsync-per-amendment measurement, not a property"]
fn measure_amendment_throughput() {
    let dir = Scratch::new("balthasar-amend-bench", "one");
    let mut held = Transcript::open(&dir.join("transcript.db")).expect("open");
    let session = SessionId::new("01BENCH");
    held.open_run(&session, "/w/t", "/w/t", "bench", NOW)
        .expect("run");

    let body = "x".repeat(400);
    let start = std::time::Instant::now();
    const N: u64 = 500;
    for n in 0..N {
        held.write(
            &session,
            &Turn {
                cursor: 7,
                at: NOW,
                role: "assistant".into(),
                kind: "prose".into(),
                text: format!("{body}{n}"),
                raw: Some(format!("{{\"grown\":{n}}}")),
                ..Turn::default()
            },
        )
        .expect("amend");
    }
    let took = start.elapsed();
    println!(
        "AMEND {N} writes to one cursor in {:?} — {:.2} ms each, {:.0}/s",
        took,
        took.as_secs_f64() * 1000.0 / N as f64,
        f64::from(u32::try_from(N).unwrap_or(1)) / took.as_secs_f64()
    );
}
