# jq vs succinctly jq

Inputs for `succinctly dev compare jq` (#5409). The harness never uses the `jq` on `PATH`.

```bash
scripts/compare-jq-build.sh 1.7.1                     # sha256-verified source build into target/compare/
cargo build --release --features cli,simd,regex,serde,bench-runner
./target/release/succinctly dev compare jq --quick    # or --full; --build-jq builds the reference if missing
```

`--ledger-only` runs just the probes the ledger (`divergences/`) lists for this OS and exits 1 on an entry that no longer
diverges: the stale half of the `Ledger drift` CI job in about a minute, without the 1.8.2 build (#5585). With no pinned
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
| `vendor/jq-1.8.2/`    | B2: jq 1.8.2's own test files, run against 1.8.2 alone and reported apart (informational, never a ledger entry) |
| `queries/perf.tsv`    | D/E: id, flags, filter, shapes, max tier (`1mb`, `10mb`, `100mb`, or `1gb` for `--huge`); `@INPUT@` in the flags is the input file's name (`--slurpfile d @INPUT@`), `@files=N` passes it `N` times, `@stdin` pipes it on stdin (#6914) |
| `probes/robust.tsv`   | F: nesting, recursion, generators, regex, big values, SIGPIPE, `ulimit -v` (`ulimit-v:<KiB>` wrap); `@gen:` builds large stdin |
| `divergences/`        | ledger of known differences, one file per entry (`<suite>/<probe>.toml`, see its README); the run fails on drift |

The `env` column of `probes/cli.tsv` (and `syntax.tsv`) is a JSON object of variables set for both tools (`{"JQ_COLORS":"1;31"}`, `{"LC_ALL":"","LANG":"de_DE.UTF-8"}`). The base environment is `LC_ALL=C` with `JQ_COLORS`, `NO_COLOR` and every other locale variable removed, so a probe sees only what its row sets (#5552). A locale the host lacks makes both tools fall back to `C`, so the `de_DE`/`tr_TR` rows only mean something on a host that has them (macOS; the Linux runners carry `C.utf8` and `en_US`).

Dimension A probes every builtin in the reference's own `builtins` (plus those new in jq 1.8.2, against 1.8.2) and, from `probes/syntax.tsv`, the language's forms: operators, paths and slices, construction, `if`/`reduce`/`foreach`/`label`, destructuring with `?//`, `def`, string and format forms, assignment and `import`/`include`;
The syntax file also holds six `manual-*` areas (types, conditionals, advanced, math, io, assignment): the behaviours the jq 1.7.1 manual states in prose beyond the example programs `upstream-man.test` runs (#6007). Their expected output is whatever the pinned jq prints, never the manual's text.
`--write-summary` regenerates `docs/compare/jq-coverage.json` and the figures in `docs/reference/jq-language.md`.
Dimension B runs the vendored upstream test files through both tools, and the `fuzz` dimension drives `scripts/jq-grammar-fuzz.py` (#5696, #5552): programs and documents from a typed grammar under the fixed seeds and program count in `manifest.toml`'s `[fuzz]`, compared on exit status, stdout and the fault's class, each divergence minimized. A finding is ledgered as `fuzz:p-<12 hex digits>` (the SHA-256 of its minimized program, since an id is a file name: `divergences/fuzz/p-<digits>.toml`; the report prints the id, and the entry's `reason` names the program). It runs the same programs under `--quick`, needs `python3`, and is not part of `--ledger-only` (an id holds a hash of the program, not the document it was found on, so the run cannot be replayed from the ledger); the `Compare jq ledger` workflow runs it. Changing the grammar, a seed or the count moves what is covered, so the ids of the ledgered findings are re-found from a fresh run. The id is the smallest program of its divergence, so a fix that changes which program is smallest re-keys the entry (the run names both the unlisted and the stale id), and a program that times out under heavy load (2 s per tool) is skipped, not counted as agreeing, so a ledgered finding can read as stale on a starved runner. C runs JSONTestSuite against the reference, `succinctly jq` and `--validate`.
The `sweeps` dimension runs the `scripts/jq-*-sweep.*` oracle sweeps listed in `manifest.toml`'s `[[sweep]]` (#5552): each runs its own grid through the harness's subject and reference (`{subject}` and `{jq}` in its arguments; `JQ_ORACLE` and `JQ` are set too) and judges its own rows, including the residuals it expects, so the runner reads only the exit status: 0 agrees, 1 diverges (ledgered as `sweep:<id>`), and any other exit, a Python traceback or the sweep's `timeout_s` is an error. A folded script therefore exits 2 on a setup error, and the shell sweeps take `JQ_ORACLE` before `/usr/bin/jq`. A sweep can be replayed from its id, so `--ledger-only` runs the listed ones. The same arguments run under `--quick`.
Twenty-seven of the 28 are folded, in 28 entries (`reduce-update-bind` runs both of its grids); `docs/guides/developer.md`'s "Differential sweeps against jq" lists each script, what it gates and how it is folded. Together they take about 25 minutes on an M4 Pro.
Nine judge a change against `--base`; folded, they run with `--jq-gate`, which fails on a wrong answer against jq alone, since none documents a residual for it (every folded row matches jq 1.7.1 on `main` at #5552), and `assign-path`, a recording, runs with `--gate` for the same. `foreach-update-state-register` and `nested-fold-source` take the two builds as positional arguments, so they are given the subject as both.
Five grids are too large to run whole and are sampled under a fixed seed: `fold-body-register-grid` (1.2M rows), `fold-source-register`, `fold-update-alt-bind` (6.9M) and `path-register` (63.6M) draw the sample while the grid is generated, so a run holds the sample, not the grid (listing `path-register`'s took about 15 GB), and `reduce-update-bind` as before.
`assign-path` and `path-context` fold their jq-mode half only (`--jq-only`): the harness has no yq reference.
`fold-body-message` is not folded: its verdict is an error message that changed against `--base`, and against jq alone its rows differ only in wording where they differ at all, while `fold-body-register-grid` gates the same rows' output and exit code.
Besides the generated shapes, seven `[[input]]` entries name a JSON file of the benchmark corpus (`corpus = "json/openapi/swagger-v2-schema.json"`, #5552): the harness copies it from `tests/data/bench-corpus/seed/` or from `data/bench/corpus/` (`scripts/sync-bench-corpus.sh` fetches the four too large to commit), refuses it when its sha256 is not the one the manifest pins, and times it like any other input. Each file is its own shape (`corpus-<workload>`), so `perf.tsv` names the queries that fit it. A fetched file that is not on the machine drops its rows, and the report says which. The files are 0.5 KB to 0.8 MB, so all but `corpus-pretty` are bound by process start-up, not by parsing: read those ratios as start-up cost on a real document, not as throughput.
The performance shapes beyond one large document are `ndjson` (one small record per line), `jsonseq` (the same records behind RS bytes, for `--seq`), `jsonseq-bad-first` and `jsonseq-bad-last` (the same stream with one truncated record at its start or end, so `--seq` runs behind a diagnostic, #6619), several input files (`@files=N`) and `--slurpfile`/`--rawfile` loading (#5622).
A query whose flags carry `@stdin` passes no file: the harness reads the input once and writes it to the tool's standard input through a pipe (`Stdio::piped()`, so the tool sees a FIFO, never a redirected regular file), in the output gate, the timed runs, the peak-memory runs and the cachegrind run alike (#6914). Its id is the file query's with `_stdin` after it (`identity_stdin` runs the program of `identity`; a test checks the two agree), and its rows go to `perf_stdin` in `results.json`, not `perf`, so no headline geomean over the file route moves. The report's "Piped stdin vs file argument" section sets each stdin row beside its file twin (time and memory, per tool, a ratio over 1.25x marked). The writer is a thread feeding from memory, not a real producer: it measures the pipe route of each tool, without a network or a slow upstream.
Dimensions D and E time every query on the pinned generated inputs (`[[input]]`, regenerated into `data/compare/inputs/`
and checked against their sha256) against jq 1.7.1 and 1.8.2, interleaved, gated on output identity; `--control` times jq
against itself to show the noise floor, `--full` adds the 10 MB and 100 MB tiers, and `--full --huge` a 1 GB tier for a few rows.
`--cachegrind` (Linux with `valgrind` installed; an error elsewhere) also runs each tool once per row under `valgrind --tool=cachegrind --cache-sim=no` and reports `I refs` beside the times (#5552). An instruction count does not move with machine load, so it is a noise-free second signal, and the report marks a row where it and the wall-clock put succinctly on opposite sides of jq (both more than 5% from a tie): the two can disagree, and a start-up-bound row on a small file does. Only rows up to 1 MB are counted, since cachegrind runs ~50x slower, and only succinctly and jq 1.7.1; valgrind emulates a CPU without AVX-512, so the instruction count can describe a different code path than the timed run on a machine that has it. CI passes the flag on both Linux runners.
`--full --huge` adds a 1 GB tier (#6915, the streaming and early-exit item of #5409's dimension E): the six rows whose `perf.tsv` cap is `1gb` (`identity`, `identity_compact`, `users_first_early_exit`, `ndjson_field`, `ndjson_inputs_reduce`, `ndjson_inputs_first`) on `users-1gb` (1,097,976,224 bytes; the users generator writes about 0.73 of the size it is asked for, so the manifest asks for `1500mb`) and `ndjson-1gb` (1,073,741,867 bytes), both sha256-pinned.
Above 100 MB both tools fit in RAM on the bench machines, so a memory difference that is a ratio at 100 MB can be pass/fail here, and the rows exist to show which: `jq` holding ≈3 MB on NDJSON where succinctly holds a multiple of the input (#6913), and whether the single-document `identity` ratio keeps growing past 100 MB.
The tier is opt-in because a row costs minutes per tool (about an hour on top of `--full`), and needs RAM: jq holds a single 1 GB document as a DOM, which is many times its size, so run it where that fits, or under a memory-cap watchdog (macOS cannot `ulimit -v`).
Its rows go to `perf_huge` in `results.json` and to their own report section, never to `perf`, so no headline geomean or tier column moves.
Each tool runs three timed repetitions (its gating run is the warm-up) and one untimed memory run, and records how it ended: `exit`, the `signal` that killed it (9 is the out-of-memory kill) or `timed_out` past the 900 s deadline.
A tool that is killed or times out makes the row `incomplete`: untimed, with its exit status and the peak memory it reached listed, instead of aborting the run (a smaller tier still aborts on a deadline).
A refusal (the tools exit differently) stays a correctness row, `output differs`, and still lists each tool's exit status.
An incomplete row where only succinctly failed is reported as a divergence like any output difference, so the run exits 1 unless it is ledgered as `perf:<query>@<input>`.
`--write-docs` takes a `--full --huge` run like any complete one, so each platform's committed json carries the rows.
Until a platform has had such a run, its committed report has no 1 GB section.

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

`--write-docs` writes `docs/compare/jq-<platform>.{md,json}` and the index `docs/compare/jq.md`, and regenerates the `<!-- compare:jq-<platform> -->` block of `docs/benchmarks/jq.md` from that json.
A platform's first run needs the two markers added at the top of its section there.
The section's `**CPU**:` line must name the run's CPU (two machines can share a platform), and `--write-docs` checks that before it measures anything.
`--render-docs` measures nothing: it re-renders the index and every block from the committed jsons, after a change to how they are rendered.
Both also regenerate the `<!-- compare:jq-summary -->` block of `CLAUDE.md` (one row per committed run: the geomean speedup of each size tier), from every committed json.
And the `<!-- compare:jq-identity -->` block of `README.md` (one table per committed run: the `identity` query on each 10 MB input, with peak memory, #6871). A test in `src/bin/succinctly/compare/docs.rs` fails when either of those two, or the `docs/benchmarks/jq.md` blocks, is edited by hand.
The bin's `benchmarks_doc_quotes_the_committed_results` test fails when a block and its json disagree.
CI is `.github/workflows/compare-jq.yml`, run by hand or on a release, not on pull requests.
`.github/workflows/compare-jq-ledger.yml` runs the correctness dimensions (no timing rows) on pull requests that touch
`comparison/jq/`, `src/jq/` or the harness, and weekly, and fails on an unlisted or stale ledger entry (#5565).
