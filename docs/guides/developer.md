# Developer Guide

[Home](../../) > [Docs](../) > [Guides](./) > Developer Guide

This guide provides detailed information for developers working on succinctly.

## Architecture Overview

Succinctly is organized into a modular hierarchy:

```
src/
├── lib.rs              # Public API, RankSelect trait, Config
├── bits/               # Bitvector implementations
│   ├── mod.rs          # Re-exports
│   ├── bitvec.rs       # BitVec with rank/select
│   ├── rank.rs         # RankDirectory (Poppy 3-level)
│   ├── select.rs       # SelectIndex (sampled)
│   └── popcount.rs     # Population count strategies
├── trees/              # Tree encodings
│   ├── mod.rs          # Re-exports
│   └── bp.rs           # BalancedParens with RangeMin
├── util/               # Internal utilities (not public API)
│   ├── mod.rs
│   ├── broadword.rs    # Bit manipulation primitives
│   ├── table.rs        # Precomputed lookup tables
│   └── simd/           # SIMD utilities
├── binary.rs           # Binary I/O utilities
├── json/               # JSON semi-indexing
│   ├── mod.rs          # Public API
│   ├── locate.rs       # Path location (jq-locate)
│   ├── pfsm_optimized.rs  # Table-driven parser (default)
│   ├── pfsm_tables.rs     # State machine tables
│   ├── standard.rs        # Standard cursor implementation
│   ├── simple.rs          # Simple 3-state cursor
│   └── simd/              # SIMD implementations
│       ├── mod.rs         # Runtime dispatch
│       ├── avx2.rs        # AVX2 (32 bytes/iter)
│       ├── x86.rs         # SSE2 baseline
│       └── neon.rs        # ARM NEON
├── jq/                 # jq query language
│   ├── mod.rs          # Public API
│   ├── expr.rs         # AST definitions
│   ├── parser.rs       # Recursive descent parser
│   └── eval.rs         # Expression evaluator
└── bin/                # CLI tool
    └── succinctly/
        ├── main.rs
        ├── generators.rs
        ├── jq_runner.rs
        └── jq_locate.rs
```

### Public API

```rust
// Recommended imports (explicit module paths)
use succinctly::bits::BitVec;
use succinctly::trees::BalancedParens;
use succinctly::json::JsonIndex;
use succinctly::jq::{parse, eval};

// Convenience re-exports (also at crate root)
use succinctly::{BitVec, BalancedParens, RankSelect};

// Backward compatibility (deprecated)
use succinctly::bp::BalancedParens;  // Use succinctly::trees instead
```

## Core Concepts

### Succinct Data Structures

Succinct data structures use space close to the information-theoretic minimum while supporting fast queries. The key operations are:

- **rank1(i)**: Count 1-bits in positions [0, i) - O(1)
- **select1(k)**: Find position of k-th 1-bit - O(log n)

### Poppy Rank Directory

The rank directory uses a 3-level structure (L0/L1/L2) for O(1) rank queries:

```
L0: Cumulative count every 2^32 bits (64-bit counters)
L1: Cumulative count every 512 bits within L0 block (32-bit)
L2: Cumulative count every 64 bits within L1 block (10-bit, packed)
```

Space overhead: ~3% of bitvector size.

### Select Index

Select uses a sampled index with binary search:

```
Sample: Store position of every 256th 1-bit
Query: Binary search to find block, then scan within block
```

Space overhead: ~1-3% depending on bit density.

### Balanced Parentheses

Trees are encoded as balanced parentheses (1=open, 0=close). The RangeMin structure stores hierarchical min-excess values for O(1) operations:

- `find_close(i)`: Find matching close parenthesis
- `find_open(i)`: Find matching open parenthesis
- `enclose(i)`: Find enclosing parenthesis pair
- `first_child(i)`, `next_sibling(i)`, `parent(i)`: Tree navigation

Space overhead: ~6% of bitvector size.

### JSON Semi-Indexing

JSON documents are converted to two bitvectors for efficient navigation without full parsing. See [parsing/json.md](../parsing/json.md) for detailed implementation documentation covering:

- Semi-index structure (Interest Bits + Balanced Parentheses)
- PFSM table-driven parser (~880 MiB/s on x86_64 AMD Zen 4)
- SIMD character classification
- Navigation cursor API

## Development Workflow

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# With specific features
cargo build --features simd
cargo build --features cli
```

### Testing

```bash
# Run all tests
cargo test

# Run specific module tests
cargo test bitvec
cargo test bp
cargo test json

