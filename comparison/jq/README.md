# jq vs succinctly jq

Inputs for `succinctly dev compare jq` (#5409). The harness never uses the `jq` on `PATH`.

```bash
scripts/compare-jq-build.sh 1.7.1                     # sha256-verified source build into target/compare/
cargo build --release --features cli,simd,regex,serde,bench-runner
./target/release/succinctly dev compare jq --quick    # or --full; --build-jq builds the reference if missing
```

Writes `env.json`, `results.json` (schema-versioned) and `report.md` to `data/compare/<run_id>/`.
A dirty tree, battery power, a non-release build or a feature set that differs from `manifest.toml`
marks the run **INVALID** in the report. A query whose output or exit status differs from jq's is
reported as a correctness row and gets no timing; the command exits 1 on it.

| File                  | Role                                                     |
|-----------------------|----------------------------------------------------------|
| `manifest.toml`       | pinned references (version, source sha256), build, queries |
| `queries/smoke.tsv`   | `id`, flags, filter, input — data, not code              |

Only the `smoke` suite exists so far. The report lists dimensions A–H as not implemented, by phase.
