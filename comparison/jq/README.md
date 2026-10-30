# jq vs succinctly jq

Inputs for `succinctly dev compare jq` (#5409). The harness never uses the `jq` on `PATH`.

```bash
scripts/compare-jq-build.sh 1.7.1                     # sha256-verified source build into target/compare/
cargo build --release --features cli,simd,regex,serde,bench-runner
./target/release/succinctly dev compare jq --quick    # or --full; --build-jq builds the reference if missing
```

`--ledger-only` runs just the probes `divergences.toml` lists for this OS and exits 1 on an entry that no longer
diverges: the stale half of the `Ledger drift` CI job in about a minute, without the 1.8.1 build (#5585). With no pinned
1.7.1 build it uses `/usr/bin/jq` when that reports `jq-1.7.1` or `jq-1.7.1-<vendor>` (macOS: `jq-1.7.1-apple`) and says
so, because a vendor build can disagree with the pinned one on libm-dependent entries. It skips `since-secondary` entries,
entries for another OS and the perf and operational dimensions, names each skip, and cannot find an unlisted divergence:
that half stays in CI.

Writes `env.json`, `results.json` (schema-versioned) and `report.md` to `data/compare/<run_id>/`.
A dirty tree, battery power, a non-release build or a feature set that differs from `manifest.toml`
marks the run **INVALID** in the report. A query whose output or exit status differs from jq's is
reported as a correctness row and gets no timing; the command exits 1 on it.

| File                  | Role                                                     |
|-----------------------|----------------------------------------------------------|
| `manifest.toml`       | pinned references (version, source sha256), build, queries |
| `queries/smoke.tsv`   | `id`, flags, filter, input — data, not code              |
| `probes/syntax.tsv`   | A: syntax forms, same columns as `cli.tsv`; run beside the builtins |
| `probes/cli.tsv`      | G: `id`, area, args (JSON array), stdin (JSON string)    |
| `probes/extensions.tsv` | succinctly-only surface the reference must reject      |
| `fixtures/`           | working directory of the CLI probes (`-f`, `-L`, ...)    |
| `vendor/jq-1.7.1/`    | B: jq's own test files (MIT), sha256-pinned in the manifest |
| `queries/perf.tsv`    | D/E: id, flags, filter, shapes, max tier                 |
| `probes/robust.tsv`   | F: nesting, recursion, generators, regex, big values, SIGPIPE; `@gen:` builds large stdin |
| `divergences.toml`    | ledger of known differences; the run fails on drift      |

Dimension A probes every builtin in the reference's own `builtins` (plus those new in jq 1.8.1, against 1.8.1) and, from `probes/syntax.tsv`, the language's forms: operators, paths and slices, construction, `if`/`reduce`/`foreach`/`label`, destructuring with `?//`, `def`, string and format forms, assignment and `import`/`include`;
`--write-summary` regenerates `docs/compare/jq-coverage.json` and the figures in `docs/reference/jq-language.md`.
Dimension B runs the vendored upstream test files through both tools; C runs JSONTestSuite against the reference, `succinctly jq` and `--validate`.
Dimensions D and E time every query on the pinned generated inputs (`[[input]]`, regenerated into `data/compare/inputs/`
and checked against their sha256) against jq 1.7.1 and 1.8.1, interleaved, gated on output identity; `--control` times jq
against itself to show the noise floor, and `--full` adds the 10 MB and 100 MB tiers.
A row whose slowest tool runs over 15 ms is topped up to 21 repetitions (within 30 s of extra timed runs per row) and, once it has them, its
ratios are of each tool's fastest run, marked † in the report, because run-to-run noise moves the median of a figure that
long (#5581). A row with a CV above 5% is marked, and its ratio is not a finding on its own.
Dimension F runs 85 resource-limit probes under a 10 s deadline and classes each outcome (ok, error, crash, timeout).

Dimension H reports binary size, dynamic libraries, the Rust dependency count, whether either tool uses more than one
core, and licence / `no_std` / CI-runner facts read from the repository; build time is not measured.

```bash
# Re-run the deterministic dimensions and spot-check 5 timing rows against a recorded run (exit 1 on any difference)
./target/release/succinctly dev compare jq --verify docs/compare/jq-<platform>.json
# Regenerate the committed report from a complete, valid run on a clean checkout (refused otherwise)
./target/release/succinctly dev compare jq --quick --write-docs
```

The harness needs `bench-runner`, which the shipped CLI does not carry (`bench-runner` pulls in `ctrlc`, which links Foundation on macOS, #5551).
To time the shipped build instead, build it separately and pass it in; the report records that the timed binary is not the harness:

```bash
cargo build --release --features cli,simd,regex,serde --target-dir target/subject
./target/release/succinctly dev compare jq --quick --subject target/subject/release/succinctly --subject-features cli,simd,regex,serde
```

`--write-docs` writes `docs/compare/jq-<platform>.{md,json}` and the index `docs/compare/jq.md`. CI is
`.github/workflows/compare-jq.yml`, run by hand or on a release, not on pull requests.
`.github/workflows/compare-jq-ledger.yml` runs the correctness dimensions (no timing rows) on pull requests that touch
`comparison/jq/`, `src/jq/` or the harness, and weekly, and fails on an unlisted or stale ledger entry (#5565).
