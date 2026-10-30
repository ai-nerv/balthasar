# Candle ZIP compatibility backport

`candle-core` is the published 0.9.2 source, with its MIT/Apache-2.0 license retained.
Archive: https://static.crates.io/crates/candle-core/candle-core-0.9.2.crate
SHA256: `c15b675b80d994b2eadb20a4bbe434eabeb454eac3ee5e2b4cf6f147ee9be091`.
Upstream commit: `3b39794c14b1de8ba4c2d10d49354d557a415547`.

The normalized manifest requests ZIP >=9.0.5. NPZ and PTH loading use fallible
borrowed/owned filenames. This follows
[upstream PR4041](https://github.com/huggingface/candle/pull/4041), still unmerged
when inspected on2026-10-20. A private lookup helper additionally preserves decoded
CP437 lookup for named NPZ and PTH metadata/storage: ZIP9 indexes raw name bytes,
so a decoded name cannot always be passed directly to its by_name. UTF8 lookup
keeps the direct index path; fallback scans existing archive metadata only.
CPU/GPU math and all public Candle APIs are unchanged.
`Cargo.toml.orig` records the original published manifest, not the patched one.
The cache marker and upstream development lockfile are omitted; the Balthasar
root lockfile is authoritative. NN and Transformers remain the registry0.9.2 crates.

`oslo make test-dense` exercises archive compatibility and a locally generated
small BERT through the real embedder without downloading trained weights.
`oslo make audit` and `oslo make deny` keep their existing policies. No ZIP advisory
is ignored. Remove this patch when an API-compatible upstream release includes
ZIP9, after the same dense/archive and default verification gates pass.
