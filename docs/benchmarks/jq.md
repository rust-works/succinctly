# jq vs succinctly Comparison Benchmarks

[Home](../../) > [Docs](../) > [Benchmarks](./) > jq

Comprehensive benchmarks comparing `succinctly jq .` vs `jq .` for JSON formatting/printing.

**See also**: [JsonIndex wiki page](../parsing/json-index.md) for architecture overview and academic references.

Results are provided for ARM (Apple M4 Pro, Apple M1 Max, AWS Graviton) and x86_64 (AMD Zen 4) platforms. The AMD Ryzen 9 7950X and Apple M4 Pro sections were re-run against jq 1.7.1 in #5336 and include a query mix beyond `.`; the other platforms' sections were measured against the jq version named in each, so do not compare speedups across sections whose jq version differs.

The AMD Ryzen 9 7950X and Apple M4 Pro sections open with a generated *Harness comparison* block: the pinned jq 1.7.1, interleaved repetitions and output-gated rows of `succinctly dev compare jq`, quoted from the committed run in [docs/compare/](../compare/jq.md) and checked against it by a test. Where a hand-written figure below disagrees with one of those blocks, the block is the measurement.

## Architectural Differences

Before reviewing the benchmarks, it's important to understand the fundamental differences between these tools:

| Aspect           | succinctly                        | jq                              |
|------------------|-----------------------------------|---------------------------------|
| **Architecture** | Semi-indexer                      | Full parser                     |
| **Approach**     | Build structural index, lazy eval | Parse entire document into DOM  |
| **Memory model** | ~3-6% index overhead              | Full DOM in memory (6-8x input) |

**What this means for benchmarks**: The performance and memory differences shown below come from architectural choices:

1. **Semi-indexing**: succinctly builds a lightweight structural index (balanced parentheses + interest bits) instead of a full DOM tree. This takes less time and memory.

2. **Lazy evaluation**: Values are only materialized when accessed. For identity queries (`.`), this means streaming output without intermediate allocations.

3. **Streaming output**: succinctly writes JSON directly to output without building intermediate String objects.

succinctly supports most common jq queries including path navigation, array slicing, filtering, and piping. See [CLAUDE.md](../../CLAUDE.md) for supported operators.

For detailed architectural documentation, see [Semi-Indexing Architecture](../architecture/semi-indexing.md).

## Methodology

Benchmarks measure:
- **Wall time**: Total elapsed time
- **Peak memory**: Maximum resident set size (RSS)
- **Output correctness**: MD5 hash comparison ensures identical output

Run with:
```bash
cargo build --release --features bench-runner
./target/release/succinctly bench run jq_bench
```

