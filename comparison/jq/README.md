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
| `probes/cli.tsv`      | G: `id`, area, args (JSON array), stdin (JSON string), optional `env` |
| `probes/extensions.tsv` | succinctly-only surface the reference must reject      |
| `fixtures/`           | working directory of the CLI probes (`-f`, `-L`, ...)    |
| `vendor/jq-1.7.1/`    | B: jq's own test files (MIT), sha256-pinned in the manifest |
| `vendor/jq-1.8.1/`    | B2: jq 1.8.1's own test files, run against 1.8.1 alone and reported apart (informational, never a ledger entry) |
| `queries/perf.tsv`    | D/E: id, flags, filter, shapes, max tier; `@INPUT@` in the flags is the input file's name (`--slurpfile d @INPUT@`), `@files=N` passes it `N` times |
| `probes/robust.tsv`   | F: nesting, recursion, generators, regex, big values, SIGPIPE, `ulimit -v` (`ulimit-v:<KiB>` wrap); `@gen:` builds large stdin |
| `divergences.toml`    | ledger of known differences; the run fails on drift      |

The `env` column of `probes/cli.tsv` (and `syntax.tsv`) is a JSON object of variables set for both tools (`{"JQ_COLORS":"1;31"}`, `{"LC_ALL":"","LANG":"de_DE.UTF-8"}`). The base environment is `LC_ALL=C` with `JQ_COLORS`, `NO_COLOR` and every other locale variable removed, so a probe sees only what its row sets (#5552). A locale the host lacks makes both tools fall back to `C`, so the `de_DE`/`tr_TR` rows only mean something on a host that has them (macOS; the Linux runners carry `C.utf8` and `en_US`).

Dimension A probes every builtin in the reference's own `builtins` (plus those new in jq 1.8.1, against 1.8.1) and, from `probes/syntax.tsv`, the language's forms: operators, paths and slices, construction, `if`/`reduce`/`foreach`/`label`, destructuring with `?//`, `def`, string and format forms, assignment and `import`/`include`;
`--write-summary` regenerates `docs/compare/jq-coverage.json` and the figures in `docs/reference/jq-language.md`.
Dimension B runs the vendored upstream test files through both tools; C runs JSONTestSuite against the reference, `succinctly jq` and `--validate`.
Besides the generated shapes, seven `[[input]]` entries name a JSON file of the benchmark corpus (`corpus = "json/openapi/swagger-v2-schema.json"`, #5552): the harness copies it from `tests/data/bench-corpus/seed/` or from `data/bench/corpus/` (`scripts/sync-bench-corpus.sh` fetches the four too large to commit), refuses it when its sha256 is not the one the manifest pins, and times it like any other input. Each file is its own shape (`corpus-<workload>`), so `perf.tsv` names the queries that fit it. A fetched file that is not on the machine drops its rows, and the report says which. The files are 0.5 KB to 0.8 MB, so all but `corpus-pretty` are bound by process start-up, not by parsing: read those ratios as start-up cost on a real document, not as throughput.
The performance shapes beyond one large document are `ndjson` (one small record per line), `jsonseq` (the same records behind RS bytes, for `--seq`), several input files (`@files=N`) and `--slurpfile`/`--rawfile` loading (#5622).
Dimensions D and E time every query on the pinned generated inputs (`[[input]]`, regenerated into `data/compare/inputs/`
and checked against their sha256) against jq 1.7.1 and 1.8.1, interleaved, gated on output identity; `--control` times jq
against itself to show the noise floor, and `--full` adds the 10 MB and 100 MB tiers.
`--cachegrind` (Linux with `valgrind` installed; an error elsewhere) also runs each tool once per row under `valgrind --tool=cachegrind --cache-sim=no` and reports `I refs` beside the times (#5552). An instruction count does not move with machine load, so it is a noise-free second signal, and the report marks a row where it and the wall-clock put succinctly on opposite sides of jq (both more than 5% from a tie): the two can disagree, and a start-up-bound row on a small file does. Only rows up to 1 MB are counted, since cachegrind runs ~50x slower, and only succinctly and jq 1.7.1; valgrind emulates a CPU without AVX-512, so the instruction count can describe a different code path than the timed run on a machine that has it. CI passes the flag on both Linux runners.
A row whose slowest tool runs over 15 ms is topped up to 21 repetitions (within 30 s of extra timed runs per row) and, once it has them, its
ratios are of each tool's fastest run, marked † in the report, because run-to-run noise moves the median of a figure that
long (#5581). A row with a CV above 5% is marked, and its ratio is not a finding on its own.
Dimension F runs the resource-limit probes under a 10 s deadline and classes each outcome (ok, error, crash, timeout).
Its `memory` probes cap the address space with `ulimit -v` (graceful error versus abort; early-exit `first`/`limit`/`isempty` on a generated array); macOS refuses `ulimit -v`, so there they run uncapped as a no-op and only Linux CI exercises the limit.

Dimension H reports binary size, dynamic libraries, the Rust dependency count, whether either tool uses more than one
core, and licence / `no_std` / CI-runner facts read from the repository. `--build-time` also times a cold build of each pinned jq (`configure` + `make` + `make install`, the tarball download not on the clock) and of succinctly (`cargo build --release --locked` into a scratch target directory after an untimed `cargo fetch`), and the rebuild of succinctly after touching `src/lib.rs`: one run each in `target/compare/build-time/`, which is removed afterwards. It takes minutes, and a figure that long moves with the disk cache and load, so the report gives a magnitude, not a ratio.

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
