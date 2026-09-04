# Benchmarking Guide

[Home](../../) > [Docs](../) > [Guides](./) > Benchmarking

This guide provides complete information for running, interpreting, and documenting benchmarks in the succinctly project.

## Table of Contents

- [Overview](#overview)
- [Benchmark Inventory](#benchmark-inventory)
- [Types of Benchmarks](#types-of-benchmarks)
- [When to Run Benchmarks](#when-to-run-benchmarks)
- [How to Run Benchmarks](#how-to-run-benchmarks)
- [Distributed Benchmark Orchestration](#distributed-benchmark-orchestration-issue-98)
- [A/B Benchmarking Method](#ab-benchmarking-method)
- [Data Generation](#data-generation)
- [Platforms and Hardware](#platforms-and-hardware)
- [CI/CD Integration](#cicd-integration)
- [Interpreting Results](#interpreting-results)
- [Updating Documentation](#updating-documentation)
- [Troubleshooting](#troubleshooting)

---

## Overview

Succinctly uses multiple types of benchmarks to measure performance:

1. **Micro-benchmarks**: Fine-grained measurements of specific operations (rank, select, parsing)
2. **End-to-end benchmarks**: Full pipeline measurements (CLI tool vs competitors)
3. **Cross-parser benchmarks**: Comparison with other Rust JSON parsers
4. **Optimization benchmarks**: Before/after measurements for specific optimizations

All benchmarks use **Criterion.rs** for statistical accuracy with warm cache conditions.

### Benchmark Philosophy

- **Measure first, optimize later**: Always profile before optimizing
- **Micro ≠ macro**: Micro-benchmark wins don't always translate to end-to-end improvements
- **Test on target hardware**: Performance varies significantly across architectures
- **Document everything**: Keep detailed records in `docs/benchmarks/` and `docs/parsing/`

---

## Benchmark Inventory

For a complete index of all benchmark reports by file and section, see:

**[docs/benchmarks/inventory.md](../benchmarks/inventory.md)** - Complete inventory of ~110+ benchmark report sections across 6 files

---

## Types of Benchmarks

### 1. Core Data Structure Benchmarks

Located in `benches/`:

| Benchmark             | Purpose                                     | Data Source      |
|-----------------------|---------------------------------------------|------------------|
| `rank_select`         | BitVec rank/select operations               | Generated inline |
| `balanced_parens`     | Tree navigation operations                  | Generated inline |
| `elias_fano`          | Elias-Fano encoding for monotone sequences  | Generated inline |
| `line_index`          | `to_line_column` vs pre-#228 dense `BitVec` | Generated inline |
| `popcount_strategies` | Popcount implementations                    | Generated inline |

### 2. JSON Benchmarks

| Benchmark               | Purpose                                    | Data Source      |
|-------------------------|--------------------------------------------|------------------|
| `json_simd`             | SIMD implementations comparison            | Generated files  |
| `pfsm_vs_simd`          | Table-based vs SIMD parsing                | Generated files  |
| `pfsm_vs_scalar`        | Table-based vs scalar parsing              | Generated files  |
| `json_pipeline`         | Full pipeline (index + navigate + print)   | Generated files  |
| `jq_comparison`         | succinctly jq vs system jq (CLI)           | Generated files  |

### 3. YAML Benchmarks

| Benchmark               | Purpose                                    | Data Source      |
|-------------------------|--------------------------------------------|------------------|
| `yaml_bench`            | YAML parsing throughput                    | Generated inline |
| `yaml_type_stack_micro` | YAML type stack operations                 | Generated inline |
| `yaml_anchor_micro`     | YAML anchor parsing                        | Generated inline |
| `yaml_transcode_micro`  | YAML→JSON transcoding                      | Generated inline |
| `yq_comparison`         | succinctly yq vs system yq (CLI)           | Generated files  |
| `yq_select`             | Partial selection queries (lazy eval)      | Generated files  |
| `bp_select_micro`       | BP select1 performance                     | Generated inline |

### 4. DSV Benchmarks

| Benchmark               | Purpose                                    | Data Source      |
|-------------------------|--------------------------------------------|------------------|
| `dsv_bench`             | DSV/CSV parsing and access                 | Generated files  |

### 5. Cross-Parser Benchmarks

Located in `bench-compare/benches/`:

| Benchmark               | Purpose                                    | Parsers Compared                           |
|-------------------------|--------------------------------------------|--------------------------------------------|
| `json_parsers`          | Compare against other Rust JSON parsers    | serde_json, simd-json, sonic-rs, succinctly|

**Why separate?** Avoids adding competitor dependencies to the main crate.

### 6. ARM-Specific Benchmarks

| Benchmark               | Purpose                                    | Data Source      |
|-------------------------|--------------------------------------------|------------------|
| `neon_movemask`         | NEON movemask implementations              | Generated inline |

---

## When to Run Benchmarks

### Required: Pre-Release

**Before every release**, run full benchmark suite on all platforms:

```bash
# On x86_64 (Zen 4 or similar)
./scripts/run-full-benchmarks.sh

# On ARM64 (Graviton or Apple Silicon)
./scripts/run-full-benchmarks.sh
```

Update `docs/benchmarks/*.md` with new results if significant changes.

### Recommended: After Optimizations

After implementing a performance optimization:

1. **Run relevant micro-benchmarks** to verify the optimization works
2. **Run end-to-end benchmarks** to verify real-world impact
3. **Document results** in `docs/parsing/yaml.md` or `docs/optimizations/`

Example: After SIMD optimization for YAML:
```bash
# Micro-benchmark
cargo bench --bench yaml_bench

# End-to-end
cargo bench --bench yq_comparison
```

### Optional: During Development

Run targeted benchmarks when:
- Modifying hot paths (parsing, rank/select)
- Investigating performance regressions
- Exploring new optimization ideas

### CI/CD: Automated

GitHub Actions runs `rank_select` benchmarks on every PR/push for smoke testing:
- x86_64 (ubuntu-latest)
- ARM64-Linux (ubuntu-24.04-arm)
- ARM64-macOS (macos-latest)

**Note**: CI does not run full benchmark suite (too time-consuming).

**Perf Regression Guard** (issue #1523): a separate CI job measures `succinctly
jq`/`yq` instruction counts via `valgrind --tool=cachegrind` (`scripts/perf-guard.py`)
for a fixed query/shape matrix, and fails if any drifts more than 5% from a baseline.
Deterministic instruction counts, not wall-clock time, are what make a tight threshold
viable on a shared, noisy CI runner — this exists because #1385's 2-3x regression
shipped through `rank_select`-only smoke testing unnoticed. Runs on x86_64 and
ARM64-Linux only (valgrind has no Apple Silicon support); currently a non-required
check while it builds a track record.

On `pull_request` runs, the baseline is a binary built fresh from the PR's own
`git merge-base` in the same CI run (`--baseline-binary`), not the committed file —
see #1582 below for why. On `push` runs (every commit the merge queue lands on
`main`), the baseline is likewise a binary built fresh from `github.event.before`
(the commit `main` pointed to immediately before that push, #2117) — the checked-in
`tests/data/perf-guard-baseline.json` is now only a fallback for the rare case
neither can supply a binary (a branch's first push, or a `before` SHA the checkout
doesn't have), and a seed for running the script locally with no comparison binary
of your own. To deliberately update that file after a real, understood cost change:

```bash
cargo build --release --features cli
scripts/perf-guard.py --binary target/release/succinctly --arch x86_64 --update-baseline
```

(run once per arch you can reach; state the reason in the commit message).

**Why the file isn't the primary comparison (#1582).** At this project's merge
cadence, the checked-in baseline drifts on its own: rebuilding at the baseline's own
source commit vs. `main` three days / 147 commits later (55 touching
`eval.rs`/`json`/`yaml`) showed **+1.0% to +3.2% drift on every one of the six
queries**, with zero PR involved — ordinary accumulated small changes eating most of
the 5% budget before any single PR is measured. The obvious first guess — that this is
the same `codegen-units=16` cross-module placement artifact #1587 found and fixed by
pinning `codegen-units=1` (see rule 9 below) — does not hold here: rebuilding both
sides of that same three-day comparison at `codegen-units=1` left the drift the same
size or larger, not collapsed the way #1587's wall-clock/icache case did. So the fix is
structural, not a build flag: compare each PR against a binary built from its own
merge-base in the same run, which cancels staleness (of any cause) instead of trying to
keep a single checked-in file fresh against a fast-moving `main`.

---

## How to Run Benchmarks

### Unified Benchmark Runner (Recommended)

The project includes a unified benchmark runner that provides discovery, listing, and running all benchmarks with automatic metadata tracking.

```bash
# Build with bench-runner feature
cargo build --release --features bench-runner

# List all available benchmarks
./target/release/succinctly bench list

# List by category
./target/release/succinctly bench list --category core
./target/release/succinctly bench list --category json

# Run specific benchmarks
./target/release/succinctly bench run rank_select yaml_bench

# Run all benchmarks in a category
./target/release/succinctly bench run --category core

# Run all benchmarks
./target/release/succinctly bench run --all

# Dry run (see what would execute)
./target/release/succinctly bench run --dry-run --all
```

**Output Location**: Results are saved to `data/bench/results/<timestamp>_<commit>/` with:
- `metadata.json` - Git, system, and toolchain information
- `summary.json` - Run summary with pass/fail status
- `stdout/` - Raw output from each benchmark

### Quick Start (Traditional)

```bash
# 1. Build release binary
cargo build --release --features cli

# 2. Generate test data
cargo run --release --features cli -- json generate-suite
cargo run --release --features cli -- yaml generate-suite
cargo run --release --features cli -- dsv generate-suite

# 3. Run all benchmarks
cargo bench

# 4. Run cross-parser comparison
cd bench-compare
cargo bench
```

### Running Specific Benchmarks

#### Core Data Structures
```bash
cargo bench --bench rank_select        # BitVec operations
cargo bench --bench balanced_parens    # Tree navigation
cargo bench --bench popcount_strategies # Popcount implementations
```

#### JSON Benchmarks
```bash
cargo bench --bench json_simd          # SIMD comparison
cargo bench --bench pfsm_vs_simd       # PFSM vs SIMD
cargo bench --bench pfsm_vs_scalar     # PFSM vs scalar
cargo bench --bench json_pipeline      # Full pipeline
cargo bench --bench jq_comparison      # vs system jq
```

#### YAML Benchmarks
```bash
cargo bench --bench yaml_bench         # Parsing throughput
cargo bench --bench yq_comparison      # vs system yq
cargo bench --bench yq_select          # Lazy evaluation
cargo bench --bench bp_select_micro    # BP select1
```

#### DSV Benchmarks
```bash
cargo bench --bench dsv_bench          # CSV/TSV parsing
```

#### Cross-Parser Comparison
```bash
cd bench-compare

# All benchmarks
cargo bench --bench json_parsers

# Specific groups
cargo bench --bench json_parsers -- "parse_only"
cargo bench --bench json_parsers -- "parse_traverse"
cargo bench --bench json_parsers -- "traverse_only"
cargo bench --bench json_parsers -- "peak_memory"
```

### Running with Specific Features

```bash
# Default (hardware popcount)
cargo bench --bench rank_select

# Explicit SIMD
cargo bench --bench rank_select --features simd

# Portable popcount (broadword)
cargo bench --bench rank_select --features portable-popcount
```

### Running with Native CPU Optimizations

For best performance on your CPU:

```bash
# x86_64 with AVX2/BMI2
RUSTFLAGS="-C target-cpu=native" cargo bench

# Verify features detected
RUSTFLAGS="-C target-cpu=native" cargo rustc -- --print=cfg | grep target_feature
```

### Filtering Benchmarks

```bash
# Run only benchmarks matching pattern
cargo bench -- "rank1"                 # Only rank1 benchmarks
cargo bench -- "10mb"                  # Only 10MB file benchmarks
cargo bench --bench yaml_bench -- "simple_kv"  # Only simple_kv pattern

# Save baseline for comparison
cargo bench -- --save-baseline main

# Compare against baseline
cargo bench -- --baseline main
```

### Controlling Sample Size

```bash
# Quick run (fewer samples)
cargo bench -- --quick

# Specific sample size
cargo bench -- --sample-size 10

# Warm-up iterations
cargo bench -- --warm-up-time 1
```

---

## Distributed Benchmark Orchestration (issue #98)

The unified benchmark runner (above) runs benchmarks on one machine. `succinctly bench orchestrate` and its companion commands (`nodes`, `sync`, `report`) automate the manual multi-machine workflow this guide otherwise describes by hand (§ "Building both halves on a remote box", § Platforms and Hardware): define your machines once in `nodes.yaml`, then fan the same `bench run` invocations out across all of them over SSH and compare the results.

Requires the `bench-runner` feature (which now also pulls in `serde_yaml` for config parsing):

```bash
cargo build --release --features bench-runner
```

### Setup

```bash
cp nodes.yaml.example nodes.yaml   # gitignored — encodes your machine-specific hosts/keys
$EDITOR nodes.yaml                 # fill in real hostnames
succinctly bench nodes --config nodes.yaml --status
```

A node's `host` is whatever `ssh` accepts as a destination; only EC2-backed nodes need `ec2_instance_id`/`ec2_region` (for `bench nodes --start/--stop`) and typically an explicit `ssh_key`. `host: localhost` (or `127.0.0.1`) runs commands directly with no `ssh` wrapper at all — useful both as the always-available "local" node and for a network-free dry run of the whole pipeline.

**Always spell out the username** (`user@host`), even one that matches your local shell's default. Some SSH access layers (mesh VPNs with per-connection ACLs, bastion proxies) enforce their own username policy independent of key-based auth — an omitted username silently falls back to your local machine's username, which such a layer can reject even though the configured key is completely valid. The failure mode looks like a broken key ("permission denied") but is actually a policy check on the wrong identity; `bench nodes --status` reporting a node `unreachable` despite `ssh` working fine by hand is the tell.

### `bench nodes` — status, and EC2 start/stop

```bash
succinctly bench nodes --config nodes.yaml --status   # default when no flag given
succinctly bench nodes --config nodes.yaml --start     # start any stopped EC2-backed nodes
succinctly bench nodes --config nodes.yaml --stop      # stop them again (EC2 instances cost money while running)
```

`bench orchestrate` never auto-starts a stopped EC2 instance — that's a silent, cost-incurring side effect a user should choose explicitly via `--start`.

### `bench sync` — cross-compile and deploy the release binary

```bash
succinctly bench sync --config nodes.yaml               # sync every node whose reported --version is stale
succinctly bench sync --config nodes.yaml --node sydney  # sync just one node
succinctly bench sync --config nodes.yaml --force        # re-sync even if the version already matches
```

Cross-compiling for a node requires that target's toolchain already installed locally (`rustup target add aarch64-unknown-linux-gnu`, etc.) **and** a linker that can produce that target's binaries — `rustup target add` alone is not enough for a cross-OS target (e.g. macOS host → Linux target; both being `aarch64` doesn't help, the linker still needs to emit ELF, not Mach-O). `bench sync` doesn't install either for you. A node's `target_triple` disambiguates cases `arch` alone can't (`aarch64-apple-darwin` vs `aarch64-unknown-linux-gnu` are both `aarch64`).

If a working cross-linker isn't set up, skip `bench sync` (`--no-sync`) and build natively on the node instead: install a Rust toolchain there (`curl https://sh.rustup.rs | sh`, plus a C toolchain — e.g. `dnf install gcc` on Amazon Linux, needed by several dependencies' build scripts), copy the source over (`rsync -az --exclude .git --exclude target`), and run `cargo build --release --features bench-runner` directly on the node. `bench orchestrate` only cares that a binary with the `bench-runner` feature ends up at `<working_dir>/target/release/succinctly` — it doesn't care how it got there.

### `bench orchestrate` — run benchmarks across nodes

```bash
succinctly bench orchestrate --config nodes.yaml --all --dry-run   # show the planned schedule, touch nothing
succinctly bench orchestrate --config nodes.yaml --all              # sync (unless --no-sync), then run
succinctly bench orchestrate --config nodes.yaml --arch aarch64 rank_select yaml_bench
succinctly bench orchestrate --config nodes.yaml --node sydney --no-sync --collect-only
```

Each node runs on its own thread; within a node, benchmarks execute in isolation-aware "waves" — every benchmark defaults to `exclusive` (it gets a node to itself, since sharing a CPU while criterion measures wall time skews results), overridable per-name via `nodes.yaml`'s `benchmarks:` list:

```yaml
benchmarks:
  - name: rank_select
    isolation: concurrent   # may share a node's `max_concurrent` slots with other concurrent benchmarks
```

Results land under `coordinator.results_dir` (default `data/bench/distributed/`) as `<run_id>/<node>/<benchmark>/summary.json` (+ `.jsonl`/`.md` for CLI-type benchmarks), plus a per-node `node_info.json` (arch/features), a flattened `results.jsonl`, and a run-level `metadata.json`.

### `bench report` — compare results across nodes/architectures

```bash
succinctly bench report --current data/bench/distributed/2026-08-07T12-00-00
succinctly bench report --current <run> --baseline <prior-run> --threshold 0.10
```

Purely offline (no `nodes.yaml`, no SSH) — reads the `summary.json`/`node_info.json` files a `bench orchestrate` run already produced and writes `<current>/summary.md` with a per-benchmark node comparison table, a per-architecture average-duration table, and (with `--baseline`) a regression table for anything more than `--threshold` (default 10%) slower than the same benchmark on the same node in the baseline run. `--check` verifies the report file is up to date instead of rewriting it (exits non-zero on drift), for CI.

---

## Data Generation

### JSON Test Data

Generate test files in `data/bench/generated/`:

```bash
# Individual sizes
cargo run --release --features cli -- json generate 1kb -o data/bench/generated/comprehensive/1kb.json
cargo run --release --features cli -- json generate 10kb -o data/bench/generated/comprehensive/10kb.json
cargo run --release --features cli -- json generate 100kb -o data/bench/generated/comprehensive/100kb.json
cargo run --release --features cli -- json generate 1mb -o data/bench/generated/comprehensive/1mb.json
cargo run --release --features cli -- json generate 10mb -o data/bench/generated/comprehensive/10mb.json
cargo run --release --features cli -- json generate 100mb -o data/bench/generated/comprehensive/100mb.json

# All sizes at once
cargo run --release --features cli -- json generate-suite
```

### YAML Test Data

```bash
# All patterns and sizes (defaults to --max-size 1gb; 16 patterns x 7 sizes is
# a lot of disk, so cap it unless you need the largest files)
cargo run --release --features cli -- yaml generate-suite --max-size 100mb

# Individual pattern/size (size is positional, pattern is a flag)
cargo run --release --features cli -- yaml generate 1mb --pattern comprehensive -o test.yaml
```

#### Patterns and what they cover

The generated suite is what exercises the **query and streaming** paths
(`dev bench yq`, `yq_comparison`, `yq_select`); `benches/yaml_bench.rs` covers
index **build** over its own fixtures. A construct absent from the suite cannot
be measured end-to-end, so "the benchmarks are neutral" would say nothing about
it — the gap that let the quadratic sequence-iteration bug of #106 survive a
full benchmark run.

> **Pattern coverage note (#517).** `benches/yq_comparison.rs` used to hardcode
> a 5-pattern subset (`comprehensive`, `users`, `nested`, `sequences`,
> `strings`) with no link to the table below, so every pattern added since
> — including `flow`, the pattern #362's closing comment asked to be measured
> end-to-end — was invisible to it. It now derives its pattern list from
> `yaml_pattern_registry::ALL_PATTERNS` (shared with `dev bench yq` via
> `#[path]`, split out of `yaml_generators.rs` so the bench doesn't also
> compile every `generate_*` function), the same source as the table below, so
> a pattern registered here is benchmarked there without a second edit.
> `config` is skipped as before — it generates one `config.yaml`, not a
> `{size}.yaml` per rung of the size ladder, the same gap `dev bench yq` has —
> but now via a named `SKIP` list rather than a bare `!exists()` fallthrough; a
> `check_skip_list()` guard (run at the top of `bench_succinctly_identity`, the
> pattern `jq_string_ops_bench.rs`'s `check_parity()` uses for a
> `harness = false` bench) fails loudly if `SKIP` drifts from the registry's
> `PatternScale::Fixed` patterns.
> Measured on Apple M1 Max: the identity-comparison groups' warm-up/measurement
> time was trimmed from criterion's defaults (3s/5s) to 1s/2s to keep the full
> 16-pattern x 4-size sweep from tripling runtime; the full `cargo bench --bench
> yq_comparison` run went from 7m50s (5 patterns, 46 benchmark ids, default
> timing) to 10m41s (16 patterns/15 with generated data, 126 benchmark ids,
> trimmed timing) — a 1.4x increase for roughly 3x the pattern coverage.

| Pattern         | Shape                                                    |
|-----------------|----------------------------------------------------------|
| `comprehensive` | Mixed features, the default comparison target             |
| `users`         | Records with realistic field mixes                        |
| `nested`        | Deep block mappings (depth 6)                             |
| `sequences`     | Sequence-heavy block content                              |
| `mixed`         | Mappings and sequences interleaved                        |
| `strings`       | Quoted and plain scalars                                  |
| `numbers`       | Integers, decimals, scientific notation                   |
| `unicode`       | Multi-script strings                                      |
| `pathological`  | Wide sibling sets with nesting                            |
| `navigation`    | Top-level array for M2 streaming queries                  |
| `config`        | Realistic fixed-size config template                      |
| `flow`          | Flow `{...}` / `[...]`, bimodal sizes plus deep nesting    |
| `anchors`       | `&name` / `*name`, three anchor-name length buckets        |
| `block-scalars` | `\|` and `>` with all three chomping modes                 |
| `explicit-keys` | `? ` / `: `, including the valueless form                 |
| `multi-doc`     | `---` / `...` streams                                     |

The last five were added by #327; before that, **none** of flow, anchors, block
scalars, explicit keys or multi-document input appeared anywhere in the suite.
Their sizing is deliberate:

- `flow` is bimodal — most collections are 10-30 bytes (the size the P5
  rejection argued real flow collections are), with a 64-256 byte tail that
  spans the ~32-byte threshold where SIMD scanning starts to win. That makes the
  P5 question answerable end-to-end rather than by argument.
- `anchors` uses 2-8, ~24 and ~48 character anchor names, because P4's SIMD
  anchor scanning only pulls ahead from ~32 bytes up.
- `block-scalars` includes ~100-line bodies, the shape P2.7 was measured on.
- `explicit-keys` emits valueless explicit keys (`? a` with no `: value`
  followed by another entry), the specific form that selects the
  `OpenPositions::Dense` fallback. Ordinary `? key` / `: value` pairs stay
  monotonic and use the compact encoding.

`tests/yaml_bench_suite_coverage.rs` asserts on every CI leg that each supported
construct still appears in some pattern, so a future omission fails the build
instead of silently narrowing what the benchmarks can see.

#### Deliberately out of scope

These are **not** generated. Each would make the affected files diverge from
`yq`, so a comparison over them would be timing two different computations. The
coverage test asserts their absence, so fixing one forces a decision here rather
than letting the gap persist unnoticed.

| Construct                            | Why not generated                                                                 |
|--------------------------------------|-----------------------------------------------------------------------------------|
| Merge keys `<<:`                     | Parsed as an ordinary key: output is `{"<<": {...}}` where yq splices (#171)       |
| Tags (`!!str`, `!custom`)            | Rejected outright in block context (documented non-support in `src/yaml/mod.rs`)   |
| Blank lines in a **folded** scalar   | Emits one newline too many per blank line (`"a\n\nb\n"` where yq folds to `"a\nb\n"`) (#329) |

Both #328 and #329 were found while building the suite for #327 (the generator
matches `yq` byte-for-byte on everything it does emit, which is how they
surfaced). #328 — `- &anchor` followed by a collection — is now fixed, and the
`anchors` pattern generates that shape; #329 remains out of scope.

### DSV Test Data

```bash
# All patterns and sizes
cargo run --release --features cli -- dsv generate-suite

# Individual pattern/size
cargo run --release --features cli -- dsv generate 1000 -p users -o users.csv

# Omit the header row
cargo run --release --features cli -- dsv generate 1000 -p users --no-header -o users.csv
```

### Available Patterns

| Format | Patterns |
|--------|----------|
| JSON   | comprehensive, users, nested, arrays, strings, numbers, pathological, mixed, literals, unicode |
| YAML   | comprehensive, users, nested, sequences, strings, numbers, pathological, mixed, unicode, config, navigation |
| DSV    | users, numeric, tabular, mixed, quoted, strings, wide, long, multiline, pathological |

### Standard Sizes

- 1KB, 10KB, 100KB (small files)
- 1MB, 10MB (medium files)
- 100MB (large files)
- 1GB (extra large, rarely used)

---

## Real-Workload Corpus (#301)

The generators above produce *synthetic* inputs. Some performance decisions hinge
on whether an input *shape* (flow-collection size, escape density, nesting depth,
anchor/alias counts, keys per object) actually occurs in real files — the check
that rejected **P5** (real flow collections are 10–30 B, not the 64–128 B the
micro-benchmark favoured). To make that a lookup instead of a bespoke
investigation, a versioned corpus of real, permissively-licensed files is
assembled by a sync script, with per-file provenance in
[`tests/data/bench-corpus/manifest.json`](../../tests/data/bench-corpus/manifest.json)
([provenance & licenses](../../tests/data/bench-corpus/NOTICES.md)).

```bash
# Populate data/bench/corpus/ (copies the committed seed + fetches the large
# tier, verifying each file's sha256 against the manifest).
./scripts/sync-bench-corpus.sh

# Verify the committed seed against the manifest (offline; what CI runs).
./scripts/sync-bench-corpus.sh --check
```

The corpus mirrors the `<workload>/<file>` layout the runners expect, so it feeds
the existing benchmarks with no code changes:

```bash
succinctly dev bench yq  --data-dir data/bench/corpus/yaml
succinctly dev bench jq  --data-dir data/bench/corpus/json
succinctly dev bench dsv --data-dir data/bench/corpus/dsv
```

### Shape-statistics lookup

`corpus-stats` derives shape distributions directly from the crate's own
semi-index (so they measure exactly what the parsers see) and writes the
representativeness table:

```bash
# Regenerate the human-facing report over the full corpus (after a sync).
succinctly dev bench corpus-stats --data-dir data/bench/corpus \
  --markdown docs/benchmarks/corpus-shape.md
```

Before making any performance doc claim (per #235 Phase 6), consult
[docs/benchmarks/corpus-shape.md](../benchmarks/corpus-shape.md): **confirm the
input shape you are optimising for actually appears at the size you expect.**
This gates the open perf issues #40 #56 #58 #64 #91 #106 #122 #123 #124 #126 #130
#133 #134. The committed golden over the seed
([`tests/data/bench-corpus/expected-shape.md`](../../tests/data/bench-corpus/expected-shape.md))
is drift-checked in CI so the tooling and report cannot silently rot.

### Frequency lookup

The corpus establishes that a shape **exists** at a given size. It cannot establish
**how often** it occurs — a handful of files cannot measure a shape present in a
fraction of a percent of real input, and reading a shape's presence in a corpus this
small as "one file in eight looks like this" inverts the error the corpus exists to
prevent. For frequency, scan whole upstream repositories:

```bash
# The 26-repo set behind the recorded results (streams tarballs; nothing hits disk).
./scripts/survey-yaml-shapes.sh --defaults
```

Results are recorded in
[docs/benchmarks/yaml-shape-survey.md](../benchmarks/yaml-shape-survey.md), which is
where #326 established that bare-dash sequence items occur in 0.042% of real YAML
files — and, conversely, that the corpus's `anchors: 0` was a sampling artefact rather
than upstream reality, since anchors reach 4–11% of files in the ecosystems that use
them. #342 acted on that finding by vendoring a Home Assistant config, so the corpus
now reports 41 anchors and 86 aliases. That page is date-pinned, not CI-checked:
upstream branches move.

### Select scan-length lookup

`corpus-stats` answers "does this input shape exist?". `select-stats` answers the
same question one level down, for the `select` word scans: **how many words does
each scan actually traverse?** It walks the corpus through the same cursor APIs
`jq`/`yq` use and reads counters compiled in by the `select-stats` feature.

```bash
cargo run --release --features cli,select-stats -- \
  dev select-stats --data-dir data/bench/corpus
```

The counters live behind a non-default feature, so they are absent from ordinary
builds and **must never be enabled for a timing run**. The command exits non-zero
if it recorded nothing, so a binary built without the feature cannot be mistaken
for a workload that performs no scans.

Read the `work >=4` column, not `calls <4`: the distribution is bimodal, and the
share of *work* in long scans is what predicts a block kernel's effect. This is
the measurement that justified [select-scan.md](../optimizations/select-scan.md).

---

## Platforms and Hardware

### Primary Benchmark Platforms

#### 1. AMD Ryzen 9 7950X (Zen 4, x86_64)
- **Location**: Local development machine
- **OS**: Linux 6.6.87.2-microsoft-standard-WSL2 (WSL2)
- **SIMD**: AVX2, BMI2 PDEP (3-cycle)
- **Use for**: x86_64 baseline, BMI2 optimizations
- **Access**: Local

#### 2. ARM Neoverse-V2 (AWS Graviton 4)
- **Location**: AWS EC2 instance
- **OS**: Linux 6.14.0-1018-aws
- **SIMD**: NEON (128-bit), SVE2 (128-bit), SVEBITPERM (BDEP/BEXT)
- **Use for**: ARM64 server performance, SVE2 features
- **Access**: Cloud instance (manual provisioning)

#### 3. ARM Neoverse-V1 (AWS Graviton 3)
- **Location**: AWS EC2 instance
- **OS**: Linux 6.14.0-1018-aws
- **SIMD**: NEON (128-bit), SVE (256-bit)
- **Use for**: ARM64 baseline, SVE support
- **Access**: Cloud instance (manual provisioning)

#### 4. Apple M1 Max (Apple Silicon)
- **Location**: Local development machine
- **OS**: macOS Darwin 25.1.0
- **SIMD**: ARM NEON (128-bit)
- **Use for**: Consumer ARM64 performance
- **Access**: Local

### CI/CD Platforms

GitHub Actions runs benchmarks on:
- **ubuntu-latest** (x86_64)
- **ubuntu-24.04-arm** (ARM64)
- **macos-latest** (Apple Silicon M-series)

### Checking CPU Features

#### Linux
```bash
# All features
lscpu | grep -E "Model name|Architecture|CPU|Flags"
cat /proc/cpuinfo | grep flags

# Specific features
grep -q avx2 /proc/cpuinfo && echo "AVX2: supported"
grep -q popcnt /proc/cpuinfo && echo "POPCNT: supported"
grep -q bmi2 /proc/cpuinfo && echo "BMI2: supported"
grep -q sve2 /proc/cpuinfo && echo "SVE2: supported"
grep -q svebitperm /proc/cpuinfo && echo "SVEBITPERM: supported"
```

#### macOS
```bash
sysctl -n machdep.cpu.brand_string
sysctl -n machdep.cpu.features
system_profiler SPHardwareDataType
```

### Performance Characteristics by Platform

| Platform | Popcount | select_in_word | JSON Parse | YAML Parse | Notes |
|----------|----------|----------------|------------|------------|-------|
| AMD Zen 4 | Hardware | BMI2 PDEP (fast) | ~880 MiB/s | ~250-400 MiB/s | 3-cycle PDEP |
| ARM Graviton 4 | Hardware | SVE2 BDEP | ~550 MiB/s | ~250 MiB/s | SVE2 BDEP slower than BMI2 |
| ARM Graviton 3 | Hardware | Broadword | ~550 MiB/s | ~250 MiB/s | No SVE2 BDEP |
| Apple M1 Max | Hardware | Broadword | ~550 MiB/s | ~250 MiB/s | No SVE/SVE2 |

---

## CI/CD Integration

### Automated Benchmarks

The CI workflow (`.github/workflows/ci.yml`) runs benchmarks on every push/PR:

**Test Matrix**:
- x86_64 (ubuntu-latest)
- ARM64-Linux (ubuntu-24.04-arm)
- ARM64-macOS (macos-latest)

**Benchmarks Run**:
```yaml
- cargo bench --bench rank_select -- --noplot
- cargo bench --bench rank_select --features simd -- --noplot
- cargo bench --bench rank_select --features portable-popcount -- --noplot
```

**Artifacts**: Uploaded as `benchmark-results-{arch}` with 3 text files per platform.

### What CI Does NOT Run

CI intentionally skips:
- End-to-end comparison benchmarks (require system jq/yq)
- Cross-parser benchmarks (require external dependencies)
- Large file benchmarks (100MB+, too time-consuming)
- All JSON/YAML/DSV benchmarks (require generated data)

These must be run **manually before releases**.

### Adding Benchmarks to CI

To add a benchmark to CI, edit `.github/workflows/ci.yml`:

```yaml
- name: Run benchmarks (new_bench)
  run: cargo bench --bench new_bench -- --noplot | tee bench-new.txt
```

**Considerations**:
- Keep runtime under 5 minutes per platform
- Use `--noplot` to skip graph generation
- Use `tee` to save results to artifact
- Avoid requiring external tools or large files

---

## A/B Benchmarking Method

Before/after measurement of a code change has its own failure modes, distinct from running a
benchmark suite. Every rule below cost a wrong conclusion at least once, during the seq-item
detection work in #106; that investigation's write-up in `docs/parsing/yaml.md` (optimisations
O5 and O6) carries the full numbers.

`scripts/ab-cli.py` implements rules 1, 2, 4 and 6 for CLI-level A/B work:

```bash
# what the harness reports for a change that does not exist — run this first, once per machine
scripts/ab-cli.py --before ./succ-base --control --corpus ~/wrk/bench-scratch/mycorpus

# the real comparison
scripts/ab-cli.py --before ./succ-base --after ./succ-head --corpus ~/wrk/bench-scratch/mycorpus
```

Run `--control` before trusting any result. It times the baseline binary against a copy of
itself, so whatever it reports is noise by construction, and that is the bar a real delta has
to clear — an idle M4 Pro reads +0.09% median with a per-row range of -1.8%..+1.5%. Without
that number, #332's +1.1% median across 22 workloads was arguable; with it, the consistent
sign was decisive, and the fix was restructured before merge.

### 1. Interleave the two binaries; never run all of A then all of B

Alternate them **within** each repetition, then compare min-of-N and median:

```python
for i in range(reps):
    if i % 2 == 0: B.append(time(before)); A.append(time(after))
    else:          A.append(time(after));  B.append(time(before))
```

Running the halves back-to-back made an *improved* binary measure **0.47-0.9x — i.e. up to 2x
slower** — on every workload, because the second half starts on a thermally loaded machine.
Interleaving on the same machine showed the truth (neutral where expected, 1.6-16x on the
affected shape).

This is a **design** fix, not a statistical one: more repetitions and trimmed means do not
rescue a sequentially-run A/B, because the bias is monotonic drift rather than noise. Note that
min and median can *agree with each other* and still both be wrong, so their agreement is
necessary but not sufficient evidence.

Interleaving is not concurrency — one process still runs at a time.

### 2. Process-spawn A/B needs inputs >= 1 MB

Binary startup is ~4-6 ms, which is the entire runtime for 1kb/10kb/100kb inputs — yet those
sizes are in `dev bench yq`'s default list. Deltas there measure startup jitter, not the change.
Use `--sizes 1mb,10mb` for CLI A/B work, or a Criterion in-process benchmark for smaller inputs.

### 3. Report the scaling curve, not a single ratio

Generate two or three sizes. A speedup that **grows with input size** is the signature of an
algorithmic term being removed rather than a constant-factor win, and that shape is far more
robust to timing noise than any absolute number:

| input               | 100 KB | 1 MB  | 4 MB   |
|---------------------|--------|-------|--------|
| #106 speedup (Zen 4)| 1.16x  | 2.97x | 16.37x |

A constant-factor change cannot produce that curve, so the curve independently corroborates the
mechanism claimed for the fix.

### 4. Gate on output identity before believing any timing

Diff both binaries' stdout across every input x query combination, and confirm they still match
the reference tool (`jq`/`yq`). #106 compared 48 configurations per machine, 0 differences. A
faster binary that changed behaviour is not a win, and this check is cheap.

### 5. Measure both architectures — the effect size differs, not just the noise

The same #106 commit measured **6.1x on Apple M4 Pro and 16.4x on Ryzen 9 7950X**. Post-fix
times were comparable (124 ms vs 137 ms); the *pre*-fix times differed 3x (758 ms vs 2240 ms),
because the removed pathology was a cache-thrashing bitmap rescan that Apple's memory subsystem
absorbed far better than Zen 4. An Apple-only measurement understated the win by 2.7x.

Cache- and memory-bound results do not port between platforms. Name the chip in every table —
see [Platforms and Hardware](#platforms-and-hardware).

### 6. Verify the machine is idle — and read the right signal

**On macOS the load average is misleading.** A bench Mac showed a steady load of 1.4-1.5 while
total CPU across all processes was **2.4% of 12 cores**; macOS load counts uninterruptible-wait
threads, so an otherwise-idle desktop with an editor open reads as loaded. Check actual CPU and
look for real work:

```bash
ps -Ao pcpu | awk '{s+=$1} END {printf "total %.1f%%\n", s}'   # actual CPU
pgrep -fl "cargo|rustc|criterion|claude"                        # builds or agent runs
pmset -g batt | head -2                                         # must be AC, not battery
```

On Linux the load average is trustworthy. Never benchmark a laptop on battery.

### 7. A benchmark cannot measure a shape it does not generate

"All benchmarks neutral" is not evidence when the suite lacks the relevant input. In #106 every
YAML generator emitted `- ` (dash-space), so the `-\n` shape carrying a quadratic bug had zero
coverage — justified by a stale comment claiming the parser required dash-space. The fix was to
add a generator pattern for that shape *first*, then measure.

Before trusting a neutral result, confirm the suite contains the shape the change touches. If it
does not, add a pattern to `src/bin/succinctly/yaml_generators.rs` (or the JSON/DSV equivalent)
and regenerate.

### 8. A share of runtime is not a comparison, and one signal finds one regression

`b6c0c3ca` was titled *"cut the duplicate-key probe from 10.4% to 3.9%"* and shipped inside the
very series that #1514 later bisected as a 2-3x regression. Both figures are shares of the
*post-change* runtime. They say the probe got cheaper relative to a binary that was already
much slower than the baseline; they cannot say whether it got closer to it. Measured against
the pre-change commit, that same commit recovered part of one regression and left the other
slightly worse.

A perf claim needs a **named baseline commit that predates the change being judged** — not a
sibling from the same series, and not the current tip. "A benchmark claim is only as fresh as
its baseline" is already on record for the broadword UTF-8 reversal, where a clean rebase
silently improved the control side; this is the same failure with the staleness chosen rather
than inherited.

**Corollary for a detector** — anything that decides whether a fast path applies. Measure it on
the workload where the guarded code was *already fastest*, not the one where it was slowest: a
precheck is charged to every input including the ones it cannot help, so the input with the
least work to save is where its cost shows most. #1514's duplicate-key detector cost ~3% of
`sjq '.'` over a document of small objects and doubled `sjq keys_unsorted` over one holding a
single wide object.

**And pick more than one signal.** #1385 shipped two regressions on two paths. A `git bisect`
on `keys_unsorted` found the evaluator commit and stepped straight over the print-path one,
because `.` had barely moved at that commit; a second bisect on `.` was needed to find it. One
signal proves one thing. Before believing a bisect result, ask which paths the suspected change
touches and whether the signal is sensitive to each of them.

### 9. Pin the build profile before comparing two binaries

`Cargo.toml` has no `[profile.release]`, so `cargo build --release` uses Rust's default
`codegen-units = 16`, no LTO — the crate is split into 16 independently-optimized chunks, and
changing code in one module can shift how LLVM lays out *unrelated* modules in the binary. That
has nothing to do with the diff under test: `succinctly jq type`, a query that touches no keys,
measured **+12% on an M4 Pro and +27% on a 7950X** between two binaries whose diff cannot reach
that code path (#1587). It doesn't bisect to the same commit on both machines either — the
signature of a placement artifact, not a real regression, which always gives the same answer on
both.

`--control` does not catch this. It times `--before` against a copy of itself, so the two
binaries always share one codegen placement by construction; it bounds machine noise, not
placement drift between two genuinely *different* builds.

Rebuilding both sides with `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1` removes it —
`docs/plan/jq-duplicate-key-collapse.md` collapsed that same +12%/+27% pair to +0.2% this way,
mid-investigation, before the practice was written down here. Set the env var for both
`--before` and `--after` builds, then tell `scripts/ab-cli.py`/`scripts/perf-ab.py` what you did
via `--before-profile`/`--after-profile` (free text, e.g. `codegen-units=1`) — the scripts
cannot read a binary's codegen-units back out, so they only warn when the two builds' profiles
are unrecorded or don't match:

```bash
CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 cargo build --release --features cli   # both checkouts

scripts/ab-cli.py --before ./succ-base --after ./succ-head \
    --before-profile codegen-units=1 --after-profile codegen-units=1 \
    --corpus ~/wrk/bench-scratch/mycorpus
```

The crate's actual `[profile.release]` is deliberately left at Rust's default: pinning
`codegen-units = 1` there costs real compile time, with no benefit to the interactive
edit-compile-test loop, which uses debug builds, not release — so this is a benchmarking-time
convention, not a shipped build setting. Three interleaved clean `cargo build --release --features
cli` reps (18-core Apple M5 Max, `Johns-M5-Pro-Max` — a development machine running concurrent
work, not one of the idle bench boxes named elsewhere in this guide, so treat this as directional)
measured **1.4x-1.9x slower** (median 1.8x; 28.9s/24.1s/31.2s at `codegen-units=16` vs
51.4s/46.9s/42.3s paired at `codegen-units=1`). The multiplier scales with available parallelism —
losing per-crate codegen concurrency costs more on a wider machine — so don't expect a fixed ratio
across hardware.

Also worth knowing: #595's x86-only 12-28% `block_scalars` anomaly was closed as "expected, no
action — binary code-layout artifact from the recompile," using cachegrind to show flat
instruction counts (`Ir`) but rising L1-icache misses (`I1mr`). That diagnosis predates this
rule and never pinned codegen-units — see #1587 for the re-check.

**Update (#1582): `Ir` is not immune to this after all.** The paragraph above once
concluded that `scripts/perf-guard.py`'s CI gate was safe by construction, since it reads
`Ir` and #595's `Ir` stayed flat while only `I1mr` moved. #1582 found a case where `Ir`
itself drifts — 1-3% across every one of perf-guard's six queries between two `main`
commits three days apart, no single PR responsible — and confirmed with a paired
`codegen-units=1` rebuild that it is *not* the `codegen-units=16` mechanism this section
describes (that rebuild left the drift the same size or larger, not collapsed). What
actually protects the gate now is structural: `pull_request` runs compare against a binary
built from the PR's own merge-base in the same run, not a file that can go stale — see the
Perf Regression Guard section above.

### 10. Attribute the cost before you A/B it — hold one suspect quantity fixed

Rules 1-9 assume you already know what to change. When a performance *issue* names a suspected
cause, that is a hypothesis, not a diagnosis — and implementing its suggested fix before testing
it can mean a large, unnecessary change that does not even address the real term. #1213 and #1301
were both filed with a specific named culprit; both named the wrong one.

When the issue blames quantity A and you suspect quantity B, construct inputs that **hold A
provably constant while varying B**. If the time moves, A is not the term.

#1301 reported `del(.items[(0,1)].foo[])` as O(n^2) and attributed it to the path resolver
emitting `k*n` resolved paths instead of `k`. Holding total elements at 80,000 while widening the
computed key from `k = 2` branches to `k = 512` keeps the resolver emitting exactly 80,000 paths
in every row, while the sibling count within one container falls from 40,000 to 156:

| k   | siblings per container | time  | model `82.5ms + 2.52e-7*(n^2/k)` |
|-----|------------------------|-------|----------------------------------|
| 2   | 40,000                 | 890ms | 890                              |
| 8   | 10,000                 | 289ms | 284                              |
| 32  | 2,500                  | 133ms | 133                              |
| 128 | 625                    |  93ms |  92                              |
| 512 | 156                    |  95ms |  85                              |

A one-parameter model fit every row within ~3%, which located the term (`k*m^2`, the square of the
per-container sibling count) and pointed at the four linear scans responsible. The resolver was
never involved.

Two things this buys that a profile does not:

- **It cannot be argued with.** `sample`/`perf` only observe the window they run in, so "you
  attached too late, the resolver had already finished" is a live objection to a profile. A table
  where the blamed quantity is constant by construction has no such escape hatch.
- **It gives you the shape of the term, and therefore a prediction.** Fitting `a + b*(n^2/k)`
  yielded a floor of ~90ms at 80,000 elements *before any code changed*. The fix landed at 88ms.
  A pre-registered target is much stronger evidence than a number admired afterwards.

Report the fitted model alongside the measurements in the commit message and PR, not just in your
own head — "the model fits all seven widths within 3%" is what makes a diagnosis reviewable.

### Building both halves on a remote box

A source-only tarball plus a reverse patch of the commit under test is enough, with no pushing:

```bash
git diff HEAD <commit>~1 -- <changed files> > /tmp/revert.patch
tar czf /tmp/src.tgz --exclude=target --exclude=.git --exclude=data \
    Cargo.toml Cargo.lock src benches tests scripts        # ~770 KB
# remote: build HEAD -> succ-after; patch -p1 < revert.patch; build -> succ-before
```

Under `ssh -o BatchMode=yes` the login profile is not read, so prepend
`export PATH="$HOME/.cargo/bin:$PATH"` in the remote script (or use `bash -lc`).

---

## Interpreting Results

### Understanding Criterion Output

```
test bench_name ... bench:  1,234 ns/iter (+/- 56)
                              ^^^^^           ^^^^
                              mean           std dev
```

**Key metrics**:
- **Mean**: Average time per iteration
- **Std dev**: Standard deviation (lower = more consistent)
- **Throughput**: MiB/s, GB/s (for data-processing benchmarks)
- **Speedup**: Relative to baseline (e.g., 1.5x, 2.0x)

### Performance Expectations

#### Core Operations (x86_64 Zen 4)

| Operation | Expected Performance | Notes |
|-----------|---------------------|-------|
| rank1 | ~1-2 ns | O(1) with 3-level directory |
| select1 | ~10-20 ns | O(log n) binary search + scan |
| find_close | ~5-10 ns | O(1) with RangeMin |
| popcount (hardware) | ~1-2 ns | Single instruction |
| popcount (portable) | ~10-15 ns | Broadword algorithm |

#### Parsing Throughput (x86_64 Zen 4)

| Parser | Expected Throughput | Notes |
|--------|---------------------|-------|
| JSON (PFSM) | 800-900 MiB/s | Table-driven |
| YAML (oracle) | 250-400 MiB/s | Context-sensitive |
| DSV (API iteration) | 85-1676 MiB/s | Depends on pattern |
| DSV (parse only) | 1.3-3.7 GB/s | Index building |

#### Comparison Benchmarks (Expected Speedups)

| Benchmark | Expected Speedup | vs |
|-----------|------------------|-----|
| jq (x86_64) | 1.1-2.6x | System jq |
| jq (ARM) | 1.0-2.9x | System jq |
| yq (x86_64) | 7-27x | System yq |
| yq (ARM) | 1.3-9.9x | System yq |

### When to Investigate

**Investigate regressions when**:
- Core operations >10% slower than baseline
- End-to-end >5% slower than baseline
- Memory usage >10% higher than baseline
- Speedup vs competitors drops >20%

**Common causes**:
- Branch mispredictions (check with `perf stat`)
- Cache misses (check with `perf stat`)
- Memory bandwidth saturation
- Compiler regression (try different rustc versions)
- System load (ensure clean benchmark environment)

### Micro-Benchmark vs End-to-End

**Critical lesson**: Micro-benchmark wins don't always translate to real-world gains.

**Examples of misleading micro-benchmarks**:
- **P2.8 (SIMD Threshold Tuning)**: +2-4% micro, **-8-15% end-to-end** ❌
- **P3 (Branchless Character Classification)**: +3-29% micro, **-25-44% end-to-end** ❌
- **P2.6 (Software Prefetching)**: Looked promising, **-30% end-to-end** ❌

**Always validate with end-to-end benchmarks** before claiming an optimization works.

---

## Updating Documentation

### When to Update Docs

**Required**:
- After implementing optimization (document in `docs/parsing/yaml.md` or `docs/optimizations/`)
- Before release (update `docs/benchmarks/*.md` if significant changes)

**Optional**:
- After micro-optimizations (if <5% improvement)
- During exploration (use `docs/plan/` for proposals)

### Which Files to Update

#### 1. Optimization Phase Results

**File**: `docs/parsing/yaml.md` (or `docs/parsing/json.md`, `docs/parsing/dsv.md`)

**When**: After implementing an optimization phase

**Format**:
```markdown
### P#: Optimization Name - ACCEPTED ✅ / REJECTED ❌

**Goal**: Brief description of what you tried to optimize

**Approach**: Implementation details

**Benchmark Results** (Platform Name, Date):

| Benchmark | Before | After | Improvement |
|-----------|--------|-------|-------------|
| test/100  | 123µs  | 100µs | **-18.7%** (1.23x faster) |

**Key Findings**:
- Bulleted summary
- Include why accepted/rejected

**See Also**: [yq.md](../benchmarks/yq.md) for end-to-end results
```

#### 2. End-to-End Comparison Results

**Files**: `docs/benchmarks/jq.md`, `docs/benchmarks/yq.md`, `docs/benchmarks/dsv.md`

**When**: Before major releases or after significant optimizations

**Format**: Follow existing table format:

```markdown
### Platform Name (CPU Details)

| Size      | succinctly   | competitor   | Speedup       | succ Mem | comp Mem | Mem Ratio |
|-----------|--------------|--------------|---------------|----------|----------|-----------|
| **1MB**   |   20.5 ms    |  111.2 ms    | **5.4x**      |   10 MB  |   82 MB  | **0.12x** |
```

**Include**:
- Platform details (CPU model, OS, SIMD features)
- Date of benchmark run
- Tool versions (jq/yq version)
- Speedup and memory ratio

#### 3. Cross-Parser Comparison

**Files**: `docs/benchmarks/cross-language.md`, `docs/benchmarks/rust-parsers.md`

**When**: After major changes to JSON parsing or before releases

**Format**: Follow existing table format with all parsers in same table

#### 4. Benchmark Inventory

**File**: `docs/benchmarks/inventory.md`

**When**: Adding new benchmark reports or reorganizing docs

**Update**: Add new sections to the inventory index

### Versioning and Dating

**Always include**:
- **Date**: When benchmark was run (e.g., "2026-01-17")
- **Platform**: CPU model and OS version
- **Tool versions**: For comparison benchmarks (jq-1.6, yq v4.48.1)
- **Build flags**: Any special RUSTFLAGS or features, and — for an A/B comparing two binaries —
  whether `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1` was pinned on both sides (see [A/B
  Benchmarking Method § 9](#a-b-benchmarking-method), #1587). Unstated means default
  `codegen-units=16`, which can add its own ±10-27% independent of the change being measured.

**Example**:
```markdown
**Date**: 2026-01-17
**Platform**: AMD Ryzen 9 7950X (Zen 4, x86_64)
**OS**: Linux 6.6.87.2-microsoft-standard-WSL2
**Build**: `RUSTFLAGS="-C target-cpu=native" cargo build --release` (codegen-units=1, both sides)
```

### Formatting Tables

Use fixed-width alignment for readability:

```markdown
| Size      | succinctly   | competitor   | Speedup       |
|-----------|--------------|--------------|---------------|
| **1KB**   |    2.5 ms    |   58.8 ms    | **24x**       |
| **10KB**  |    2.6 ms    |   59.9 ms    | **23x**       |
```

**Tips**:
- Bold the size column for emphasis
- Bold speedup numbers that exceed baseline
- Right-align numeric columns
- Use consistent units (ms, µs, ns, MiB/s)

---

## Troubleshooting

### Benchmark Won't Run

#### Error: "No such file or directory"

**Cause**: Missing test data

**Fix**:
```bash
# Generate all test data
cargo run --release --features cli -- json generate-suite
cargo run --release --features cli -- yaml generate-suite
cargo run --release --features cli -- dsv generate-suite
```

#### Error: "Command 'jq' not found"

**Cause**: Comparison benchmarks require system tools

**Fix**:
```bash
# Ubuntu/Debian
sudo apt-get install jq

# macOS
brew install jq

# For yq
sudo wget https://github.com/mikefarah/yq/releases/latest/download/yq_linux_amd64 -O /usr/local/bin/yq
sudo chmod +x /usr/local/bin/yq
```

#### Error: "Benchmark takes too long"

**Cause**: Large file benchmarks (100MB+) can take 10+ minutes

**Fix**: Use `--quick` flag or filter to smaller sizes
```bash
cargo bench -- --quick
cargo bench -- "1mb"  # Only 1MB files
```

### Inconsistent Results

#### High Standard Deviation

**Causes**:
- System load (other processes running)
- Thermal throttling
- Power management

**Fix**:
```bash
# Close other applications
# Disable CPU frequency scaling (Linux)
sudo cpupower frequency-set --governor performance

# Run with higher sample size
cargo bench -- --sample-size 100
```

#### Results Vary Between Runs

**Causes**:
- Cache state differences
- Memory allocator non-determinism
- Background processes

**Fix**:
- Run multiple times and average
- Use `--warm-up-time` for longer warm-up
- Check for background processes (`top`, `htop`)

### Cross-Parser Benchmark Issues

#### Error: "Failed to parse with simd-json"

**Cause**: simd-json requires mutable input

**Fix**: Already handled in `bench-compare`, but verify test data is valid JSON

#### Memory Measurements Don't Match Docs

**Cause**: Allocator differences or OS differences

**Fix**: Use tracking allocator in `bench-compare` (already implemented)

### CI Benchmark Failures

#### Timeout

**Cause**: Benchmark takes too long on CI runners

**Fix**: Reduce sample size or filter benchmarks in `.github/workflows/ci.yml`

#### Artifact Upload Failed

**Cause**: File size too large or network issue

**Fix**: Use `--noplot` to skip graphs, check file size

---

## Advanced Topics

### Profiling with Perf

```bash
# Build with debug symbols
cargo build --release

# Record profile
perf record --call-graph=dwarf ./target/release/succinctly jq '.users[]' large.json

# Analyze
perf report
```

### Flame Graphs

```bash
# Install flamegraph
cargo install flamegraph

# Generate flame graph
cargo flamegraph --bench yaml_bench -- --bench
```

### Comparing Multiple Baselines

```bash
# Save baseline before optimization
git checkout main
cargo bench -- --save-baseline main

# Implement optimization
git checkout feature-branch

# Compare
cargo bench -- --baseline main
```

### Custom Benchmark Filters

```bash
# Multiple patterns (OR)
cargo bench -- "rank1|select1"

# Case insensitive
cargo bench -- "(?i)RANK"

# Exclude pattern
cargo bench -- --skip "100mb"
```

---

## See Also

- [docs/benchmarks/inventory.md](../benchmarks/inventory.md) - Complete benchmark report index
- [docs/benchmarks/README.md](../benchmarks/README.md) - Benchmark results overview
- [docs/guides/developer.md](developer.md) - General development workflow
- [docs/guides/release.md](release.md) - Release process (includes benchmark requirements)
- [docs/optimizations/history.md](../optimizations/history.md) - Optimization history and lessons learned
