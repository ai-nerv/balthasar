//! Pre-registered evaluations: the manifest lands in a receipt before anything is measured, and
//! what was measured is added beside it afterwards, whichever way it went.

use balthasar_testkit::{Baseline, Manifest};
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};

/// A registered run, waiting for its numbers.
#[derive(Debug)]
pub(crate) struct Receipt {
    path: PathBuf,
    registered: serde_json::Value,
    manifest: Manifest,
}

/// Write `manifest`'s receipt into the existing directory `into` before the run. A manifest
/// registers once: its receipt is never replaced, so a result nobody liked stays where it landed.
pub(crate) fn register(manifest: &Path, into: &Path, at: i64) -> anyhow::Result<Receipt> {
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(manifest)?)?;
    let encoded = serde_json::to_string(&manifest)?;
    let digest = balthasar_model::content_hash(&encoded);
    let name: String = manifest
        .name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let path = into.join(format!(
        "balthasar-receipt-{name}-{}.json",
        digest.get(..12).unwrap_or(&digest)
    ));
    let registered = json!({ "state": "registered", "registered_at": at, "manifest": manifest });
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|why| anyhow::anyhow!("{} is already registered: {why}", path.display()))?;
    file.write_all(serde_json::to_string_pretty(&registered)?.as_bytes())?;
    file.sync_all()?;
    Ok(Receipt {
        path,
        registered,
        manifest,
    })
}

impl Receipt {
    /// Add what was measured: the baseline, and the manifest's own metrics read off it.
    pub(crate) fn record(
        &self,
        baseline: &Baseline,
        attack_success: f64,
    ) -> anyhow::Result<PathBuf> {
        let read = |metric: balthasar_testkit::Metric| json!({ "metric": metric, "value": metric.of(baseline, attack_success) });
        let mut measured = self.registered.clone();
        if let Some(fields) = measured.as_object_mut() {
            fields.insert("state".to_owned(), json!("measured"));
            fields.insert("primary".to_owned(), read(self.manifest.primary));
            fields.insert(
                "guardrails".to_owned(),
                json!(
                    self.manifest
                        .guardrails
                        .iter()
                        .map(|m| read(*m))
                        .collect::<Vec<_>>()
                ),
            );
            fields.insert("baseline".to_owned(), serde_json::to_value(baseline)?);
        }
        let mut fresh = self.path.as_os_str().to_owned();
        fresh.push(".new");
        let fresh = PathBuf::from(fresh);
        std::fs::write(&fresh, serde_json::to_string_pretty(&measured)?)?;
        std::fs::rename(&fresh, &self.path)?;
        Ok(self.path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: i64 = 1_756_000_000;

    fn manifest(dir: &Path) -> PathBuf {
        let path = dir.join("manifest.json");
        let said = json!({
            "name": "decision change", "hypothesis": "nothing closed is stated as current",
            "primary": "assertion_accuracy", "guardrails": ["stale_served", "attack_success"],
            "baseline_policy": "shipped", "policy": "shipped", "scenario": "one-lesson/4",
            "seed": START, "config": "default", "schema": 10, "revision": "test",
            "stop_when": "any stale memory is served"
        });
        std::fs::write(&path, said.to_string()).expect("manifest");
        path
    }

    #[test]
    fn the_manifest_is_on_disk_before_anything_is_measured() {
        let dir = balthasar_model::scratch::Scratch::new("balthasar-receipt", "before");
        std::fs::create_dir_all(dir.join("out")).expect("out");
        let receipt = register(&manifest(&dir), &dir.join("out"), START).expect("registered");
        let held: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&receipt.path).expect("read")).expect("json");
        assert_eq!(held["state"], "registered");
        assert_eq!(held["manifest"]["primary"], "assertion_accuracy");
        assert!(held.get("baseline").is_none(), "nothing measured yet");

        let baseline = Baseline::of(
            &balthasar_testkit::Scenario::one_lesson(4, START),
            "one-lesson",
            START,
            &balthasar_lua::Settings::default(),
        )
        .expect("measured");
        receipt.record(&baseline, 0.0).expect("recorded");
        let held: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&receipt.path).expect("read")).expect("json");
        assert_eq!(held["state"], "measured");
        assert_eq!(
            held["manifest"]["hypothesis"],
            "nothing closed is stated as current"
        );
        assert_eq!(held["guardrails"][0]["metric"], "stale_served");
        assert_eq!(held["guardrails"][0]["value"], 0.0);
    }

    #[test]
    fn a_manifest_registers_once() {
        let dir = balthasar_model::scratch::Scratch::new("balthasar-receipt", "once");
        std::fs::create_dir_all(dir.join("out")).expect("out");
        let manifest = manifest(&dir);
        register(&manifest, &dir.join("out"), START).expect("first");
        assert!(
            register(&manifest, &dir.join("out"), START).is_err(),
            "kept, not replaced"
        );
    }
}