`bench run` times `.` over every pattern. For the query mix over the `users` pattern (#5336):

```bash
./target/release/succinctly dev bench jq --patterns users --queries users_mix --runs 5
```

Those two runners time all of jq's runs and then all of succinctly's, and use whichever `jq` is on `PATH`. The *Harness comparison* blocks come from `succinctly dev compare jq` instead ([comparison/jq/README.md](../../comparison/jq/README.md)). It builds jq 1.7.1 and 1.8.1 from source, interleaves the tools within each repetition, and times a row only when the outputs are identical. `--write-docs` regenerates a platform's block with its report, and `--render-docs` re-renders every block from the committed runs without measuring anything:

```bash
cargo build --release --features cli,simd,regex,serde,bench-runner
# clean checkout, idle machine, on AC power; --build-jq builds the pinned jqs on first use
./target/release/succinctly dev compare jq --quick --build-jq --write-docs
# or time the shipped build rather than the harness, as the M4 Pro block does (#5551)
cargo build --release --features cli,simd,regex,serde --target-dir target/subject
./target/release/succinctly dev compare jq --quick --build-jq --write-docs \
  --subject target/subject/release/succinctly --subject-features cli,simd,regex,serde
```

Each block names the binary it timed. `--write-docs` refuses, before measuring anything, on a machine whose CPU is not the one named on the `**CPU**:` line of the section holding its platform's block: the M1 Max and the M4 Pro are both `macos-aarch64`.

---

# AMD Ryzen 9 7950X (Zen 4, x86_64)

**CPU**: AMD Ryzen 9 7950X 16-Core Processor
**OS**: Linux 6.18.54.1-microsoft-standard-WSL2 (WSL2)
**jq version**: jq-1.7.1
**succinctly**: Built with `--release --features cli,bench-runner` from `main` @ `4395a966a` plus the `users_mix` query set added in the same PR as this section (#5336); eval code is `main`'s
**Date**: 2026-10-27 (re-run against jq 1.7.1, #5336)
**Method**: median of 3 timed runs after 1 warmup for `.`; median of 5 for the query mix; output MD5 identical to jq on every row

## Harness comparison (`dev compare jq`)

<!-- compare:jq-linux-x86_64 -->
Generated by `succinctly dev compare jq --write-docs` from [jq-linux-x86_64.json](../compare/jq-linux-x86_64.json); do not edit. Full report: [jq-linux-x86_64.md](../compare/jq-linux-x86_64.md); every platform: [jq.md](../compare/jq.md).

**Run**: `./target/release/succinctly dev compare jq --quick` on AMD Ryzen 9 7950X 16-Core Processor (Ubuntu 22.04.5 LTS), succinctly `83b287ef9`, run `20261031_071057_83b287ef9`. jq-1.7.1 is built from source by the harness; the tools are interleaved within each of 5 repetitions, and a row is timed only when the outputs are identical.

**Timed binary**: the harness itself, built with `cli,simd,regex,serde`.

Geomean speedup over jq-1.7.1: **2.03x** across 52 timed rows; jq-1.7.1 is faster on 8 of them. Speedup is jq-1.7.1's time over succinctly jq's, so below 1.00x jq is faster; memory is succinctly jq's peak RSS over jq-1.7.1's. Times are medians, except where marked †: both times and the ratio are then each tool's fastest run, which run-to-run noise moves least on a long row.

| Query                    | Input             | jq-1.7.1 (ms) | succinctly jq (ms) |     Speedup | Memory ratio |
|--------------------------|-------------------|---------------|--------------------|-------------|--------------|
| `startup`                | none              |           1.3 |                1.4 |       0.90x |        3.12x |
| `identity`               | users-1mb         |          16.7 |                8.0 |  **2.10x**† |        1.18x |
| `identity`               | numbers-1mb       |          21.9 |                6.7 |  **3.28x**† |        1.04x |
| `identity`               | strings-1mb       |          12.4 |                4.5 |   **2.77x** |        3.04x |
| `identity`               | wide-1mb          |          30.5 |               12.5 |  **2.43x**† |        0.67x |
| `identity`               | nested-1mb        |          13.8 |                4.5 |   **3.05x** |        2.20x |
| `identity`               | comprehensive-1mb |          30.9 |               11.0 |  **2.82x**† |        0.67x |
| `identity_compact`       | users-1mb         |          14.8 |                3.1 |  **4.71x**† |        1.00x |
| `identity_compact`       | numbers-1mb       |          20.2 |                3.6 |  **5.63x**† |        0.85x |
| `identity_compact`       | strings-1mb       |          11.8 |                3.2 |   **3.62x** |        2.71x |
| `identity_compact`       | wide-1mb          |          29.8 |                6.1 |  **4.88x**† |        0.61x |
| `identity_compact`       | nested-1mb        |          13.5 |                3.4 |   **3.95x** |        1.92x |
| `identity_compact`       | comprehensive-1mb |          25.1 |               10.5 |  **2.38x**† |        0.60x |
| `users_field`            | users-1mb         |           9.1 |                3.1 |   **2.95x** |        1.20x |
| `users_length`           | users-1mb         |           9.6 |                3.3 |   **2.88x** |        1.29x |
| `users_names`            | users-1mb         |           9.9 |                5.6 |   **1.78x** |        1.19x |
| `users_map_select`       | users-1mb         |          11.2 |                7.3 |   **1.54x** |        1.41x |
| `users_sort_by`          | users-1mb         |          15.4 |               10.3 |  **1.49x**† |        1.39x |
| `users_group_by`         | users-1mb         |          16.0 |               10.1 |  **1.59x**† |        1.36x |
| `users_unique_by`        | users-1mb         |          16.1 |                9.9 |  **1.63x**† |        1.40x |
| `users_reduce`           | users-1mb         |          10.3 |                8.4 |   **1.23x** |        1.43x |
| `users_to_entries`       | users-1mb         |           9.1 |                3.2 |   **2.80x** |        1.40x |
| `users_csv`              | users-1mb         |          15.8 |               12.2 |  **1.30x**† |        1.27x |
| `users_regex`            | users-1mb         |          15.6 |                8.4 |  **1.86x**† |        1.67x |
| `users_ascii_downcase`   | users-1mb         |          26.6 |                7.1 |  **3.75x**† |        1.43x |
| `users_first_early_exit` | users-1mb         |           8.8 |                3.1 |   **2.88x** |        1.30x |
| `users_limit_early_exit` | users-1mb         |           9.2 |                3.0 |   **3.06x** |        1.34x |
| `users_assign_update`    | users-1mb         |           9.2 |               14.3 |       0.64x |        2.24x |
| `users_del`              | users-1mb         |           9.5 |               14.3 |       0.66x |        2.08x |
| `users_paths`            | users-1mb         |          33.3 |               20.4 |  **1.63x**† |        1.68x |
| `users_tojson_roundtrip` | users-1mb         |          22.0 |               29.9 |      0.74x† |        2.25x |
| `users_walk`             | users-1mb         |          77.5 |               25.1 |  **3.08x**† |        2.32x |
| `users_slurp`            | users-1mb         |           9.0 |                3.7 |   **2.44x** |        1.31x |
| `users_stream`           | users-1mb         |          26.5 |               41.0 |      0.65x† |       10.55x |
| `numbers_add`            | numbers-1mb       |          27.4 |               10.4 |  **2.63x**† |        1.61x |
| `numbers_sort`           | numbers-1mb       |          44.8 |               29.9 |  **1.50x**† |        1.79x |
| `numbers_max`            | numbers-1mb       |          12.9 |               14.2 |       0.91x |        2.38x |
| `numbers_map_floor`      | numbers-1mb       |          32.9 |               23.7 |  **1.39x**† |        1.83x |
| `strings_length`         | strings-1mb       |           7.9 |                5.4 |   **1.47x** |        3.41x |
| `strings_upcase`         | strings-1mb       |         328.3 |                9.6 | **34.27x**† |        2.86x |
| `strings_split`          | strings-1mb       |          11.8 |               12.5 |       0.94x |        3.63x |
| `strings_test`           | strings-1mb       |          12.3 |                8.7 |   **1.42x** |        3.61x |
| `wide_keys`              | wide-1mb          |          30.2 |                7.6 |  **4.00x**† |        0.63x |
| `wide_to_entries`        | wide-1mb          |          61.2 |               63.8 |      0.96x† |        1.27x |
| `wide_map_values`        | wide-1mb          |         125.5 |               58.1 |  **2.16x**† |        2.06x |
| `wide_add_values`        | wide-1mb          |          37.1 |               32.6 |  **1.14x**† |        1.45x |
| `wide_with_entries`      | wide-1mb          |         305.6 |              163.9 |  **1.86x**† |        1.42x |
| `nested_paths`           | nested-1mb        |           6.3 |                3.7 |   **1.68x** |        2.31x |
| `nested_recurse`         | nested-1mb        |           6.1 |                4.3 |   **1.43x** |        2.34x |
| `comprehensive_paths`    | comprehensive-1mb |          77.8 |               43.4 |  **1.79x**† |        1.41x |
| `comprehensive_tostream` | comprehensive-1mb |         157.9 |               80.2 |  **1.97x**† |        1.87x |
| `comprehensive_numbers`  | comprehensive-1mb |          57.7 |               27.4 |  **2.11x**† |        1.82x |
<!-- /compare:jq-linux-x86_64 -->

The sections below are `bench run jq_bench` and `dev bench jq` results, measured by the method this section's header describes: all jq runs, then all succinctly runs, against the `jq` on `PATH`. That is not the interleaved method of [benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method), so where a figure below disagrees with the block above, the block is the measurement.

## Query mix (users pattern)

The `users_mix` query set (`succinctly dev bench jq --patterns users --queries users_mix`) over the `users` pattern (`{"users":[{id,name,email,age,active,score},...]}`), one process per query: speedup is jq's wall time over succinctly's, so **below 1.0x succinctly is slower**. Memory ratio is succinctly's peak RSS over jq's at 100mb.

| Query                                      | jq (100mb) | succinctly (100mb) | 100mb    | 10mb     | 1mb      | 100kb    | 10kb | Mem Ratio (100mb) |
|--------------------------------------------|------------|--------------------|----------|----------|----------|----------|------|-------------------|
| `.`                                        |      1.83s |            737.6ms | **2.5x** | **2.5x** | **2.2x** | **1.3x** | 1.0x |             0.30x |
| `.users[0].name`                           |    980.4ms |            154.2ms | **6.4x** | **5.6x** | **3.0x** | **1.4x** | 1.0x |             0.16x |
| `.users \| length`                         |    980.4ms |            156.3ms | **6.3x** | **5.8x** | **2.9x** | **1.2x** | 1.0x |             0.16x |
| `[.users[] \| select(.age > 50) \| .name]` |      1.30s |            646.4ms | **2.0x** | **1.9x** | **1.5x** | **1.2x** | 1.0x |             0.28x |
| `.users \| map(.score)`                    |      1.28s |            394.9ms | **3.2x** | **3.1x** | **2.0x** | **1.1x** | 1.0x |             0.24x |
| `.users \| sort_by(.age)`                  |      4.44s |              1.88s | **2.4x** | **2.1x** | **1.6x** | **1.1x** | 1.0x |             0.52x |
| `.users[] \| "\(.name) <\(.email)>"`       |      1.43s |            864.1ms | **1.7x** | **1.6x** | **1.5x** | **1.1x** | 1.0x |             0.16x |
| `.users \| group_by(.age)`                 |      4.29s |              2.01s | **2.1x** | **1.9x** | **1.5x** | **1.1x** | 1.0x |             0.48x |
| `.users \| INDEX(.id)`                     |      2.56s |              3.90s |     0.7x |     0.6x |     0.7x |     0.8x | 1.0x |             2.26x |

## Pattern: arrays

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    6.02s |    **1.59s** |      **3.8x** |    4 GB |   385 MB |      0.11x |
| **10mb**  |  608.6ms |  **164.7ms** |      **3.7x** |  367 MB |    49 MB |      0.13x |
| **1mb**   |   61.0ms |   **17.7ms** |      **3.4x** |   38 MB |    14 MB |      0.38x |
| **100kb** |    7.3ms |    **3.4ms** |      **2.1x** |    5 MB |    12 MB |      2.40x |
| **10kb**  |    2.2ms |    **1.9ms** |      **1.2x** |    2 MB |    12 MB |      5.92x |
| **1kb**   |    1.8ms |      1.8ms   |          1.0x |    2 MB |    11 MB |      5.63x |

## Pattern: comprehensive

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.39s |    **1.05s** |      **3.2x** |    1 GB |   297 MB |      0.20x |
| **10mb**  |  335.2ms |  **105.0ms** |      **3.2x** |  149 MB |    40 MB |      0.27x |
| **1mb**   |   34.9ms |   **12.5ms** |      **2.8x** |   17 MB |    14 MB |      0.85x |
| **100kb** |    5.0ms |    **3.1ms** |      **1.6x** |    3 MB |    12 MB |      3.98x |
| **10kb**  |    2.1ms |    **2.0ms** |      **1.0x** |    2 MB |    12 MB |      5.23x |
| **1kb**   |    1.8ms |      1.9ms   |          0.9x |    2 MB |    11 MB |      4.96x |

## Pattern: literals

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    2.44s |    **1.08s** |      **2.3x** |  550 MB |   331 MB |      0.60x |
| **10mb**  |  241.7ms |  **114.1ms** |      **2.1x** |   50 MB |    43 MB |      0.86x |
| **1mb**   |   25.1ms |   **13.0ms** |      **1.9x** |    7 MB |    15 MB |      2.29x |
| **100kb** |    4.1ms |    **3.0ms** |      **1.4x** |    3 MB |    12 MB |      4.42x |
| **10kb**  |    2.0ms |    **1.9ms** |      **1.0x** |    2 MB |    12 MB |      5.05x |
| **1kb**   |    1.7ms |      1.8ms   |          1.0x |    2 MB |    12 MB |      5.88x |

## Pattern: mixed

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |  465.8ms |  **158.8ms** |      **2.9x** |  218 MB |    51 MB |      0.24x |
| **10mb**  |   45.8ms |   **17.0ms** |      **2.7x** |   23 MB |    15 MB |      0.64x |
| **1mb**   |    5.8ms |    **3.3ms** |      **1.7x** |    4 MB |    12 MB |      3.45x |
| **100kb** |    2.1ms |    **1.9ms** |      **1.1x** |    2 MB |    12 MB |      5.14x |
| **10kb**  |    1.9ms |      1.9ms   |          1.0x |    2 MB |    12 MB |      5.77x |
| **1kb**   |    1.8ms |      1.8ms   |          1.0x |    2 MB |    12 MB |      5.30x |

## Pattern: nested

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    1.40s |  **384.1ms** |      **3.7x** |  202 MB |   230 MB |      1.14x |
| **10mb**  |  144.5ms |   **39.0ms** |      **3.7x** |   22 MB |    33 MB |      1.49x |
| **1mb**   |   15.7ms |    **5.3ms** |      **3.0x** |    4 MB |    14 MB |      3.17x |
| **100kb** |    3.2ms |    **2.2ms** |      **1.4x** |    2 MB |    11 MB |      5.02x |
| **10kb**  |    1.9ms |    **1.8ms** |      **1.1x** |    2 MB |    12 MB |      5.08x |
| **1kb**   |    1.8ms |    **1.8ms** |      **1.0x** |    2 MB |    12 MB |      5.14x |

## Pattern: numbers

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    2.42s |  **614.4ms** |      **3.9x** |  855 MB |   250 MB |      0.29x |
| **10mb**  |  236.2ms |   **60.8ms** |      **3.9x** |   87 MB |    35 MB |      0.41x |
| **1mb**   |   25.2ms |    **8.0ms** |      **3.1x** |   10 MB |    13 MB |      1.34x |
| **100kb** |    3.8ms |    **2.5ms** |      **1.5x** |    2 MB |    12 MB |      5.30x |
| **10kb**  |    2.0ms |    **1.9ms** |      **1.1x** |    2 MB |    11 MB |      5.02x |
| **1kb**   |    1.8ms |      1.8ms   |          1.0x |    2 MB |    12 MB |      5.09x |

## Pattern: pathological

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    6.70s |    **2.38s** |      **2.8x** |    5 GB |   513 MB |      0.11x |
| **10mb**  |  670.4ms |  **244.7ms** |      **2.7x** |  472 MB |    62 MB |      0.13x |
| **1mb**   |   68.4ms |   **25.5ms** |      **2.7x** |   49 MB |    16 MB |      0.33x |
| **100kb** |    7.9ms |    **4.2ms** |      **1.9x** |    6 MB |    12 MB |      2.06x |
| **10kb**  |    2.2ms |    **2.0ms** |      **1.1x** |    2 MB |    12 MB |      5.15x |
| **1kb**   |    1.8ms |      1.8ms   |          1.0x |    2 MB |    12 MB |      5.11x |

## Pattern: strings

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    1.30s |  **354.2ms** |      **3.7x** |  130 MB |   208 MB |      1.60x |
| **10mb**  |  128.9ms |   **36.0ms** |      **3.6x** |   14 MB |    31 MB |      2.21x |
| **1mb**   |   14.2ms |    **5.3ms** |      **2.7x** |    3 MB |    14 MB |      4.92x |
| **100kb** |    3.0ms |    **2.2ms** |      **1.3x** |    2 MB |    12 MB |      6.10x |
| **10kb**  |    1.8ms |    **1.8ms** |      **1.0x** |    2 MB |    12 MB |      5.67x |
| **1kb**   |    1.8ms |      1.8ms   |          1.0x |    2 MB |    12 MB |      5.28x |

## Pattern: unicode

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    1.56s |  **544.1ms** |      **2.9x** |  297 MB |   252 MB |      0.85x |
| **10mb**  |  150.2ms |   **53.5ms** |      **2.8x** |   31 MB |    36 MB |      1.17x |
| **1mb**   |   16.5ms |    **7.0ms** |      **2.4x** |    5 MB |    14 MB |      2.85x |
| **100kb** |    3.1ms |    **2.3ms** |      **1.3x** |    2 MB |    12 MB |      5.12x |
| **10kb**  |    1.9ms |    **1.9ms** |      **1.0x** |    2 MB |    12 MB |      5.11x |
| **1kb**   |    1.8ms |      1.8ms   |          1.0x |    2 MB |    12 MB |      5.19x |

## Pattern: users

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    1.84s |  **735.5ms** |      **2.5x** |  663 MB |   201 MB |      0.30x |
| **10mb**  |  181.8ms |   **71.2ms** |      **2.6x** |   68 MB |    30 MB |      0.44x |
| **1mb**   |   19.8ms |    **9.3ms** |      **2.1x** |    8 MB |    13 MB |      1.67x |
| **100kb** |    3.3ms |    **2.6ms** |      **1.3x** |    2 MB |    12 MB |      5.19x |
| **10kb**  |    1.9ms |    **1.9ms** |      **1.0x** |    2 MB |    12 MB |      5.23x |
| **1kb**   |    1.8ms |      1.9ms   |          1.0x |    2 MB |    12 MB |      5.21x |

## Pattern: wide

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    4.17s |    **1.09s** |      **3.8x** |    1 GB |   262 MB |      0.19x |
| **10mb**  |  348.3ms |  **112.4ms** |      **3.1x** |  143 MB |    44 MB |      0.31x |
| **1mb**   |   36.0ms |   **14.0ms** |      **2.6x** |   18 MB |    16 MB |      0.89x |
| **100kb** |    4.9ms |    **3.1ms** |      **1.6x** |    3 MB |    12 MB |      3.71x |
| **10kb**  |    2.1ms |    **2.0ms** |      **1.1x** |    2 MB |    12 MB |      5.36x |
| **1kb**   |    1.8ms |    **1.8ms** |      **1.0x** |    2 MB |    12 MB |      5.18x |

---

# ARM Neoverse-V2 (AWS Graviton 4, aarch64)

**CPU**: ARM Neoverse-V2 (AWS Graviton 4)
**OS**: Linux 6.14.0-1018-aws
**jq version**: jq-1.8.1
**succinctly**: Built with `--release --features bench-runner`
**SIMD**: NEON (128-bit), SVE2 (128-bit vectors), SVEBITPERM (BDEP/BEXT)
**Date**: 2026-01-29

## Summary - Comprehensive Pattern

| Size       | succinctly | jq       | Speedup  | succ Mem | jq Mem  | Mem Ratio |
|------------|------------|----------|----------|----------|---------|-----------|
| **1KB**    | 1.6 ms     | 2.2 ms   | **1.4x** | 4 MB     | 2 MB    | 2.13x     |
| **10KB**   | 1.8 ms     | 2.6 ms   | **1.4x** | 4 MB     | 2 MB    | 2.13x     |
| **100KB**  | 4.3 ms     | 6.3 ms   | **1.5x** | 4 MB     | 3 MB    | 1.37x     |
| **1MB**    | 28.0 ms    | 46.1 ms  | **1.6x** | 5 MB     | 17 MB   | 0.31x     |
| **10MB**   | 270.0 ms   | 434.2 ms | **1.6x** | 14 MB    | 149 MB  | 0.10x     |
| **100MB**  | 2.74 s     | 4.34 s   | **1.6x** | 107 MB   | 1 GB    | 0.07x     |

## Pattern: arrays

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    8.06s |    **5.06s** |      **1.6x** |    4 GB |   473 MB |      0.13x |
| **10mb**  |  807.4ms |  **489.8ms** |      **1.6x** |  367 MB |    51 MB |      0.14x |
| **1mb**   |   83.6ms |   **48.9ms** |      **1.7x** |   38 MB |     9 MB |      0.23x |
| **100kb** |    9.9ms |    **6.1ms** |      **1.6x** |    5 MB |     4 MB |      0.87x |
| **10kb**  |    2.7ms |    **2.0ms** |      **1.4x** |    2 MB |     4 MB |      2.01x |
| **1kb**   |    2.2ms |    **1.6ms** |      **1.3x** |    2 MB |     4 MB |      2.01x |

## Pattern: comprehensive

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    4.34s |    **2.74s** |      **1.6x** |    1 GB |   107 MB |      0.07x |
| **10mb**  |  434.2ms |  **270.0ms** |      **1.6x** |  149 MB |    14 MB |      0.10x |
| **1mb**   |   46.1ms |   **28.0ms** |      **1.6x** |   17 MB |     5 MB |      0.31x |
| **100kb** |    6.3ms |    **4.3ms** |      **1.5x** |    3 MB |     4 MB |      1.37x |
| **10kb**  |    2.6ms |    **1.8ms** |      **1.4x** |    2 MB |     4 MB |      2.13x |
| **1kb**   |    2.2ms |    **1.6ms** |      **1.4x** |    2 MB |     4 MB |      2.13x |

## Pattern: literals

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.07s |    **3.00s** |      **1.0x** |  550 MB |   133 MB |      0.24x |
| **10mb**  |  307.0ms |  **292.4ms** |      **1.1x** |   50 MB |    17 MB |      0.33x |
| **1mb**   |   31.8ms |   **29.4ms** |      **1.1x** |    6 MB |     5 MB |      0.80x |
| **100kb** |    5.0ms |    **4.2ms** |      **1.2x** |    3 MB |     4 MB |      1.51x |
| **10kb**  |    2.3ms |    **1.8ms** |      **1.3x** |    2 MB |     4 MB |      2.00x |
| **1kb**   |    2.2ms |    **1.6ms** |      **1.3x** |    2 MB |     4 MB |      2.01x |

## Pattern: mixed

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |  591.8ms |  **368.1ms** |      **1.6x** |  217 MB |    20 MB |      0.09x |
| **10mb**  |   59.8ms |   **36.5ms** |      **1.6x** |   23 MB |     5 MB |      0.23x |
| **1mb**   |    7.3ms |    **4.9ms** |      **1.5x** |    4 MB |     4 MB |      1.19x |
| **100kb** |    2.5ms |    **1.9ms** |      **1.4x** |    2 MB |     4 MB |      2.01x |
| **10kb**  |    2.2ms |    **1.6ms** |      **1.3x** |    2 MB |     4 MB |      2.00x |
| **1kb**   |    2.1ms |    **1.6ms** |      **1.3x** |    2 MB |     4 MB |      2.01x |

## Pattern: nested

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    1.85s |  **666.0ms** |      **2.8x** |  202 MB |   226 MB |      1.12x |
| **10mb**  |  187.1ms |   **65.0ms** |      **2.9x** |   22 MB |    26 MB |      1.18x |
| **1mb**   |   20.3ms |    **7.9ms** |      **2.6x** |    4 MB |     6 MB |      1.49x |
| **100kb** |    3.9ms |    **2.2ms** |      **1.8x** |    2 MB |     4 MB |      1.95x |
| **10kb**  |    2.3ms |    **1.6ms** |      **1.4x** |    2 MB |     4 MB |      2.00x |
| **1kb**   |    2.1ms |    **1.5ms** |      **1.4x** |    2 MB |     4 MB |      2.01x |

## Pattern: numbers

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.14s |    **1.75s** |      **1.8x** |  855 MB |   119 MB |      0.14x |
| **10mb**  |  311.1ms |  **172.3ms** |      **1.8x** |   87 MB |    15 MB |      0.18x |
| **1mb**   |   33.3ms |   **18.2ms** |      **1.8x** |   10 MB |     5 MB |      0.51x |
| **100kb** |    4.9ms |    **3.2ms** |      **1.5x** |    2 MB |     4 MB |      1.79x |
| **10kb**  |    2.4ms |    **1.7ms** |      **1.4x** |    2 MB |     4 MB |      2.01x |
| **1kb**   |    2.2ms |    **1.6ms** |      **1.3x** |    2 MB |     4 MB |      2.01x |

## Pattern: pathological

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    7.90s |    **4.98s** |      **1.6x** |    5 GB |   279 MB |      0.06x |
| **10mb**  |  792.5ms |  **486.4ms** |      **1.6x** |  472 MB |    32 MB |      0.07x |
| **1mb**   |   82.4ms |   **47.5ms** |      **1.7x** |   49 MB |     7 MB |      0.14x |
| **100kb** |    9.5ms |    **5.9ms** |      **1.6x** |    6 MB |     4 MB |      0.72x |
| **10kb**  |    2.7ms |    **1.9ms** |      **1.4x** |    2 MB |     4 MB |      2.01x |
| **1kb**   |    2.2ms |    **1.7ms** |      **1.3x** |    2 MB |     4 MB |      2.00x |

## Pattern: strings

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    1.76s |  **655.9ms** |      **2.7x** |  130 MB |   111 MB |      0.86x |
| **10mb**  |  175.2ms |   **66.4ms** |      **2.6x** |   14 MB |    15 MB |      1.04x |
| **1mb**   |   19.1ms |    **8.1ms** |      **2.4x** |    3 MB |     5 MB |      1.89x |
| **100kb** |    3.7ms |    **2.2ms** |      **1.7x** |    2 MB |     4 MB |      2.01x |
| **10kb**  |    2.3ms |    **1.6ms** |      **1.4x** |    2 MB |     4 MB |      2.01x |
| **1kb**   |    2.2ms |    **1.7ms** |      **1.3x** |    2 MB |     4 MB |      2.00x |

## Pattern: unicode

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    2.05s |    **1.33s** |      **1.5x** |  297 MB |   127 MB |      0.43x |
| **10mb**  |  208.4ms |  **131.6ms** |      **1.6x** |   31 MB |    16 MB |      0.52x |
| **1mb**   |   22.6ms |   **14.3ms** |      **1.6x** |    5 MB |     5 MB |      1.03x |
| **100kb** |    4.0ms |    **2.8ms** |      **1.4x** |    2 MB |     4 MB |      2.07x |
| **10kb**  |    2.2ms |    **1.7ms** |      **1.3x** |    2 MB |     4 MB |      2.00x |
| **1kb**   |    2.2ms |    **1.6ms** |      **1.4x** |    2 MB |     4 MB |      2.01x |

## Pattern: users

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    2.49s |    **1.52s** |      **1.6x** |  663 MB |    90 MB |      0.14x |
| **10mb**  |  248.1ms |  **148.8ms** |      **1.7x** |   68 MB |    12 MB |      0.18x |
| **1mb**   |   26.4ms |   **15.8ms** |      **1.7x** |    8 MB |     5 MB |      0.58x |
| **100kb** |    4.2ms |    **2.9ms** |      **1.5x** |    2 MB |     4 MB |      2.00x |
| **10kb**  |    2.3ms |    **1.7ms** |      **1.4x** |    2 MB |     4 MB |      2.01x |
| **1kb**   |    2.2ms |    **1.5ms** |      **1.4x** |    2 MB |     4 MB |      1.97x |

## Key Findings (ARM Neoverse-V2)

- **Speedup range**: 1.0x - 2.9x faster than jq across all patterns
- **Best performance**: Nested structures (2.8-2.9x on 10-100MB) and strings (2.6-2.7x)
- **Memory efficiency**: 6-14% of jq's memory on large files (100MB pathological: 0.06x)
- **Consistent speedup**: 1.5-1.8x across most patterns at 1MB+
- **NEON SIMD**: Effective use of ARM NEON for string scanning
- **SVE2 support**: Platform supports SVE2 with SVEBITPERM (BDEP/BEXT for DSV parsing)

---

# ARM Neoverse-V1 (AWS Graviton 3, aarch64)

**CPU**: ARM Neoverse-V1 (8 cores)
**OS**: Linux 6.14.0-1018-aws
**jq version**: jq-1.6
**succinctly**: Built with `--release --features cli`
**SIMD**: NEON (128-bit), SVE (256-bit vectors)
**Date**: 2026-01-29

## Summary - Comprehensive Pattern

| Size       | succinctly | jq       | Speedup  | succ Mem | jq Mem  | Mem Ratio |
|------------|------------|----------|----------|----------|---------|-----------|
| **1KB**    | 1.7 ms     | 2.3 ms   | **1.3x** | 4 MB     | 2 MB    | 2.19x     |
| **10KB**   | 2.0 ms     | 2.7 ms   | **1.3x** | 4 MB     | 2 MB    | 2.19x     |
| **100KB**  | 4.6 ms     | 6.6 ms   | **1.4x** | 4 MB     | 3 MB    | 1.41x     |
| **1MB**    | 28.2 ms    | 46.6 ms  | **1.7x** | 5 MB     | 17 MB   | 0.32x     |
| **10MB**   | 270.0 ms   | 437.8 ms | **1.6x** | 15 MB    | 149 MB  | 0.10x     |
| **100MB**  | 2.74 s     | 4.41 s   | **1.6x** | 107 MB   | 1 GB    | 0.07x     |

## Pattern: arrays

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    8.06s |    **5.06s** |      **1.6x** |    4 GB |   473 MB |      0.13x |
| **10mb**  |  808.7ms |  **492.4ms** |      **1.6x** |  367 MB |    51 MB |      0.14x |
| **1mb**   |   83.8ms |   **49.5ms** |      **1.7x** |   38 MB |     9 MB |      0.23x |
| **100kb** |   10.3ms |    **6.3ms** |      **1.6x** |    5 MB |     5 MB |      0.90x |
| **10kb**  |    2.8ms |    **2.1ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |
| **1kb**   |    2.3ms |    **1.7ms** |      **1.3x** |    2 MB |     4 MB |      2.06x |

## Pattern: comprehensive

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    4.41s |    **2.74s** |      **1.6x** |    1 GB |   107 MB |      0.07x |
| **10mb**  |  437.8ms |  **270.0ms** |      **1.6x** |  149 MB |    15 MB |      0.10x |
| **1mb**   |   46.6ms |   **28.2ms** |      **1.7x** |   17 MB |     5 MB |      0.32x |
| **100kb** |    6.6ms |    **4.6ms** |      **1.4x** |    3 MB |     4 MB |      1.41x |
| **10kb**  |    2.7ms |    **2.0ms** |      **1.3x** |    2 MB |     4 MB |      2.19x |
| **1kb**   |    2.3ms |    **1.7ms** |      **1.3x** |    2 MB |     4 MB |      2.19x |

## Pattern: literals

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.06s |    **3.02s** |      **1.0x** |  550 MB |   133 MB |      0.24x |
| **10mb**  |  304.8ms |  **293.8ms** |      **1.0x** |   50 MB |    17 MB |      0.34x |
| **1mb**   |   32.0ms |   **29.8ms** |      **1.1x** |    6 MB |     5 MB |      0.82x |
| **100kb** |    5.2ms |    **4.5ms** |      **1.2x** |    3 MB |     4 MB |      1.56x |
| **10kb**  |    2.4ms |    **1.9ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |
| **1kb**   |    2.2ms |    **1.6ms** |      **1.4x** |    2 MB |     4 MB |      2.07x |

## Pattern: mixed

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |  594.1ms |  **367.1ms** |      **1.6x** |  217 MB |    20 MB |      0.09x |
| **10mb**  |   60.7ms |   **36.7ms** |      **1.7x** |   23 MB |     5 MB |      0.23x |
| **1mb**   |    7.6ms |    **5.1ms** |      **1.5x** |    4 MB |     4 MB |      1.23x |
| **100kb** |    2.6ms |    **2.0ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |
| **10kb**  |    2.2ms |    **1.7ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |
| **1kb**   |    2.3ms |    **1.7ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |

## Pattern: nested

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    1.85s |  **634.0ms** |      **2.9x** |  202 MB |   226 MB |      1.12x |
| **10mb**  |  185.3ms |   **64.1ms** |      **2.9x** |   22 MB |    26 MB |      1.19x |
| **1mb**   |   20.6ms |    **8.1ms** |      **2.5x** |    4 MB |     6 MB |      1.52x |
| **100kb** |    3.9ms |    **2.3ms** |      **1.7x** |    2 MB |     4 MB |      2.01x |
| **10kb**  |    2.3ms |    **1.8ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |
| **1kb**   |    2.2ms |    **1.7ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |

## Pattern: numbers

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.13s |    **1.75s** |      **1.8x** |  855 MB |   119 MB |      0.14x |
| **10mb**  |  313.4ms |  **172.0ms** |      **1.8x** |   87 MB |    16 MB |      0.18x |
| **1mb**   |   33.8ms |   **18.5ms** |      **1.8x** |   10 MB |     5 MB |      0.52x |
| **100kb** |    5.1ms |    **3.3ms** |      **1.5x** |    2 MB |     4 MB |      1.84x |
| **10kb**  |    2.4ms |    **1.9ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |
| **1kb**   |    2.2ms |    **1.7ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |

## Pattern: pathological

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    7.96s |    **4.98s** |      **1.6x** |    5 GB |   280 MB |      0.06x |
| **10mb**  |  803.0ms |  **483.4ms** |      **1.7x** |  472 MB |    32 MB |      0.07x |
| **1mb**   |   82.3ms |   **47.9ms** |      **1.7x** |   49 MB |     7 MB |      0.14x |
| **100kb** |    9.7ms |    **6.1ms** |      **1.6x** |    6 MB |     4 MB |      0.74x |
| **10kb**  |    2.7ms |    **2.1ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |
| **1kb**   |    2.3ms |    **1.7ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |

## Pattern: strings

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    1.74s |  **657.3ms** |      **2.7x** |  130 MB |   111 MB |      0.86x |
| **10mb**  |  176.5ms |   **66.6ms** |      **2.7x** |   14 MB |    15 MB |      1.05x |
| **1mb**   |   19.3ms |    **8.5ms** |      **2.3x** |    3 MB |     5 MB |      1.93x |
| **100kb** |    3.7ms |    **2.3ms** |      **1.6x** |    2 MB |     4 MB |      2.07x |
| **10kb**  |    2.3ms |    **1.7ms** |      **1.4x** |    2 MB |     4 MB |      2.07x |
| **1kb**   |    2.2ms |    **1.6ms** |      **1.4x** |    2 MB |     4 MB |      2.07x |

## Pattern: unicode

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    2.06s |    **1.33s** |      **1.5x** |  297 MB |   128 MB |      0.43x |
| **10mb**  |  208.6ms |  **131.9ms** |      **1.6x** |   31 MB |    16 MB |      0.53x |
| **1mb**   |   23.0ms |   **14.8ms** |      **1.6x** |    5 MB |     5 MB |      1.06x |
| **100kb** |    4.1ms |    **2.9ms** |      **1.4x** |    2 MB |     4 MB |      2.13x |
| **10kb**  |    2.3ms |    **1.7ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |
| **1kb**   |    2.3ms |    **1.7ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |

## Pattern: users

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    2.49s |    **1.52s** |      **1.6x** |  663 MB |    90 MB |      0.14x |
| **10mb**  |  249.0ms |  **149.0ms** |      **1.7x** |   68 MB |    12 MB |      0.18x |
| **1mb**   |   27.1ms |   **16.2ms** |      **1.7x** |    8 MB |     5 MB |      0.59x |
| **100kb** |    4.3ms |    **3.0ms** |      **1.4x** |    2 MB |     4 MB |      2.07x |
| **10kb**  |    2.4ms |    **1.8ms** |      **1.3x** |    2 MB |     4 MB |      2.07x |
| **1kb**   |    2.3ms |    **1.7ms** |      **1.4x** |    2 MB |     4 MB |      2.07x |

## Key Findings (ARM Neoverse-V1)

- **Speedup range**: 1.0x - 2.9x faster than jq across all patterns
- **Best performance**: Nested structures (2.9x on 100MB) and strings (2.7x on 100MB)
- **Memory efficiency**: 7-14% of jq's memory on large files (100MB)
- **Consistent speedup**: 1.5-1.8x across most patterns at 1MB+
- **NEON SIMD**: Effective use of ARM NEON for string scanning
- **SVE support**: Platform supports SVE but benchmarks use NEON paths

---

# Apple M1 Max (ARM, aarch64)

**CPU**: Apple M1 Max
**OS**: macOS
**jq version**: System jq
**succinctly**: Built with `--release --features cli`

## Pattern: arrays

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |   10.31s |    **7.86s** |      **1.3x** |    4 GB |   490 MB |      0.14x |
| **10mb**  |    1.04s |  **670.2ms** |      **1.6x** |  369 MB |    57 MB |      0.15x |
| **1mb**   |  106.9ms |   **68.3ms** |      **1.6x** |   39 MB |    14 MB |      0.35x |
| **100kb** |   16.0ms |   **11.3ms** |      **1.4x** |    6 MB |     8 MB |      1.30x |
| **10kb**  |    6.5ms |      6.6ms   |          1.0x |    3 MB |     8 MB |      2.67x |
| **1kb**   |    6.0ms |      6.5ms   |          0.9x |    2 MB |     7 MB |      3.00x |

## Pattern: comprehensive

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    6.47s |    **3.76s** |      **1.7x** |    1 GB |   120 MB |      0.09x |
| **10mb**  |  670.8ms |  **343.1ms** |      **2.0x** |  136 MB |    19 MB |      0.14x |
| **1mb**   |   70.9ms |   **38.9ms** |      **1.8x** |   16 MB |     9 MB |      0.54x |
| **100kb** |   12.0ms |    **8.2ms** |      **1.5x** |    4 MB |     8 MB |      1.85x |
| **10kb**  |    6.0ms |    **5.7ms** |      **1.0x** |    3 MB |     8 MB |      2.82x |
| **1kb**   |    5.9ms |      6.1ms   |          1.0x |    2 MB |     7 MB |      2.99x |

## Pattern: literals

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    4.89s |    **4.59s** |      **1.1x** |    1 GB |   149 MB |      0.13x |
| **10mb**  |  488.2ms |  **402.4ms** |      **1.2x** |  103 MB |    22 MB |      0.21x |
| **1mb**   |   54.1ms |   **42.5ms** |      **1.3x** |   10 MB |     9 MB |      0.87x |
| **100kb** |   10.6ms |    **8.8ms** |      **1.2x** |    3 MB |     8 MB |      2.27x |
| **10kb**  |    5.5ms |    **5.2ms** |      **1.0x** |    3 MB |     7 MB |      2.84x |
| **1kb**   |    6.0ms |      6.1ms   |          1.0x |    2 MB |     7 MB |      2.98x |

## Pattern: mixed

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |  933.5ms |  **465.2ms** |      **2.0x** |  250 MB |    25 MB |      0.10x |
| **10mb**  |   92.1ms |   **47.8ms** |      **1.9x** |   27 MB |     9 MB |      0.34x |
| **1mb**   |   12.8ms |    **8.7ms** |      **1.5x** |    5 MB |     8 MB |      1.55x |
| **100kb** |    5.6ms |    **5.1ms** |      **1.1x** |    3 MB |     8 MB |      2.74x |
| **10kb**  |    5.0ms |    **4.8ms** |      **1.0x** |    2 MB |     7 MB |      2.99x |
| **1kb**   |    6.0ms |    **4.9ms** |      **1.2x** |    2 MB |     7 MB |      2.98x |

## Pattern: nested

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.39s |  **589.0ms** |      **5.8x** |  205 MB |   242 MB |      1.18x |
| **10mb**  |  331.9ms |   **64.2ms** |      **5.2x** |   25 MB |    31 MB |      1.26x |
| **1mb**   |   39.2ms |   **12.4ms** |      **3.2x** |    5 MB |    10 MB |      2.11x |
| **100kb** |    8.0ms |    **5.4ms** |      **1.5x** |    3 MB |     8 MB |      2.80x |
| **10kb**  |    5.2ms |    **5.0ms** |      **1.0x** |    3 MB |     7 MB |      2.95x |
| **1kb**   |    4.9ms |    **4.8ms** |      **1.0x** |    2 MB |     7 MB |      3.00x |

## Pattern: numbers

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.65s |    **2.26s** |      **1.6x** |  983 MB |   135 MB |      0.14x |
| **10mb**  |  379.1ms |  **213.0ms** |      **1.8x** |   96 MB |    20 MB |      0.21x |
| **1mb**   |   42.7ms |   **25.9ms** |      **1.6x** |   12 MB |     9 MB |      0.71x |
| **100kb** |    9.5ms |    **8.2ms** |      **1.2x** |    4 MB |     8 MB |      2.14x |
| **10kb**  |    6.0ms |      6.2ms   |          1.0x |    3 MB |     7 MB |      2.86x |
| **1kb**   |    5.8ms |    **5.7ms** |      **1.0x** |    2 MB |     8 MB |      3.02x |

## Pattern: pathological

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |   13.37s |    **7.43s** |      **1.8x** |    5 GB |   297 MB |      0.06x |
| **10mb**  |    1.33s |  **634.3ms** |      **2.1x** |  526 MB |    38 MB |      0.07x |
| **1mb**   |  140.8ms |   **64.2ms** |      **2.2x** |   55 MB |    12 MB |      0.21x |
| **100kb** |   17.5ms |   **10.2ms** |      **1.7x** |    8 MB |     8 MB |      1.02x |
| **10kb**  |    6.1ms |    **5.1ms** |      **1.2x** |    3 MB |     8 MB |      2.53x |
| **1kb**   |    5.0ms |      5.2ms   |          1.0x |    2 MB |     7 MB |      2.99x |

## Pattern: strings

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.07s |  **745.5ms** |      **4.1x** |  159 MB |   126 MB |      0.79x |
| **10mb**  |  307.2ms |   **79.4ms** |      **3.9x** |   16 MB |    19 MB |      1.20x |
| **1mb**   |   36.6ms |   **13.2ms** |      **2.8x** |    4 MB |     9 MB |      2.43x |
| **100kb** |    7.7ms |    **5.4ms** |      **1.4x** |    3 MB |     8 MB |      3.01x |
| **10kb**  |    5.1ms |    **4.7ms** |      **1.1x** |    2 MB |     7 MB |      3.01x |
| **1kb**   |    4.9ms |    **4.6ms** |      **1.1x** |    2 MB |     7 MB |      3.01x |

## Pattern: unicode

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.18s |    **1.70s** |      **1.9x** |  424 MB |   144 MB |      0.34x |
| **10mb**  |  323.9ms |  **153.5ms** |      **2.1x** |   41 MB |    21 MB |      0.51x |
| **1mb**   |   37.6ms |   **19.2ms** |      **2.0x** |    7 MB |     9 MB |      1.32x |
| **100kb** |    8.9ms |    **7.0ms** |      **1.3x** |    3 MB |     8 MB |      2.54x |
| **10kb**  |    5.8ms |      6.0ms   |          1.0x |    2 MB |     8 MB |      3.01x |
| **1kb**   |    5.0ms |      5.1ms   |          1.0x |    2 MB |     7 MB |      3.00x |

## Pattern: users

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    4.02s |    **1.93s** |      **2.1x** |  681 MB |   102 MB |      0.15x |
| **10mb**  |  398.9ms |  **191.9ms** |      **2.1x** |   69 MB |    17 MB |      0.24x |
| **1mb**   |   52.7ms |   **24.9ms** |      **2.1x** |    9 MB |     8 MB |      0.90x |
| **100kb** |   10.2ms |    **8.1ms** |      **1.3x** |    3 MB |     8 MB |      2.37x |
| **10kb**  |    7.5ms |    **6.5ms** |      **1.2x** |    3 MB |     7 MB |      2.88x |
| **1kb**   |    5.2ms |      5.9ms   |          0.9x |    2 MB |     7 MB |      2.98x |

---

# Apple M4 Pro (ARM, aarch64)

**CPU**: Apple M4 Pro (12 cores)
**OS**: macOS 26.5.1
**jq version**: jq-1.7.1 (Apple build, 1.7.1-apple)
**succinctly**: Built with `--release --features cli,bench-runner` from `main` @ `4395a966a` plus the `users_mix` query set added in the same PR as this section (#5336); eval code is `main`'s
**Date**: 2026-10-27 (re-run against jq 1.7.1, #5336)
**Method**: median of 3 timed runs after 1 warmup for `.`; median of 5 for the query mix; output MD5 identical to jq on every row

## Harness comparison (`dev compare jq`)

<!-- compare:jq-macos-aarch64 -->
Generated by `succinctly dev compare jq --write-docs` from [jq-macos-aarch64.json](../compare/jq-macos-aarch64.json); do not edit. Full report: [jq-macos-aarch64.md](../compare/jq-macos-aarch64.md); every platform: [jq.md](../compare/jq.md).

**Run**: `./target/release/succinctly dev compare jq --quick --subject target/subject/release/succinctly --subject-features cli,simd,regex,serde` on Apple M4 Pro (macOS 26.5.1), succinctly `b53b0f9a5`, run `20261103_123922_b53b0f9a5`. jq-1.7.1 is built from source by the harness; the tools are interleaved within each of 5 repetitions, and a row is timed only when the outputs are identical.

**Timed binary**: a separate build passed with `--subject`, not the harness, built with `cli,simd,regex,serde`.

Geomean speedup over jq-1.7.1: **1.85x** across 85 timed rows; jq-1.7.1 is faster on 18 of them. Speedup is jq-1.7.1's time over succinctly jq's, so below 1.00x jq is faster; memory is succinctly jq's peak RSS over jq-1.7.1's. Times are medians, except where marked †: both times and the ratio are then each tool's fastest run, which run-to-run noise moves least on a long row.

| Query                        | Input             | jq-1.7.1 (ms) | succinctly jq (ms) |     Speedup | Memory ratio |
|------------------------------|-------------------|---------------|--------------------|-------------|--------------|
| `startup`                    | none              |           1.9 |                2.0 |       0.95x |        1.87x |
| `identity`                   | users-1mb         |          24.7 |                7.4 |  **3.34x**† |        0.72x |
| `identity`                   | numbers-1mb       |          22.8 |                6.6 |  **3.43x**† |        0.64x |
| `identity`                   | strings-1mb       |          22.0 |                3.0 |  **7.26x**† |        1.92x |
| `identity`                   | wide-1mb          |          36.3 |               10.3 |  **3.53x**† |        0.32x |
| `identity`                   | nested-1mb        |          23.4 |                3.4 |  **6.97x**† |        1.52x |
| `identity`                   | comprehensive-1mb |          42.3 |               10.5 |  **4.02x**† |        0.50x |
| `identity`                   | corpus-charts     |           1.9 |                2.0 |       0.94x |        1.91x |
| `identity`                   | corpus-npm        |           1.9 |                2.0 |       0.97x |        1.90x |
| `identity`                   | corpus-openapi    |           3.1 |                2.4 |   **1.28x** |        1.76x |
| `identity_compact`           | users-1mb         |          19.6 |                3.2 |  **6.07x**† |        0.59x |
| `identity_compact`           | numbers-1mb       |          17.9 |                4.4 |  **4.03x**† |        0.51x |
| `identity_compact`           | strings-1mb       |          21.5 |                2.4 |  **8.92x**† |        1.61x |
| `identity_compact`           | wide-1mb          |          33.2 |                5.1 |  **6.56x**† |        0.32x |
| `identity_compact`           | nested-1mb        |          23.8 |                2.4 | **10.12x**† |        1.26x |
| `identity_compact`           | comprehensive-1mb |          26.7 |               10.4 |  **2.58x**† |        0.43x |
| `identity_compact`           | corpus-charts     |           1.9 |                2.2 |       0.86x |        1.91x |
| `identity_compact`           | corpus-npm        |           1.9 |                2.2 |       0.89x |        1.93x |
| `identity_compact`           | corpus-openapi    |           3.1 |                2.4 |   **1.25x** |        1.73x |
| `users_field`                | users-1mb         |           7.8 |                3.3 |   **2.36x** |        0.67x |
| `users_length`               | users-1mb         |           7.8 |                3.4 |   **2.33x** |        0.72x |
| `users_names`                | users-1mb         |           8.5 |                5.5 |   **1.55x** |        0.67x |
| `users_map_select`           | users-1mb         |           9.0 |                6.5 |   **1.39x** |        0.84x |
| `users_sort_by`              | users-1mb         |          12.6 |                8.6 |   **1.46x** |        0.91x |
| `users_group_by`             | users-1mb         |          12.7 |                8.5 |   **1.50x** |        0.87x |
| `users_unique_by`            | users-1mb         |          12.9 |                8.5 |   **1.51x** |        0.88x |
| `users_reduce`               | users-1mb         |           8.7 |                8.0 |   **1.09x** |        0.76x |
| `users_to_entries`           | users-1mb         |           7.6 |                3.1 |   **2.44x** |        0.77x |
| `users_csv`                  | users-1mb         |          13.9 |               13.3 |   **1.05x** |        0.70x |
| `users_regex`                | users-1mb         |          13.6 |                6.4 |   **2.13x** |        0.87x |
| `users_ascii_downcase`       | users-1mb         |          21.8 |                6.4 |  **3.37x**† |        0.82x |
| `users_first_early_exit`     | users-1mb         |           7.7 |                3.1 |   **2.47x** |        0.69x |
| `users_limit_early_exit`     | users-1mb         |           7.7 |                3.0 |   **2.61x** |        0.71x |
| `users_assign_update`        | users-1mb         |           7.6 |                9.0 |       0.84x |        1.40x |
| `users_del`                  | users-1mb         |           7.7 |                9.2 |       0.85x |        1.35x |
| `users_paths`                | users-1mb         |          23.6 |               11.4 |  **2.06x**† |        1.10x |
| `users_tojson_roundtrip`     | users-1mb         |          17.9 |               15.0 |  **1.19x**† |        1.70x |
| `users_walk`                 | users-1mb         |          63.7 |               14.7 |  **4.34x**† |        1.54x |
| `users_slurp`                | users-1mb         |           7.6 |                3.2 |   **2.39x** |        0.76x |
| `users_stream`               | users-1mb         |          23.7 |               18.9 |  **1.25x**† |        2.40x |
| `numbers_add`                | numbers-1mb       |          21.2 |               10.8 |  **1.96x**† |        1.15x |
| `numbers_sort`               | numbers-1mb       |          36.1 |               23.8 |  **1.52x**† |        1.70x |
| `numbers_max`                | numbers-1mb       |          10.3 |               11.2 |       0.92x |        1.72x |
| `numbers_map_floor`          | numbers-1mb       |          24.4 |               18.4 |  **1.33x**† |        1.83x |
| `strings_length`             | strings-1mb       |           7.8 |                4.2 |   **1.84x** |        2.10x |
| `strings_upcase`             | strings-1mb       |         252.3 |                7.1 | **35.51x**† |        1.74x |
| `strings_split`              | strings-1mb       |          12.1 |                8.1 |   **1.50x** |        2.13x |
| `strings_test`               | strings-1mb       |          12.0 |                3.9 |   **3.07x** |        2.02x |
| `wide_keys`                  | wide-1mb          |          27.4 |                6.3 |  **4.34x**† |        0.35x |
| `wide_to_entries`            | wide-1mb          |          35.7 |               28.1 |  **1.27x**† |        0.94x |
| `wide_map_values`            | wide-1mb          |          96.0 |               36.6 |  **2.62x**† |        1.36x |
| `wide_add_values`            | wide-1mb          |          26.3 |               19.9 |  **1.32x**† |        1.43x |
| `wide_with_entries`          | wide-1mb          |         210.8 |               79.8 |  **2.64x**† |        0.99x |
| `nested_paths`               | nested-1mb        |           6.4 |                3.2 |   **1.99x** |        1.41x |
| `nested_recurse`             | nested-1mb        |           6.2 |                3.9 |   **1.58x** |        1.43x |
| `comprehensive_paths`        | comprehensive-1mb |          52.0 |               22.4 |  **2.32x**† |        1.19x |
| `comprehensive_tostream`     | comprehensive-1mb |         112.4 |               34.7 |  **3.24x**† |        1.59x |
| `comprehensive_numbers`      | comprehensive-1mb |          42.7 |               21.1 |  **2.03x**† |        1.71x |
| `ndjson_identity`            | ndjson-1mb        |          31.0 |                6.6 |  **4.66x**† |        2.29x |
| `ndjson_field`               | ndjson-1mb        |          13.3 |                8.7 |   **1.52x** |        2.34x |
| `ndjson_select`              | ndjson-1mb        |          14.5 |               11.2 |  **1.29x**† |        2.58x |
| `ndjson_tsv`                 | ndjson-1mb        |          21.3 |               18.3 |  **1.16x**† |        2.47x |
| `ndjson_slurp`               | ndjson-1mb        |          14.9 |                8.6 |   **1.73x** |        0.71x |
| `ndjson_inputs_reduce`       | ndjson-1mb        |          13.4 |               17.3 |      0.77x† |        3.09x |
| `ndjson_inputs_first`        | ndjson-1mb        |           1.9 |                4.1 |       0.46x |        2.78x |
| `ndjson_raw_lines`           | ndjson-1mb        |           8.4 |                4.2 |   **2.01x** |        3.25x |
| `ndjson_raw_fromjson`        | ndjson-1mb        |          15.9 |               16.8 |      0.94x† |        3.43x |
| `ndjson_raw_slurp`           | ndjson-1mb        |           6.1 |                2.3 |   **2.64x** |        1.43x |
| `ndjson_three_files`         | ndjson-1mb        |          35.6 |               18.8 |  **1.89x**† |        3.07x |
| `users_three_files`          | users-1mb         |          17.7 |                5.2 |  **3.40x**† |        0.82x |
| `seq_identity`               | jsonseq-1mb       |          31.1 |               12.2 |  **2.54x**† |        2.46x |
| `seq_field`                  | jsonseq-1mb       |          13.9 |               13.7 |   **1.02x** |        2.47x |
| `seq_slurp`                  | jsonseq-1mb       |          14.9 |               14.6 |   **1.02x** |        0.73x |
| `users_seq_output`           | users-1mb         |           3.3 |                3.6 |       0.92x |        2.10x |
| `users_slurpfile`            | users-1mb         |           7.6 |                9.2 |       0.82x |        1.40x |
| `ndjson_slurpfile`           | ndjson-1mb        |          11.1 |               13.9 |       0.80x |        1.29x |
| `users_rawfile`              | users-1mb         |           3.4 |                2.1 |   **1.62x** |        1.39x |
| `args_many`                  | none              |           1.8 |                2.0 |       0.90x |        2.15x |
| `corpus_npm_deps`            | corpus-npm        |           1.8 |                2.0 |       0.90x |        2.36x |
| `corpus_npm_scripts`         | corpus-npm        |           1.9 |                2.1 |       0.91x |        2.28x |
| `corpus_charts_titles`       | corpus-charts     |           1.8 |                2.1 |       0.85x |        2.04x |
| `corpus_openapi_definitions` | corpus-openapi    |           2.4 |                2.1 |   **1.14x** |        1.85x |
| `corpus_openapi_strings`     | corpus-openapi    |           2.6 |                2.5 |   **1.03x** |        2.03x |
| `corpus_openapi_refs`        | corpus-openapi    |           2.8 |                2.7 |   **1.04x** |        2.32x |
| `corpus_openapi_paths`       | corpus-openapi    |           2.8 |                2.6 |   **1.06x** |        1.91x |
<!-- /compare:jq-macos-aarch64 -->

The sections below are `bench run jq_bench` and `dev bench jq` results, measured by the method this section's header describes: all jq runs, then all succinctly runs, against the `jq` on `PATH`. That is not the interleaved method of [benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method), so where a figure below disagrees with the block above, the block is the measurement.

## Query mix (users pattern)

The `users_mix` query set (`succinctly dev bench jq --patterns users --queries users_mix`) over the `users` pattern (`{"users":[{id,name,email,age,active,score},...]}`), one process per query: speedup is jq's wall time over succinctly's, so **below 1.0x succinctly is slower**. Memory ratio is succinctly's peak RSS over jq's at 100mb.

| Query                                      | jq (100mb) | succinctly (100mb) | 100mb    | 10mb     | 1mb      | 100kb    | 10kb | Mem Ratio (100mb) |
|--------------------------------------------|------------|--------------------|----------|----------|----------|----------|------|-------------------|
| `.`                                        |      2.74s |            536.8ms | **5.1x** | **5.1x** | **3.1x** | **1.2x** | 0.8x |             0.30x |
| `.users[0].name`                           |    678.7ms |             97.5ms | **7.0x** | **5.5x** | **1.9x** |     0.9x | 0.7x |             0.15x |
| `.users \| length`                         |    664.8ms |             97.1ms | **6.8x** | **5.3x** | **2.0x** |     0.9x | 0.7x |             0.15x |
| `[.users[] \| select(.age > 50) \| .name]` |    977.1ms |            537.4ms | **1.8x** | **1.7x** | **1.3x** |     0.8x | 0.7x |             0.31x |
| `.users \| map(.score)`                    |    871.1ms |            320.1ms | **2.7x** | **2.4x** | **1.6x** |     0.8x | 0.7x |             0.24x |
| `.users \| sort_by(.age)`                  |      3.75s |              1.25s | **3.0x** | **3.1x** | **2.3x** | **1.1x** | 0.8x |             0.58x |
| `.users[] \| "\(.name) <\(.email)>"`       |      1.50s |            779.3ms | **1.9x** | **1.9x** | **1.4x** |     0.9x | 0.7x |             0.15x |
| `.users \| group_by(.age)`                 |      3.98s |              1.40s | **2.8x** | **3.0x** | **2.1x** | **1.1x** | 0.8x |             0.56x |
| `.users \| INDEX(.id)`                     |      3.22s |              2.19s | **1.5x** | **1.7x** | **1.4x** |     0.9x | 0.7x |             1.87x |

## Pattern: arrays

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    6.43s |    **1.23s** |      **5.2x** |    4 GB |   386 MB |      0.11x |
| **10mb**  |  672.6ms |  **127.4ms** |      **5.3x** |  367 MB |    50 MB |      0.14x |
| **1mb**   |   67.5ms |   **16.3ms** |      **4.1x** |   38 MB |    15 MB |      0.39x |
| **100kb** |   11.2ms |    **4.9ms** |      **2.3x** |    6 MB |     9 MB |      1.61x |
| **10kb**  |    3.3ms |      3.8ms   |          0.9x |    3 MB |     9 MB |      3.46x |
| **1kb**   |    2.7ms |      3.6ms   |          0.7x |    2 MB |     9 MB |      3.92x |

## Pattern: comprehensive

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    4.30s |  **773.3ms** |      **5.6x** |    1 GB |   296 MB |      0.23x |
| **10mb**  |  464.8ms |   **79.4ms** |      **5.9x** |  132 MB |    40 MB |      0.30x |
| **1mb**   |   50.4ms |   **11.9ms** |      **4.2x** |   16 MB |    12 MB |      0.78x |
| **100kb** |    7.3ms |    **4.7ms** |      **1.6x** |    4 MB |    10 MB |      2.51x |
| **10kb**  |    3.1ms |      3.9ms   |          0.8x |    2 MB |     9 MB |      3.76x |
| **1kb**   |    2.7ms |      3.6ms   |          0.7x |    2 MB |     9 MB |      4.01x |

## Pattern: literals

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    3.22s |  **870.2ms** |      **3.7x** |    1 GB |   332 MB |      0.30x |
| **10mb**  |  326.5ms |   **91.1ms** |      **3.6x** |  103 MB |    44 MB |      0.43x |
| **1mb**   |   34.6ms |   **12.3ms** |      **2.8x** |   10 MB |    12 MB |      1.23x |
| **100kb** |    6.4ms |    **4.5ms** |      **1.4x** |    3 MB |    10 MB |      3.01x |
| **10kb**  |    3.0ms |      3.6ms   |          0.8x |    2 MB |     9 MB |      3.82x |
| **1kb**   |    2.6ms |      3.5ms   |          0.7x |    2 MB |     9 MB |      3.95x |

## Pattern: mixed

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |  711.7ms |  **123.5ms** |      **5.8x** |  248 MB |    52 MB |      0.21x |
| **10mb**  |   64.9ms |   **16.0ms** |      **4.1x** |   27 MB |    15 MB |      0.57x |
| **1mb**   |    8.8ms |    **4.9ms** |      **1.8x** |    5 MB |    10 MB |      2.05x |
| **100kb** |    3.2ms |      3.8ms   |          0.8x |    3 MB |     9 MB |      3.61x |
| **10kb**  |    2.6ms |      3.7ms   |          0.7x |    2 MB |     9 MB |      3.97x |
| **1kb**   |    2.6ms |      3.5ms   |          0.7x |    2 MB |     9 MB |      3.96x |

## Pattern: nested

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    2.48s |  **131.5ms** |     **18.9x** |  204 MB |   228 MB |      1.12x |
| **10mb**  |  268.0ms |   **16.9ms** |     **15.9x** |   24 MB |    31 MB |      1.27x |
| **1mb**   |   29.6ms |    **5.0ms** |      **5.9x** |    4 MB |    11 MB |      2.59x |
| **100kb** |    4.9ms |    **3.9ms** |      **1.2x** |    3 MB |     9 MB |      3.60x |
| **10kb**  |    2.9ms |      3.7ms   |          0.8x |    2 MB |     9 MB |      3.91x |
| **1kb**   |    2.5ms |      3.6ms   |          0.7x |    2 MB |     9 MB |      3.94x |

## Pattern: numbers

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    2.33s |  **465.3ms** |      **5.0x** |  983 MB |   250 MB |      0.25x |
| **10mb**  |  245.1ms |   **50.7ms** |      **4.8x** |   96 MB |    35 MB |      0.37x |
| **1mb**   |   26.7ms |    **8.4ms** |      **3.2x** |   11 MB |    12 MB |      1.03x |
| **100kb** |    5.0ms |    **4.3ms** |      **1.2x** |    3 MB |     9 MB |      2.81x |
| **10kb**  |    2.8ms |      3.7ms   |          0.8x |    2 MB |     9 MB |      3.83x |
| **1kb**   |    2.6ms |      3.5ms   |          0.7x |    2 MB |     9 MB |      3.94x |

## Pattern: pathological

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    8.74s |    **1.80s** |      **4.8x** |    5 GB |   513 MB |      0.10x |
| **10mb**  |  938.5ms |  **185.7ms** |      **5.1x** |  525 MB |    63 MB |      0.12x |
| **1mb**   |   94.1ms |   **22.8ms** |      **4.1x** |   55 MB |    16 MB |      0.30x |
| **100kb** |   12.2ms |    **5.5ms** |      **2.2x** |    7 MB |    10 MB |      1.30x |
| **10kb**  |    3.5ms |      3.9ms   |          0.9x |    3 MB |     9 MB |      3.32x |
| **1kb**   |    2.7ms |      3.6ms   |          0.7x |    2 MB |     9 MB |      3.91x |

## Pattern: strings

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    2.34s |  **160.8ms** |     **14.5x** |  159 MB |   206 MB |      1.30x |
| **10mb**  |  244.4ms |   **20.2ms** |     **12.1x** |   16 MB |    30 MB |      1.89x |
| **1mb**   |   23.8ms |    **5.2ms** |      **4.6x** |    3 MB |    11 MB |      3.32x |
| **100kb** |    4.8ms |    **3.8ms** |      **1.3x** |    2 MB |     9 MB |      3.97x |
| **10kb**  |    2.7ms |      3.5ms   |          0.8x |    2 MB |     9 MB |      3.96x |
| **1kb**   |    2.5ms |      3.6ms   |          0.7x |    2 MB |     9 MB |      3.94x |

## Pattern: unicode

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    2.30s |  **354.4ms** |      **6.5x** |  422 MB |   252 MB |      0.60x |
| **10mb**  |  230.0ms |   **39.2ms** |      **5.9x** |   41 MB |    35 MB |      0.86x |
| **1mb**   |   25.0ms |    **7.1ms** |      **3.5x** |    7 MB |    12 MB |      1.77x |
| **100kb** |    4.7ms |    **4.0ms** |      **1.2x** |    3 MB |     9 MB |      3.36x |
| **10kb**  |    2.8ms |      3.7ms   |          0.8x |    2 MB |     9 MB |      3.94x |
| **1kb**   |    2.5ms |      3.5ms   |          0.7x |    2 MB |     9 MB |      3.94x |

## Pattern: users

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    2.79s |  **538.5ms** |      **5.2x** |  680 MB |   201 MB |      0.30x |
| **10mb**  |  303.6ms |   **57.3ms** |      **5.3x** |   70 MB |    31 MB |      0.44x |
| **1mb**   |   31.3ms |    **9.3ms** |      **3.4x** |    9 MB |    11 MB |      1.21x |
| **100kb** |    5.2ms |    **4.3ms** |      **1.2x** |    3 MB |     9 MB |      3.20x |
| **10kb**  |    2.9ms |      3.7ms   |          0.8x |    2 MB |     9 MB |      3.89x |
| **1kb**   |    2.6ms |      3.7ms   |          0.7x |    2 MB |     9 MB |      3.95x |

## Pattern: wide

| Size      | jq       | succinctly   | Speedup       | jq Mem  | succ Mem | Mem Ratio  |
|-----------|----------|--------------|---------------|---------|----------|------------|
| **100mb** |    4.25s |  **776.3ms** |      **5.5x** |    1 GB |   317 MB |      0.21x |
| **10mb**  |  461.4ms |   **81.3ms** |      **5.7x** |  180 MB |    44 MB |      0.24x |
| **1mb**   |   44.5ms |   **11.8ms** |      **3.8x** |   23 MB |    12 MB |      0.51x |
| **100kb** |    6.8ms |    **4.6ms** |      **1.5x** |    5 MB |    10 MB |      1.95x |
| **10kb**  |    3.0ms |      3.8ms   |          0.8x |    3 MB |     9 MB |      3.63x |
| **1kb**   |    2.6ms |      3.7ms   |          0.7x |    2 MB |     9 MB |      3.92x |

---

## Pattern Descriptions

| Pattern           | Description                                     |
|-------------------|-------------------------------------------------|
| **arrays**        | Arrays of arrays (tests iteration performance)  |
| **comprehensive** | Mixed content with all JSON features            |
| **literals**      | Mix of null, true, false literals               |
| **mixed**         | Heterogeneous nested structures                 |
| **nested**        | Deeply nested objects (tests tree navigation)   |
| **numbers**       | Number-heavy documents with various formats     |
| **pathological**  | Worst-case patterns (deep nesting, escapes)     |
| **strings**       | String-heavy with escape sequences              |
| **unicode**       | UTF-8 multibyte sequences                       |
| **users**         | Realistic user record objects                   |

## Key Findings

### Speed

**AMD Zen 4 (x86_64, jq 1.7.1, #5336)**:
- **0.95-3.9x** across all patterns and sizes for `.`: 2.1-3.9x at 10-100 MB, 1.7-3.4x at 1 MB, 1.1-2.1x at 100 KB, 1.0-1.2x at 10 KB and 0.95-1.0x at 1 KB, where process start-up is the whole runtime
- **Best on numbers, arrays and nested data**: 3.7-3.9x at 10 MB; strings 3.6x
- **Query mix** (`users`, 100 MB): navigation and `length` 6.3-6.4x, `map` 3.2x, `select` 2.0x, `sort_by` 2.4x, `group_by` 2.1x, string interpolation 1.7x
- **`INDEX(.id)`**: the table's 0.64-0.76x from 100 KB to 100 MB (peak RSS 2.3x jq's at 100 MB) was measured before the #5306 `INDEX` change. With it, on the same 100 MB `users` file, the 7950X takes about 2.2 s against jq's 2.4 s (1.09x) and 0.97 GB against jq's 0.76 GB

**Apple M1 Max (ARM)**:
- **1.0-5.8x faster** across all patterns and sizes
- **Best performance on nested data**: 5.8x speedup on deeply nested structures (100MB)
- **String-heavy data**: 4.1x speedup due to efficient escape handling
- **Generally faster than x86_64**: ARM NEON SIMD provides better acceleration for JSON parsing

**Apple M4 Pro (ARM, jq 1.7.1-apple, #5336)**:
- **0.7-18.9x** across all patterns and sizes for `.`: 3.6-18.9x at 10-100 MB, 1.8-5.9x at 1 MB, 0.84-2.3x at 100 KB and 0.7-0.9x at 10 KB and below, where process start-up is the whole runtime
- **Best on nested and string-heavy data**: nested 18.9x and strings 14.5x at 100 MB; comprehensive 5.6x
- **Query mix** (`users`, 100 MB): navigation and `length` 6.9-7.0x, `sort_by` 3.0x, `group_by` 2.8x, `map` 2.7x, string interpolation 1.9x, `select` 1.8x, `INDEX(.id)` 1.5x (peak RSS 1.9x jq's; the #5306 `INDEX` change cuts its wall time by 34% and its peak RSS to jq's 0.85 GB)
- **Slower than jq on small inputs**: at 100 KB navigation, `length`, `select`, `map`, string interpolation and `INDEX` are 0.82-0.87x (`.`, `sort_by` and `group_by` are 1.1-1.2x), and every query is 0.68-0.78x at 10 KB and 1 KB

### Memory

**Both platforms show similar memory characteristics**:
- **Dramatically lower memory usage** on large files (1MB+)
- **3-17x less memory** on larger files: pathological (0.06x), comprehensive (0.09x), mixed (0.10x)
- **Slightly higher on small files**: 1-2x overhead due to minimum index size (~4-7 MB baseline)
- **Streaming output**: Uses lazy cursor evaluation - only materializes values when needed

### Why succinctly is faster

1. **Succinct indexing**: JSON structure is pre-indexed using balanced parentheses, enabling O(1) navigation
2. **SIMD acceleration**: Uses NEON on ARM and SSE/AVX on x86_64 for character classification
3. **Table-driven parser**: PFSM (Parallel Finite State Machine) with lookup tables
4. **Lazy evaluation**: Only materializes values that are actually accessed
5. **Streaming output**: For identity queries, outputs directly from source without building intermediate structures

## Reproducing Benchmarks

```bash
# Build release binary with benchmark runner
cargo build --release --features bench-runner

# Generate benchmark data
./target/release/succinctly json generate-suite

# Run comparison
./target/release/succinctly bench run jq_bench
```