# Run with large bitvectors (requires ~125MB RAM)
cargo test --features large-tests

# Run property tests
cargo test --test property_tests
cargo test --test properties
```

Two `jq_cli_tests` take 100-150 s each in a debug build, because the cost is the native
stack being filled to the point where ADR-0025's guard refuses (not logic that can be
trimmed). CI runs them in their own `deep-recursion` leg rather than in `cli-gated`;
`scripts/deep-recursion-tests.sh` holds the list for both sides (#3698), so to move another
slow `jq_cli_tests` test there, add its name to that script (it handles that one test target
only). `scripts/deep-recursion-tests.sh run` runs just those tests locally, and
`scripts/test_deep_recursion_tests.py` is the script's own self-test, which the leg runs
first. A plain `cargo test --features cli` still runs everything.

### Coverage

CI line coverage uses the feature set `cli,simd,regex,serde` (see `CLAUDE.md`'s Coverage
section). To reproduce its numbers and the PR patch-coverage list locally:

```bash
cargo llvm-cov --features cli,simd,regex,serde --workspace --summary-only --fail-under-lines 0
cargo llvm-cov report --lcov --output-path target/coverage.lcov
patchcov diff --report target/coverage.lcov
```

The two `jq_cli_tests` listed in `scripts/deep-recursion-tests.sh` are `#[ignore]`d under
`cfg(coverage)` (an attribute on each; cargo-llvm-cov sets the cfg), so `cargo llvm-cov` and the CI
`Coverage` job report them as ignored. They still run in the `deep-recursion` leg and under a plain
`cargo test`. When measured (#3750) they covered no line the rest of the suite did not: 4,605 lines on
their own, none outside the 145,475 the other tests cover. That is a point-in-time result, so repeat it
before giving another test the same attribute, or if a PR touching the native-stack guard shows
uncovered lines. Run both from one checkout (the lcov files hold absolute paths) and compare the two
line sets. (`cargo llvm-cov` does not run doctests on stable, which CI's `cargo test` does; the 52 of them
added no covered line when this was measured.)

```bash
F=cli,simd,regex,serde
A=test_composed_recursion_across_two_defs_errors_not_aborts_1371
B=test_recursion_refuses_before_the_native_stack_runs_out_3262
cargo llvm-cov --features $F --workspace --lcov --output-path rest.info   # Coverage's tests, less its doctests
cargo llvm-cov --features $F --test jq_cli_tests --lcov --output-path two.info \
  -- --ignored --exact $A $B                                              # only the two

covered() { awk '/^SF:/{f=$0} /^DA:/{split(substr($0,4),a,","); if (a[2]>0) print f":"a[1]}' "$1" | sort -u; }
comm -13 <(covered rest.info) <(covered two.info)   # lines only the two cover; empty = nothing lost
```

#### Patchcov migration verification

The shared coverage action uses `patchcov-action@v1` with **patchcov 0.1.1**
(#3873; moved from the omni-dev-named action in #3880, same commit). The other omni-dev
commands still use `.omni-dev/`; coverage alone reads `.patchcov/config.yaml`. Install the pinned tool with `cargo install patchcov --version 0.1.1`.
CI runs `patchcov lint-markers --include 'src/**/*.rs'` before reporting, alongside the
existing pooled-profile check. The migrated source has 361 `tolerate-line`, 21 `tolerate`
and 21 `end` markers across 17 Rust files; every Rust edit changes only the introducer.

Verification on 2026-10-06 used the released macOS ARM64 binaries of omni-dev 0.46.0
and patchcov 0.1.1, and unchanged x86_64 main artifacts from baseline runs
[37457872796](https://github.com/rust-works/succinctly/actions/runs/37457872796)
(`1c71942d0`) and [37458546292](https://github.com/rust-works/succinctly/actions/runs/37458546292)
(`c4a62bfbb`). Both artifacts contain `coverage-head.lcov`; no baseline conversion is needed.

To repeat, download `coverage-baseline-x86_64` from each run with `gh run download`,
and extract each revision with `git archive` into temporary Git repositories. Commit
base then head in one repository with the original markers/config, and in another with
both revisions' marker introducers and config path migrated. Keep every source line in
place. Run the old `omni-dev coverage diff` and new `patchcov diff` with the corresponding
snapshot refs and identical reports/options:

```bash
# BASE_OLD/HEAD_OLD and BASE_NEW/HEAD_NEW are snapshot refs; OLD/NEW are their roots.
# REPORTS holds base/coverage-head.lcov and head/coverage-head.lcov from the two runs.
for format in markdown json; do
  omni-dev coverage diff -C "$OLD" --base-ref "$BASE_OLD" --head-ref "$HEAD_OLD" \
    --baseline-report "$REPORTS/base/coverage-head.lcov" \
    --report "$REPORTS/head/coverage-head.lcov" \
    --strip-prefix /home/runner/work/succinctly/succinctly --all-files -o "$format"
  patchcov diff -C "$NEW" --base-ref "$BASE_NEW" --head-ref "$HEAD_NEW" \
    --baseline-report "$REPORTS/base/coverage-head.lcov" \
    --report "$REPORTS/head/coverage-head.lcov" \
    --strip-prefix /home/runner/work/succinctly/succinctly --all-files -o "$format"
done
```

Both tools reported **266/269 (98.88%) patch coverage**, project totals **94.84% → 94.86%**,
identical per-file deltas, indirect changes and uncovered lines (`src/jq/eval.rs:67206`,
`src/jq/eval.rs:67207`, `src/jq/eval_generic.rs:1063`). Zeroing all added executable `DA:`
records in a copy of the head lcov changed both to **0/269**, with identical uncovered
lists and deltas. Patchcov adds a Markdown exclusion note and a JSON `excluded_files`
object (plus its explanation field); those disclosures are the only output differences.

Mutation checks deliberately force report hit flips so stable/unreachable lines exercise
the masks. Setting `src/jq/value.rs:365` to one hit in the baseline and zero in head
keeps that file's effective delta at **0.00pp** with its `tolerate-line`; removing that
single marker suffix from both source snapshots changes the delta to **−0.02pp**.
For the config check, zero all head `DA:` counts in `src/bits/popcount.rs`, then run
with and without its ignore entry (an empty `--config-dir` is equivalent): the excluded
file reappeared and changed the total from **94.86% to 94.77%**. Keep these mutations
in temporary fixtures.

A third snapshot kept the original baseline source/config and migrated only head.
Patchcov consumed the old lcov unchanged and produced identical project totals,
per-file deltas and indirect changes: head markers drive tolerance masking. Its patch
coverage was **276/613 (45.02%)**, because replacing a marker suffix on an executable
line makes that line newly added in the Git diff. This affects the migration PR itself;
`tolerate` does not forgive newly added executable lines. The patch gate remains
report-only, and subsequent PRs compare migrated source on both sides. Baseline artifact
names, the `cargo llvm-cov` 55% floor and profile pooling stay the same.

#### `warning: N functions have mismatched data`

`cargo llvm-cov report` prints this on every run (41 on a local macOS build, 56 on x86_64
Linux, more on ARM64 Linux), and llvm-cov prints only the count. It is harmless, and
it is not a profile mix-up between baseline and head (#3672, #3649).

- **Cause.** A non-generic `#[inline(always)]` function is codegen'd as its own instrumented
  copy in each crate that inlines it, so the library and the test binaries carry copies of the
  same symbol whose coverage hashes differ. When the merged profile has a record for that
  name under another binary's hash and none under this binary's own, llvm-cov drops this
  binary's copy of the function and counts it as mismatched. A copy that has a record
  matches, so only copies this binary never executed are dropped. Measured: macOS names one
  function (`json::light::word_special_mask`, in 41 test binaries); x86_64 Linux names two
  (`yaml::scalar::could_be_null_or_bool`, 41 binaries; `yaml::end_positions` `get`, 15).
  Changing those three from `#[inline(always)]` to `#[inline]` takes the x86_64 count to 0.
- **Effect on coverage.** None found. The line counts the report shows for an affected
  function are the sum of every record in the profile (`word_special_mask`'s lines read
  2,692,884 = 2,660,434 + 32,450, its two records), so no executed copy is lost. What is
  dropped is a copy with zero executions in that binary.
- **Not fixed by removing the attribute.** `#[inline(always)]` is deliberate on those hot
  paths and a warning is not worth a perf-guard shift.

To name the functions behind the count on any platform, run a coverage pass and then the
script, with no other arguments:

```bash
cargo llvm-cov --no-report --features cli,simd,regex,serde --workspace
scripts/coverage-mismatched-functions.py
```

It reads each instrumented binary's coverage mapping against the merged profile and against a
profile that names none of its functions; per binary, the functions present only in the second
are the mismatched ones. The comparison cannot be made on the combined export, because llvm-cov
merges identical functions across binaries.

### Benchmarking

For comprehensive benchmarking instructions, see [benchmarking.md](benchmarking.md).

Quick reference:
```bash
# Generate test data
cargo run --release --features cli -- json generate-suite
cargo run --release --features cli -- yaml generate-suite

# Run all benchmarks
cargo bench

# Run specific benchmark
cargo bench --bench rank_select
```

### Profiling

For CPU profiling on macOS:

```bash
# Build with debug symbols
cargo build --release

# Profile with Instruments
xcrun xctrace record --template "Time Profiler" --launch -- ./target/release/succinctly jq '.users[]' large.json
```

For Linux with perf:

```bash
cargo build --release
perf record ./target/release/succinctly jq '.users[]' large.json
perf report
```

## SIMD Development

### Adding a New SIMD Implementation

1. Create a new file in `src/json/simd/` (e.g., `avx512.rs`)
2. Implement the character classification function
3. Add runtime detection in `src/json/simd/mod.rs`
4. Update dispatch logic to use new implementation

### Runtime Feature Detection

```rust
#[cfg(target_arch = "x86_64")]
{
    if is_x86_feature_detected!("avx2") {
        // Use AVX2 path
    } else {
        // Use SSE2 baseline
    }
}
```

### Performance Guidelines

1. **Profile before optimizing** - Measure actual bottlenecks
2. **Consider memory bandwidth** - SIMD helps compute-bound, not memory-bound
3. **Test on target hardware** - Zen 4 splits AVX-512 into 2x256-bit ops
4. **Measure end-to-end** - Micro-benchmarks can be misleading
5. **Keep scalar fallback** - Not all CPUs support all features

## Unsafe Code Guidelines

When writing unsafe code:

1. **Minimize scope** - Keep unsafe blocks as small as possible
2. **Document invariants** - Add `// SAFETY:` comments explaining why it's safe
3. **Prefer safe abstractions** - Wrap unsafe in safe APIs when possible

Example:

```rust
// SAFETY: We verified that `i < self.len` at the start of this function,
// and the buffer was allocated with at least `len` elements.
unsafe {
    *self.ptr.add(i)
}
```

## `no_std` Development

The library is `no_std` compatible:

```rust
#![cfg_attr(not(any(test, feature = "std")), no_std)]

extern crate alloc;
use alloc::vec::Vec;
```

Guidelines:
- Use `alloc` instead of `std` for `Vec`, `String`, etc.
- Gate std-only features with `#[cfg(feature = "std")]`
- Test with `--no-default-features` to verify `no_std` works

## Code Style

### Formatting

Use rustfmt with default settings:

```bash
cargo fmt
```

### Linting

Use clippy with all warnings as errors. `--all-features` alone is not
enough: it enables `scalar-yaml` and `portable-popcount`, which compile out
the real YAML SIMD backends and the AVX-512 popcount path respectively, so
neither is ever linted by that invocation alone (see CLAUDE.md's Feature
Flags section for why). Run the invocation(s) matching your architecture,
or just run `scripts/build.sh`/`scripts/test.sh`, which already run all of
them (the feature lists below have one source of truth,
`scripts/lib/clippy-variants.sh` — #3354):

```bash
# Every architecture
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --all-targets --features std,simd,serde,cli,regex,bench-runner,large-tests,mmap-tests -- -D warnings

# ARM64 only, additionally (lints neon.rs/broadword.rs, which don't compile on x86_64 at all)
cargo clippy --all-targets --features std,simd,broadword-yaml,serde,cli,regex,bench-runner,large-tests,mmap-tests -- -D warnings
```

### Documentation

- All public items must have doc comments
- Include examples for complex APIs
- Use `#[doc(hidden)]` for internal-but-public items

```rust
/// Counts 1-bits in positions [0, i).
///
/// # Examples
///
/// ```
/// use succinctly::{BitVec, RankSelect};
///
/// let bv = BitVec::from_words(vec![0b1111], 4);
/// assert_eq!(bv.rank1(2), 2);
/// ```
fn rank1(&self, i: usize) -> usize;
```

## Release Process

See [release.md](release.md) for detailed release instructions.

Quick summary:
1. Update version in `Cargo.toml`
2. Run `python3 scripts/changelog.py collect --version X.Y.Z` to assemble `CHANGELOG.md` from the `changelog.d/` fragments
3. Create annotated tag: `git tag -a v0.1.0 -m "Release v0.1.0"`
4. Push: `git push origin v0.1.0`
5. CI will build and publish

## Getting Help

- **Questions**: Open a GitHub issue or discussion
- **Bugs**: Open a GitHub issue with reproduction steps
- **Features**: Open a GitHub issue describing the use case
