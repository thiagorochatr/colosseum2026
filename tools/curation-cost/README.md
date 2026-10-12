# curation-cost

Measures what a clean-set policy costs a pool's anonymity set. It applies curation policies to the depositors in mirror-pool's committed mainnet samples, then recomputes the funding-provenance metrics over each clean set with `mirror-provenance` itself. Results and how to read them: [docs/ANONYMITY_COST.md](../../docs/ANONYMITY_COST.md).

```
cargo run --release --offline --manifest-path tools/curation-cost/Cargo.toml
```

Run it from the repository root. It prints the tables for `mirror-pool/data/sample-privacycash-run6.json` and `mirror-pool/data/sample-marinade-run8.json`. To measure other samples, pass their paths as arguments. `--offline` works once the dependencies are in cargo's cache. On a fresh machine, leave it out for the first build: the analysis itself makes no network call.

- **Pure and deterministic.** It reads only the committed samples. The seeds are fixed (mirror-provenance's published `DEFAULT_SEED`, plus fixed per-row streams derived from it), so a rerun prints the same bytes.
- **Checked against the published run.** Before the table, it prints P0 through the crate's own `Quotation` renderer, the one `mirror analyze` uses, and checks it against `mirror_provenance::PUBLISHED_HEADLINE`.
- **The same bootstrap as the crate.** `cargo test` checks that the local resampler reproduces `mirror_provenance::loss_factor_interval` bit for bit.
- **Standalone.** It depends on `mirror-pool/crates/mirror-provenance` by path, the way `tools/vectors` depends on `mirror-core`, and leaves the mirror-pool tree untouched. `Cargo.lock` was seeded from `mirror-pool/Cargo.lock`.
