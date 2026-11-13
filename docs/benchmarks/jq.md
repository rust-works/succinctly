# jq vs succinctly Comparison Benchmarks

[Home](../../) > [Docs](../) > [Benchmarks](./) > jq

Comprehensive benchmarks comparing `succinctly jq .` vs `jq .` for JSON formatting/printing.

**See also**: [JsonIndex wiki page](../parsing/json-index.md) for architecture overview and academic references.

Results are provided for ARM (Apple M4 Pro, Apple M1 Max, AWS Graviton) and x86_64 (AMD Zen 4) platforms. The AMD Ryzen 9 7950X and Apple M4 Pro sections were re-run against jq 1.7.1 in #5336 and include a query mix beyond `.`; the other platforms' sections were measured against the jq version named in each, so do not compare speedups across sections whose jq version differs.

The AMD Ryzen 9 7950X, Apple M4 Pro and ARM Neoverse-V2 sections each carry a generated *Harness comparison* block: the pinned jq 1.7.1, interleaved repetitions and output-gated rows of `succinctly dev compare jq`, quoted from the committed run in [docs/compare/](../compare/jq.md) and checked against it by a test. Where a hand-written figure below disagrees with one of those blocks, the block is the measurement.

Every other table and figure in this page is a `bench run jq_bench` or `dev bench jq` result measured before #5409 made those runners interleave, so none of it is generated from or checked against a committed run. The M1 Max and Neoverse-V1 sections have no harness run to replace them, and the per-pattern tables cover shapes the harness does not generate (`arrays`, `literals`, `mixed`, `pathological`, `unicode`), so all of it is kept as history. For a current figure, quote a *Harness comparison* block or the table in [CLAUDE.md](../../CLAUDE.md#performance-summary).

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
- **Peak memory**: Maximum resident set size (RSS). On a file of many top-level values (NDJSON) the plain rows (`ndjson_identity`, `ndjson_field`, `ndjson_select`, `ndjson_tsv`) are flat from 10 MiB to 100 MiB since [#6920](https://github.com/rust-works/succinctly/issues/6920) (about 10 MB on x86_64 and 13 MB on aarch64 at 100 MiB, from 1.2 times the file), where the tables below were measured before it; `ndjson_inputs_reduce` and `ndjson_raw_lines` are flat too since [#7023](https://github.com/rust-works/succinctly/issues/7023) (11 to 15 MB at 100 MiB on both machines, from about 1.0 times the file), and `ndjson_inputs_first` is flat, where jq's stays at a few MB; `seq_identity` is flat since [#7082](https://github.com/rust-works/succinctly/issues/7082) too when the file is RFC 7464's own framing (7 to 10 MiB at 100 MiB, from 1.2 times the file), and over a pipe of 4 MiB or more since [#7164](https://github.com/rust-works/succinctly/issues/7164), up to its first record that is not that framing. The plain rows over a regular file on stdin, over a pipe of 4 MiB or more, or over several regular files read as one stream are flat too since [#7024](https://github.com/rust-works/succinctly/issues/7024). `-R` over standard input (a pipe of 4 MiB or more, or a redirected file) and over several regular files with 4 MiB or more between them is flat since #7024 too. `input`/`inputs` over up to 32 regular files with 4 MiB or more between them is flat too since #7024 (13 to 17 MB at 100 MiB on x86_64, 8 to 19 MB on aarch64) up to a fault or a document a file ends inside, after which the rest is held; over more than 32 files they hold the whole input, so jq is the tool for those at a size too large to hold: [#6913](https://github.com/rust-works/succinctly/issues/6913), measured in [jq limitations](../compliance/jq/limitations.md#peak-memory-on-a-stream-of-many-values-grows-with-the-input-6913)
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

Both runners interleave the two tools within each repetition, alternating which runs first, and take the median of the runs (#5409). Before #5409 they timed all of jq's runs and then all of succinctly's, which is how every hand-written section below was measured.
They compare against the `jq` on `PATH`; `dev bench jq --jq target/compare/jq-1.7.1/bin/jq` uses the pinned build instead (`bench run jq_bench` always takes `PATH`'s), and the generated markdown names the jq and the method under its title.
The *Harness comparison* blocks come from `succinctly dev compare jq` instead ([comparison/jq/README.md](../../comparison/jq/README.md)). It builds jq 1.7.1 and 1.8.2 from source, interleaves the tools within each repetition, and times a row only when the outputs are identical. `--write-docs` regenerates a platform's block with its report, and `--render-docs` re-renders every block from the committed runs without measuring anything:

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

**Run**: `./target/release/succinctly dev compare jq --full --huge` on AMD Ryzen 9 7950X 16-Core Processor (Ubuntu 22.04.5 LTS), succinctly `f056da9c6`, run `20261112_092959_f056da9c6`. jq-1.7.1 is built from source by the harness; the tools are interleaved within each of 21 repetitions, and a row is timed only when the outputs are identical.

**Timed binary**: the harness itself, built with `cli,simd,regex,serde`.

Geomean speedup over jq-1.7.1: **2.20x** across 221 timed rows; jq-1.7.1 is faster on 26 of them. Speedup is jq-1.7.1's time over succinctly jq's, so below 1.00x jq is faster; memory is succinctly jq's peak RSS over jq-1.7.1's. Times are medians, except where marked †: both times and the ratio are then each tool's fastest run, which run-to-run noise moves least on a long row.

| Query                        | Input                   | jq-1.7.1 (ms) | succinctly jq (ms) |     Speedup | Memory ratio |
|------------------------------|-------------------------|---------------|--------------------|-------------|--------------|
| `startup`                    | none                    |           1.2 |                1.2 |       0.99x |        3.54x |
| `identity`                   | users-1mb               |          16.5 |                7.7 |  **2.15x**† |        1.34x |
| `identity`                   | users-10mb              |         157.6 |               65.5 |  **2.41x**† |        0.42x |
| `identity`                   | users-100mb             |        1626.6 |              648.8 |   **2.51x** |        0.30x |
| `identity`                   | numbers-1mb             |          22.0 |                6.6 |  **3.34x**† |        1.17x |
| `identity`                   | numbers-10mb            |         210.2 |               52.4 |  **4.02x**† |        0.39x |
| `identity`                   | numbers-100mb           |        2185.0 |              508.9 |   **4.29x** |        0.29x |
| `identity`                   | strings-1mb             |          12.1 |                4.0 |   **3.04x** |        3.57x |
| `identity`                   | strings-10mb            |         111.9 |               25.8 |  **4.33x**† |        2.00x |
| `identity`                   | strings-100mb           |        1153.4 |              242.0 |   **4.77x** |        1.58x |
| `identity`                   | wide-1mb                |          30.3 |               12.3 |  **2.47x**† |        0.74x |
| `identity`                   | wide-10mb               |         310.2 |              105.9 |  **2.93x**† |        0.30x |
| `identity`                   | wide-100mb              |        3672.8 |              977.9 |   **3.76x** |        0.19x |
| `identity`                   | nested-1mb              |          13.5 |                4.4 |   **3.04x** |        2.53x |
| `identity`                   | nested-10mb             |         123.7 |               33.1 |  **3.74x**† |        1.41x |
| `identity`                   | nested-100mb            |        1248.7 |              324.7 |   **3.85x** |        1.13x |
| `identity`                   | comprehensive-1mb       |          30.7 |               10.6 |  **2.90x**† |        0.77x |
| `identity`                   | comprehensive-10mb      |         293.4 |               87.0 |  **3.37x**† |        0.26x |
| `identity`                   | comprehensive-100mb     |        3034.6 |              884.7 |   **3.43x** |        0.20x |
| `identity`                   | corpus-charts           |           1.2 |                1.2 |       1.00x |        3.65x |
| `identity`                   | corpus-npm              |           1.2 |                1.2 |       0.99x |        3.56x |
| `identity`                   | corpus-graph            |           1.5 |                1.4 |   **1.09x** |        3.47x |
| `identity`                   | corpus-openapi          |           1.8 |                1.6 |   **1.17x** |        3.63x |
| `identity`                   | corpus-geojson          |           3.9 |                2.3 |   **1.72x** |        3.05x |
| `identity`                   | corpus-tabular          |           1.8 |                1.5 |   **1.20x** |        3.52x |
| `identity`                   | corpus-pretty           |          11.2 |                4.8 |   **2.34x** |        1.84x |
| `identity_compact`           | users-1mb               |          14.7 |                2.8 |  **5.19x**† |        1.19x |
| `identity_compact`           | users-10mb              |         139.2 |               17.5 |  **7.96x**† |        0.26x |
| `identity_compact`           | users-100mb             |        1435.4 |              165.5 |   **8.67x** |        0.14x |
| `identity_compact`           | numbers-1mb             |          19.9 |                3.6 |  **5.61x**† |        1.00x |
| `identity_compact`           | numbers-10mb            |         194.9 |               23.5 |  **8.29x**† |        0.24x |
| `identity_compact`           | numbers-100mb           |        2007.5 |              224.2 |   **8.95x** |        0.14x |
| `identity_compact`           | strings-1mb             |          11.8 |                2.9 |   **4.07x** |        3.21x |
| `identity_compact`           | strings-10mb            |         108.4 |               16.9 |  **6.40x**† |        1.38x |
| `identity_compact`           | strings-100mb           |        1115.9 |              155.2 |   **7.19x** |        0.88x |
| `identity_compact`           | wide-1mb                |          29.1 |                5.8 |  **5.03x**† |        0.70x |
| `identity_compact`           | wide-10mb               |         300.6 |               41.9 |  **7.18x**† |        0.25x |
| `identity_compact`           | wide-100mb              |        3578.6 |              976.0 |   **3.67x** |        0.18x |
| `identity_compact`           | nested-1mb              |          13.3 |                3.1 |   **4.30x** |        2.28x |
| `identity_compact`           | nested-10mb             |         121.9 |               18.0 |  **6.77x**† |        0.95x |
| `identity_compact`           | nested-100mb            |        1228.6 |              167.5 |   **7.34x** |        0.63x |
| `identity_compact`           | comprehensive-1mb       |          24.6 |               10.1 |  **2.43x**† |        0.71x |
| `identity_compact`           | comprehensive-10mb      |         242.7 |               83.1 |  **2.92x**† |        0.20x |
| `identity_compact`           | comprehensive-100mb     |        2505.1 |              841.0 |   **2.98x** |        0.14x |
| `identity_compact`           | corpus-charts           |           1.2 |                1.2 |       0.98x |        3.50x |
| `identity_compact`           | corpus-npm              |           1.2 |                1.2 |   **1.01x** |        3.61x |
| `identity_compact`           | corpus-graph            |           1.4 |                1.2 |   **1.15x** |        3.39x |
| `identity_compact`           | corpus-openapi          |           1.7 |                1.5 |   **1.13x** |        3.66x |
| `identity_compact`           | corpus-geojson          |           3.0 |                2.0 |   **1.51x** |        2.86x |
| `identity_compact`           | corpus-tabular          |           1.7 |                1.4 |   **1.20x** |        3.62x |
| `identity_compact`           | corpus-pretty           |           8.9 |                4.4 |   **2.03x** |        1.77x |
| `users_field`                | users-1mb               |           9.0 |                3.0 |   **3.02x** |        1.46x |
| `users_field`                | users-10mb              |          88.8 |               15.7 |  **5.65x**† |        0.29x |
| `users_field`                | users-100mb             |         930.0 |              139.7 |   **6.66x** |        0.16x |
| `users_length`               | users-1mb               |           9.3 |                3.2 |   **2.90x** |        1.55x |
| `users_length`               | users-10mb              |          92.5 |               17.2 |  **5.39x**† |        0.34x |
| `users_length`               | users-100mb             |         984.6 |              159.2 |   **6.19x** |        0.19x |
| `users_names`                | users-1mb               |           9.8 |                5.1 |   **1.92x** |        1.45x |
| `users_names`                | users-10mb              |          97.4 |               37.3 |  **2.61x**† |        0.31x |
| `users_names`                | users-100mb             |        1018.4 |              362.2 |   **2.81x** |        0.16x |
| `users_map_select`           | users-1mb               |          11.0 |                6.6 |   **1.66x** |        1.71x |
| `users_map_select`           | users-10mb              |         130.6 |               50.5 |  **2.59x**† |        0.45x |
| `users_map_select`           | users-100mb             |        1431.4 |              499.3 |   **2.87x** |        0.28x |
| `users_sort_by`              | users-1mb               |          15.1 |                9.7 |  **1.55x**† |        1.59x |
| `users_sort_by`              | users-10mb              |         179.1 |               88.3 |  **2.03x**† |        0.57x |
| `users_group_by`             | users-1mb               |          15.5 |                9.4 |  **1.64x**† |        1.62x |
| `users_group_by`             | users-10mb              |         193.1 |               86.2 |  **2.24x**† |        0.56x |
| `users_unique_by`            | users-1mb               |          15.6 |                9.3 |  **1.69x**† |        1.62x |
| `users_unique_by`            | users-10mb              |         202.1 |               85.8 |  **2.35x**† |        0.58x |
| `users_reduce`               | users-1mb               |          10.3 |                8.0 |   **1.28x** |        1.71x |
| `users_reduce`               | users-10mb              |         104.8 |               66.2 |  **1.58x**† |        0.34x |
| `users_reduce`               | users-100mb             |        1089.6 |              647.5 |   **1.68x** |        0.16x |
| `users_to_entries`           | users-1mb               |           8.9 |                3.0 |   **2.93x** |        1.77x |
| `users_to_entries`           | users-10mb              |          87.9 |               15.5 |  **5.66x**† |        0.35x |
| `users_to_entries`           | users-100mb             |         929.2 |              142.3 |   **6.53x** |        0.16x |
| `users_csv`                  | users-1mb               |          15.4 |               11.6 |  **1.32x**† |        1.52x |
| `users_csv`                  | users-10mb              |         148.9 |              103.3 |  **1.44x**† |        0.31x |
| `users_csv`                  | users-100mb             |        1533.9 |             1032.0 |   **1.49x** |        0.15x |
| `users_regex`                | users-1mb               |          15.2 |                6.4 |  **2.36x**† |        1.89x |
| `users_regex`                | users-10mb              |         156.8 |               50.1 |  **3.13x**† |        0.47x |
| `users_ascii_downcase`       | users-1mb               |          26.1 |                7.0 |  **3.73x**† |        1.68x |
| `users_ascii_downcase`       | users-10mb              |         286.5 |               58.9 |  **4.87x**† |        0.45x |
| `users_ascii_downcase`       | users-100mb             |        3120.9 |              580.9 |   **5.37x** |        0.30x |
| `users_first_early_exit`     | users-1mb               |           8.9 |                2.7 |   **3.24x** |        1.51x |
| `users_first_early_exit`     | users-10mb              |          76.0 |               14.0 |  **5.42x**† |        0.33x |
| `users_first_early_exit`     | users-100mb             |         779.2 |              125.4 |   **6.21x** |        0.16x |
| `users_limit_early_exit`     | users-1mb               |           9.0 |                2.7 |   **3.29x** |        1.55x |
| `users_limit_early_exit`     | users-10mb              |          88.8 |               14.0 |  **6.36x**† |        0.33x |
| `users_limit_early_exit`     | users-100mb             |         928.1 |              125.1 |   **7.42x** |        0.16x |
| `users_assign_update`        | users-1mb               |           8.9 |                9.3 |       0.96x |        2.43x |
| `users_assign_update`        | users-10mb              |          90.1 |               82.2 |  **1.10x**† |        1.41x |
| `users_del`                  | users-1mb               |           9.2 |                8.9 |   **1.03x** |        2.34x |
| `users_del`                  | users-10mb              |          92.9 |               78.5 |  **1.18x**† |        1.33x |
| `users_paths`                | users-1mb               |          33.1 |               17.6 |  **1.88x**† |        1.81x |
| `users_paths`                | users-10mb              |         349.6 |              190.7 |  **1.83x**† |        1.16x |
| `users_tojson_roundtrip`     | users-1mb               |          22.0 |               13.3 |  **1.65x**† |        2.04x |
| `users_tojson_roundtrip`     | users-10mb              |         238.6 |              137.6 |  **1.73x**† |        1.22x |
| `users_walk`                 | users-1mb               |          77.1 |               18.1 |  **4.27x**† |        2.48x |
| `users_walk`                 | users-10mb              |         770.0 |              189.3 |  **4.07x**† |        1.78x |
| `users_slurp`                | users-1mb               |           8.8 |                3.3 |   **2.64x** |        1.55x |
| `users_slurp`                | users-10mb              |          89.8 |               20.0 |  **4.48x**† |        0.38x |
| `users_stream`               | users-1mb               |          26.2 |               23.2 |  **1.13x**† |        4.78x |
| `users_stream`               | users-10mb              |         253.3 |              220.7 |  **1.15x**† |        4.64x |
| `numbers_add`                | numbers-1mb             |          27.3 |               10.7 |  **2.56x**† |        1.84x |
| `numbers_add`                | numbers-10mb            |         264.0 |               94.6 |  **2.79x**† |        0.79x |
| `numbers_add`                | numbers-100mb           |        2749.0 |              935.3 |   **2.94x** |        0.63x |
| `numbers_sort`               | numbers-1mb             |          44.7 |               29.4 |  **1.52x**† |        1.97x |
| `numbers_sort`               | numbers-10mb            |         886.7 |              327.7 |  **2.71x**† |        1.15x |
| `numbers_max`                | numbers-1mb             |          12.6 |               13.5 |       0.93x |        2.62x |
| `numbers_max`                | numbers-10mb            |         116.8 |              117.0 |      1.00x† |        1.94x |
| `numbers_max`                | numbers-100mb           |        1254.5 |             1167.6 |   **1.07x** |        1.82x |
| `numbers_map_floor`          | numbers-1mb             |          32.7 |               14.8 |  **2.21x**† |        1.97x |
| `numbers_map_floor`          | numbers-10mb            |         319.9 |              136.2 |  **2.35x**† |        1.25x |
| `numbers_map_floor`          | numbers-100mb           |        3305.8 |             1395.2 |   **2.37x** |        1.25x |
| `strings_length`             | strings-1mb             |           7.8 |                4.4 |   **1.80x** |        3.77x |
| `strings_length`             | strings-10mb            |          68.8 |               29.9 |  **2.30x**† |        1.73x |
| `strings_length`             | strings-100mb           |         721.1 |              284.5 |   **2.53x** |        1.22x |
| `strings_upcase`             | strings-1mb             |         322.2 |                8.7 | **36.93x**† |        3.28x |
| `strings_upcase`             | strings-10mb            |        3341.4 |               73.5 |  **45.44x** |        1.47x |
| `strings_upcase`             | strings-100mb           |       33756.3 |              706.7 |  **47.76x** |        1.12x |
| `strings_split`              | strings-1mb             |          11.9 |                7.7 |   **1.55x** |        3.83x |
| `strings_split`              | strings-10mb            |         108.9 |               63.1 |  **1.72x**† |        1.75x |
| `strings_test`               | strings-1mb             |          12.2 |                5.1 |   **2.42x** |        3.91x |
| `strings_test`               | strings-10mb            |         111.1 |               34.9 |  **3.18x**† |        1.81x |
| `wide_keys`                  | wide-1mb                |          29.6 |                7.2 |  **4.08x**† |        0.72x |
| `wide_keys`                  | wide-10mb               |         311.6 |               55.2 |  **5.65x**† |        0.22x |
| `wide_keys`                  | wide-100mb              |        3909.1 |              556.6 |   **7.02x** |        0.14x |
| `wide_to_entries`            | wide-1mb                |          59.4 |               52.0 |  **1.14x**† |        1.30x |
| `wide_to_entries`            | wide-10mb               |         676.7 |              504.8 |  **1.34x**† |        1.18x |
| `wide_map_values`            | wide-1mb                |         124.1 |               51.4 |  **2.41x**† |        2.17x |
| `wide_map_values`            | wide-10mb               |        1253.4 |              539.5 |   **2.32x** |        1.94x |
| `wide_add_values`            | wide-1mb                |          37.0 |               32.8 |  **1.13x**† |        1.56x |
| `wide_add_values`            | wide-10mb               |         384.1 |              290.0 |  **1.32x**† |        1.13x |
| `wide_add_values`            | wide-100mb              |        4293.4 |             2601.7 |   **1.65x** |        1.16x |
| `wide_with_entries`          | wide-1mb                |         297.5 |              146.6 |  **2.03x**† |        1.44x |
| `wide_with_entries`          | wide-10mb               |        3000.8 |             1470.3 |   **2.04x** |        1.25x |
| `nested_paths`               | nested-1mb              |           6.1 |                3.7 |   **1.67x** |        2.70x |
| `nested_paths`               | nested-10mb             |          51.0 |               23.0 |  **2.22x**† |        1.05x |
| `nested_recurse`             | nested-1mb              |           6.1 |                4.3 |   **1.40x** |        2.74x |
| `nested_recurse`             | nested-10mb             |          50.4 |               28.9 |  **1.74x**† |        1.05x |
| `comprehensive_paths`        | comprehensive-1mb       |          76.9 |               37.2 |  **2.07x**† |        1.47x |
| `comprehensive_paths`        | comprehensive-10mb      |         795.4 |              385.8 |  **2.06x**† |        1.21x |
| `comprehensive_tostream`     | comprehensive-1mb       |         155.8 |               62.2 |  **2.50x**† |        1.91x |
| `comprehensive_tostream`     | comprehensive-10mb      |        1661.2 |              617.8 |   **2.69x** |        1.72x |
| `comprehensive_numbers`      | comprehensive-1mb       |          56.9 |               27.6 |  **2.06x**† |        1.91x |
| `comprehensive_numbers`      | comprehensive-10mb      |         577.2 |              263.9 |  **2.19x**† |        1.45x |
| `ndjson_identity`            | ndjson-1mb              |          19.8 |                6.3 |  **3.15x**† |        3.68x |
| `ndjson_identity`            | ndjson-10mb             |         184.6 |               47.0 |  **3.93x**† |        3.72x |
| `ndjson_identity`            | ndjson-100mb            |        1825.7 |              452.2 |   **4.04x** |        3.75x |
| `ndjson_field`               | ndjson-1mb              |          11.3 |                8.5 |   **1.33x** |        3.93x |
| `ndjson_field`               | ndjson-10mb             |          97.4 |               67.1 |  **1.45x**† |        3.96x |
| `ndjson_field`               | ndjson-100mb            |         966.3 |              644.2 |   **1.50x** |        3.92x |
| `ndjson_select`              | ndjson-1mb              |          13.3 |               13.2 |   **1.01x** |        4.72x |
| `ndjson_select`              | ndjson-10mb             |         116.9 |              110.5 |  **1.06x**† |        4.67x |
| `ndjson_select`              | ndjson-100mb            |        1162.3 |             1071.8 |   **1.08x** |        4.78x |
| `ndjson_tsv`                 | ndjson-1mb              |          20.6 |               19.8 |  **1.04x**† |        4.43x |
| `ndjson_tsv`                 | ndjson-10mb             |         191.6 |              180.1 |  **1.06x**† |        4.46x |
| `ndjson_tsv`                 | ndjson-100mb            |        1913.1 |             1742.6 |   **1.10x** |        4.28x |
| `ndjson_slurp`               | ndjson-1mb              |          17.1 |                9.6 |  **1.79x**† |        1.36x |
| `ndjson_slurp`               | ndjson-10mb             |         189.1 |               78.2 |  **2.42x**† |        0.42x |
| `ndjson_inputs_reduce`       | ndjson-1mb              |          12.6 |               11.0 |   **1.14x** |        4.98x |
| `ndjson_inputs_reduce`       | ndjson-10mb             |         109.6 |               92.8 |  **1.18x**† |        8.13x |
| `ndjson_inputs_reduce`       | ndjson-100mb            |        1079.2 |              910.5 |   **1.19x** |       39.18x |
| `ndjson_inputs_first`        | ndjson-1mb              |           1.2 |                1.5 |       0.81x |        4.60x |
| `ndjson_inputs_first`        | ndjson-10mb             |           1.3 |                1.5 |       0.81x |        4.63x |
| `ndjson_inputs_first`        | ndjson-100mb            |           1.2 |                1.5 |       0.79x |        4.48x |
| `ndjson_raw_lines`           | ndjson-1mb              |           6.7 |                3.6 |   **1.84x** |        3.63x |
| `ndjson_raw_lines`           | ndjson-10mb             |          51.9 |               22.2 |  **2.34x**† |        6.69x |
| `ndjson_raw_lines`           | ndjson-100mb            |         509.1 |              207.6 |  **2.45x**† |       38.05x |
| `ndjson_raw_fromjson`        | ndjson-1mb              |          13.9 |               11.5 |   **1.21x** |        3.87x |
| `ndjson_raw_fromjson`        | ndjson-10mb             |         124.6 |               98.5 |  **1.26x**† |        6.92x |
| `ndjson_raw_slurp`           | ndjson-1mb              |           5.1 |                2.2 |   **2.30x** |        2.85x |
| `ndjson_raw_slurp`           | ndjson-10mb             |          42.1 |                9.6 |  **4.39x**† |        1.48x |
| `ndjson_raw_slurp`           | ndjson-100mb            |         396.4 |               84.3 |  **4.70x**† |        1.52x |
| `ndjson_three_files`         | ndjson-1mb              |          31.9 |               21.2 |  **1.50x**† |        4.52x |
| `ndjson_three_files`         | ndjson-10mb             |         303.8 |              196.8 |  **1.54x**† |       14.12x |
| `users_three_files`          | users-1mb               |          19.2 |                5.5 |  **3.50x**† |        1.65x |
| `users_three_files`          | users-10mb              |         220.4 |               44.2 |  **4.99x**† |        0.52x |
| `seq_identity`               | jsonseq-1mb             |          19.9 |                8.4 |  **2.38x**† |        4.01x |
| `seq_identity`               | jsonseq-10mb            |         185.1 |               72.0 |  **2.57x**† |        7.27x |
| `seq_identity`               | jsonseq-100mb           |        1836.2 |              711.0 |   **2.58x** |       42.88x |
| `seq_field`                  | jsonseq-1mb             |          11.4 |               10.2 |   **1.13x** |        3.90x |
| `seq_field`                  | jsonseq-10mb            |          97.6 |               87.1 |  **1.12x**† |        7.40x |
| `seq_field`                  | jsonseq-100mb           |         979.5 |              851.1 |   **1.15x** |       43.29x |
| `seq_slurp`                  | jsonseq-1mb             |          17.2 |               11.6 |  **1.49x**† |        1.39x |
| `seq_slurp`                  | jsonseq-10mb            |         188.2 |              100.3 |  **1.88x**† |        0.42x |
| `users_seq_output`           | users-1mb               |           2.4 |                3.2 |       0.75x |        3.47x |
| `users_seq_output`           | users-10mb              |          12.6 |               21.2 |      0.60x† |        5.28x |
| `seq_field_bad_first`        | jsonseq-bad-first-1mb   |          11.4 |               12.3 |       0.93x |        4.03x |
| `seq_field_bad_first`        | jsonseq-bad-first-10mb  |          98.7 |              106.1 |      0.93x† |        8.13x |
| `seq_field_bad_first`        | jsonseq-bad-first-100mb |         979.4 |             1053.1 |       0.93x |       50.88x |
| `seq_field_bad_last`         | jsonseq-bad-last-1mb    |          11.3 |               12.6 |       0.90x |        4.01x |
| `seq_field_bad_last`         | jsonseq-bad-last-10mb   |          98.0 |              110.5 |      0.89x† |        8.06x |
| `seq_field_bad_last`         | jsonseq-bad-last-100mb  |         983.2 |             1091.8 |       0.90x |       50.73x |
| `users_slurpfile`            | users-1mb               |           9.1 |                6.8 |   **1.34x** |        2.04x |
| `users_slurpfile`            | users-10mb              |          89.4 |               49.5 |  **1.81x**† |        1.26x |
| `ndjson_slurpfile`           | ndjson-1mb              |          13.6 |               10.4 |   **1.31x** |        1.79x |
| `ndjson_slurpfile`           | ndjson-10mb             |         137.0 |               85.0 |  **1.61x**† |        1.31x |
| `users_rawfile`              | users-1mb               |           3.4 |                1.7 |   **1.97x** |        3.56x |
| `users_rawfile`              | users-10mb              |          23.0 |                4.4 |  **5.21x**† |        1.82x |
| `users_rawfile`              | users-100mb             |         245.3 |               32.3 |  **7.59x**† |        0.61x |
| `args_many`                  | none                    |           1.2 |                1.4 |       0.92x |        4.19x |
| `corpus_npm_deps`            | corpus-npm              |           1.2 |                1.4 |       0.92x |        4.85x |
| `corpus_npm_scripts`         | corpus-npm              |           1.3 |                1.4 |       0.93x |        4.62x |
| `corpus_charts_titles`       | corpus-charts           |           1.2 |                1.3 |       0.95x |        3.70x |
| `corpus_graph_weights`       | corpus-graph            |           1.4 |                1.4 |       0.98x |        4.60x |
| `corpus_graph_by_source`     | corpus-graph            |           1.5 |                1.7 |       0.91x |        4.46x |
| `corpus_graph_select`        | corpus-graph            |           1.4 |                1.4 |       1.00x |        4.47x |
| `corpus_openapi_definitions` | corpus-openapi          |           1.6 |                1.4 |   **1.13x** |        4.04x |
| `corpus_openapi_strings`     | corpus-openapi          |           2.0 |                1.8 |   **1.11x** |        4.27x |
| `corpus_openapi_refs`        | corpus-openapi          |           2.2 |                2.1 |   **1.05x** |        5.09x |
| `corpus_openapi_paths`       | corpus-openapi          |           2.1 |                1.9 |   **1.14x** |        4.23x |
| `corpus_geojson_districts`   | corpus-geojson          |           2.2 |                1.5 |   **1.42x** |        3.36x |
| `corpus_geojson_types`       | corpus-geojson          |           2.2 |                1.6 |   **1.35x** |        3.91x |
| `corpus_geojson_points`      | corpus-geojson          |           6.1 |                3.1 |   **1.98x** |        4.07x |
| `corpus_tabular_hours`       | corpus-tabular          |           1.7 |                1.6 |   **1.07x** |        4.89x |
| `corpus_tabular_csv`         | corpus-tabular          |           1.8 |                1.9 |       0.94x |        4.57x |
| `corpus_tabular_group`       | corpus-tabular          |           1.7 |                2.1 |       0.83x |        4.65x |
| `corpus_pretty_numbers`      | corpus-pretty           |          15.9 |                8.0 |  **1.98x**† |        2.71x |
| `corpus_pretty_paths`        | corpus-pretty           |          20.0 |               10.0 |  **2.00x**† |        2.12x |
| `corpus_pretty_tostream`     | corpus-pretty           |          36.8 |               14.8 |  **2.49x**† |        2.47x |
<!-- /compare:jq-linux-x86_64 -->

The sections below are `bench run jq_bench` and `dev bench jq` results, measured before #5409 made those runners interleave: all jq runs, then all succinctly runs, against the `jq` on `PATH`. That is not the interleaved method of [benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method), so where a figure below disagrees with the block above, the block is the measurement.

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
**Method**: `bench run jq_bench`, measured before #5409 made it interleave (all of jq's runs, then all of succinctly's, against the `jq` named above). Historical: the tables below predate the harness; the generated *Harness comparison* block that follows (`dev compare jq --full`, run on an AWS c8g.large Graviton 4 instance at 027745e38) is the current measurement for this chip.

## Harness comparison (`dev compare jq`)

<!-- compare:jq-linux-aarch64 -->
Generated by `succinctly dev compare jq --write-docs` from [jq-linux-aarch64.json](../compare/jq-linux-aarch64.json); do not edit. Full report: [jq-linux-aarch64.md](../compare/jq-linux-aarch64.md); every platform: [jq.md](../compare/jq.md).

**Run**: `./target/release/succinctly dev compare jq --full` on ARM Neoverse-V2 (Amazon Linux 2023.12.20260803), succinctly `027745e`, run `20261111_190258_027745e`. jq-1.7.1 is built from source by the harness; the tools are interleaved within each of 21 repetitions, and a row is timed only when the outputs are identical.

**Timed binary**: the harness itself, built with `cli,simd,regex,serde`.

Geomean speedup over jq-1.7.1: **2.41x** across 221 timed rows; jq-1.7.1 is faster on 37 of them. Speedup is jq-1.7.1's time over succinctly jq's, so below 1.00x jq is faster; memory is succinctly jq's peak RSS over jq-1.7.1's. Times are medians, except where marked †: both times and the ratio are then each tool's fastest run, which run-to-run noise moves least on a long row.

| Query                        | Input                   | jq-1.7.1 (ms) | succinctly jq (ms) |     Speedup | Memory ratio |
|------------------------------|-------------------------|---------------|--------------------|-------------|--------------|
| `startup`                    | none                    |           1.3 |                1.4 |       0.93x |        3.50x |
| `identity`                   | users-1mb               |          36.0 |               11.2 |  **3.21x**† |        1.20x |
| `identity`                   | users-10mb              |         348.3 |               96.5 |  **3.61x**† |        0.39x |
| `identity`                   | users-100mb             |        3527.9 |              948.1 |   **3.72x** |        0.30x |
| `identity`                   | numbers-1mb             |          35.7 |                9.7 |  **3.69x**† |        1.01x |
| `identity`                   | numbers-10mb            |         344.5 |               81.3 |  **4.24x**† |        0.37x |
| `identity`                   | numbers-100mb           |        3443.6 |              791.9 |   **4.35x** |        0.29x |
| `identity`                   | strings-1mb             |          32.0 |                4.0 |  **8.10x**† |        3.22x |
| `identity`                   | strings-10mb            |         310.7 |               25.0 | **12.41x**† |        1.93x |
| `identity`                   | strings-100mb           |        3127.0 |              240.1 |  **13.02x** |        1.57x |
| `identity`                   | wide-1mb                |          57.2 |               17.5 |  **3.26x**† |        0.64x |
| `identity`                   | wide-10mb               |         602.4 |              151.8 |  **3.97x**† |        0.29x |
| `identity`                   | wide-100mb              |        6585.5 |             1464.4 |   **4.50x** |        0.19x |
| `identity`                   | nested-1mb              |          35.5 |                4.9 |  **7.28x**† |        2.32x |
| `identity`                   | nested-10mb             |         343.0 |               34.4 |  **9.97x**† |        1.33x |
| `identity`                   | nested-100mb            |        3434.3 |              335.5 |  **10.24x** |        1.12x |
| `identity`                   | comprehensive-1mb       |          63.6 |               15.4 |  **4.13x**† |        0.65x |
| `identity`                   | comprehensive-10mb      |         614.7 |              131.1 |  **4.69x**† |        0.25x |
| `identity`                   | comprehensive-100mb     |        6108.5 |             1326.6 |   **4.60x** |        0.20x |
| `identity`                   | corpus-charts           |           1.3 |                1.3 |       0.95x |        3.41x |
| `identity`                   | corpus-npm              |           1.3 |                1.3 |       1.00x |        3.53x |
| `identity`                   | corpus-graph            |           2.0 |                1.6 |   **1.25x** |        3.44x |
| `identity`                   | corpus-openapi          |           2.7 |                1.8 |   **1.54x** |        3.60x |
| `identity`                   | corpus-geojson          |           7.3 |                2.7 |   **2.75x** |        2.65x |
| `identity`                   | corpus-tabular          |           2.6 |                1.7 |   **1.53x** |        3.29x |
| `identity`                   | corpus-pretty           |          20.1 |                6.6 |  **3.03x**† |        1.59x |
| `identity_compact`           | users-1mb               |          28.5 |                3.8 |  **7.50x**† |        1.03x |
| `identity_compact`           | users-10mb              |         274.0 |               23.6 | **11.63x**† |        0.23x |
| `identity_compact`           | users-100mb             |        2798.5 |              226.9 |  **12.34x** |        0.14x |
| `identity_compact`           | numbers-1mb             |          28.9 |                5.2 |  **5.60x**† |        0.85x |
| `identity_compact`           | numbers-10mb            |         277.6 |               37.0 |  **7.50x**† |        0.22x |
| `identity_compact`           | numbers-100mb           |        2793.2 |              360.8 |   **7.74x** |        0.14x |
| `identity_compact`           | strings-1mb             |          31.4 |                2.6 | **12.04x**† |        2.87x |
| `identity_compact`           | strings-10mb            |         303.7 |               11.9 | **25.44x**† |        1.26x |
| `identity_compact`           | strings-100mb           |        3056.8 |              110.0 |  **27.79x** |        0.86x |
| `identity_compact`           | wide-1mb                |          53.4 |                6.8 |  **7.90x**† |        0.60x |
| `identity_compact`           | wide-10mb               |         570.4 |               53.0 | **10.75x**† |        0.24x |
| `identity_compact`           | wide-100mb              |        6256.5 |             1456.7 |   **4.30x** |        0.18x |
| `identity_compact`           | nested-1mb              |          35.4 |                2.6 | **13.51x**† |        2.06x |
| `identity_compact`           | nested-10mb             |         342.1 |               12.2 | **28.02x**† |        0.87x |
| `identity_compact`           | nested-100mb            |        3426.6 |              113.7 |  **30.14x** |        0.62x |
| `identity_compact`           | comprehensive-1mb       |          41.7 |               14.8 |  **2.82x**† |        0.60x |
| `identity_compact`           | comprehensive-10mb      |         403.7 |              125.5 |  **3.22x**† |        0.18x |
| `identity_compact`           | comprehensive-100mb     |        4013.0 |             1262.9 |   **3.18x** |        0.14x |
| `identity_compact`           | corpus-charts           |           1.3 |                1.3 |       0.94x |        3.48x |
| `identity_compact`           | corpus-npm              |           1.3 |                1.4 |       0.99x |        3.51x |
| `identity_compact`           | corpus-graph            |           1.7 |                1.3 |   **1.31x** |        3.27x |
| `identity_compact`           | corpus-openapi          |           2.4 |                1.8 |   **1.35x** |        3.36x |
| `identity_compact`           | corpus-geojson          |           3.9 |                2.5 |   **1.57x** |        2.63x |
| `identity_compact`           | corpus-tabular          |           2.5 |                1.7 |   **1.49x** |        3.50x |
| `identity_compact`           | corpus-pretty           |          12.9 |                6.4 |   **2.02x** |        1.58x |
| `users_field`                | users-1mb               |          11.9 |                3.3 |   **3.59x** |        1.22x |
| `users_field`                | users-10mb              |         114.6 |               18.6 |  **6.17x**† |        0.27x |
| `users_field`                | users-100mb             |        1164.3 |              174.0 |   **6.69x** |        0.15x |
| `users_length`               | users-1mb               |          12.6 |                3.6 |   **3.54x** |        1.27x |
| `users_length`               | users-10mb              |         119.9 |               19.9 |  **6.02x**† |        0.30x |
| `users_length`               | users-100mb             |        1218.2 |              188.2 |   **6.47x** |        0.18x |
| `users_names`                | users-1mb               |          13.0 |                7.0 |   **1.84x** |        1.19x |
| `users_names`                | users-10mb              |         124.6 |               55.1 |  **2.26x**† |        0.26x |
| `users_names`                | users-100mb             |        1266.5 |              540.0 |   **2.35x** |        0.15x |
| `users_map_select`           | users-1mb               |          14.5 |                8.9 |  **1.62x**† |        1.46x |
| `users_map_select`           | users-10mb              |         142.6 |               74.2 |  **1.92x**† |        0.40x |
| `users_map_select`           | users-100mb             |        1462.5 |              734.0 |   **1.99x** |        0.28x |
| `users_sort_by`              | users-1mb               |          18.7 |               13.8 |  **1.35x**† |        1.35x |
| `users_sort_by`              | users-10mb              |         222.4 |              129.7 |  **1.72x**† |        0.53x |
| `users_group_by`             | users-1mb               |          19.8 |               13.3 |  **1.49x**† |        1.33x |
| `users_group_by`             | users-10mb              |         242.6 |              124.9 |  **1.94x**† |        0.52x |
| `users_unique_by`            | users-1mb               |          19.9 |               13.7 |  **1.45x**† |        1.37x |
| `users_unique_by`            | users-10mb              |         242.6 |              128.4 |  **1.89x**† |        0.54x |
| `users_reduce`               | users-1mb               |          13.9 |               12.1 |   **1.15x** |        1.42x |
| `users_reduce`               | users-10mb              |         134.9 |              104.1 |  **1.30x**† |        0.29x |
| `users_reduce`               | users-100mb             |        1335.8 |             1025.6 |   **1.30x** |        0.16x |
| `users_to_entries`           | users-1mb               |          12.0 |                3.4 |   **3.48x** |        1.44x |
| `users_to_entries`           | users-10mb              |         115.1 |               18.7 |  **6.15x**† |        0.30x |
| `users_to_entries`           | users-100mb             |        1165.2 |              173.9 |   **6.70x** |        0.16x |
| `users_csv`                  | users-1mb               |          21.0 |               17.5 |  **1.20x**† |        1.24x |
| `users_csv`                  | users-10mb              |         198.8 |              159.2 |  **1.25x**† |        0.27x |
| `users_csv`                  | users-100mb             |        2024.7 |             1588.9 |   **1.27x** |        0.15x |
| `users_regex`                | users-1mb               |          20.6 |                9.1 |  **2.26x**† |        1.53x |
| `users_regex`                | users-10mb              |         199.9 |               74.0 |  **2.70x**† |        0.42x |
| `users_ascii_downcase`       | users-1mb               |          35.6 |               10.1 |  **3.51x**† |        1.39x |
| `users_ascii_downcase`       | users-10mb              |         367.4 |               87.4 |  **4.20x**† |        0.41x |
| `users_ascii_downcase`       | users-100mb             |        3896.7 |              862.7 |   **4.52x** |        0.30x |
| `users_first_early_exit`     | users-1mb               |          11.9 |                3.0 |   **3.95x** |        1.23x |
| `users_first_early_exit`     | users-10mb              |         107.3 |               15.9 |  **6.76x**† |        0.27x |
| `users_first_early_exit`     | users-100mb             |        1089.8 |              148.1 |   **7.36x** |        0.15x |
| `users_limit_early_exit`     | users-1mb               |          12.0 |                3.1 |   **3.91x** |        1.30x |
| `users_limit_early_exit`     | users-10mb              |         114.1 |               15.9 |  **7.18x**† |        0.28x |
| `users_limit_early_exit`     | users-100mb             |        1166.2 |              147.9 |   **7.88x** |        0.15x |
| `users_assign_update`        | users-1mb               |          12.0 |               11.5 |   **1.04x** |        2.18x |
| `users_assign_update`        | users-10mb              |         116.1 |               96.0 |  **1.21x**† |        1.38x |
| `users_del`                  | users-1mb               |          12.6 |               11.9 |   **1.06x** |        2.11x |
| `users_del`                  | users-10mb              |         119.4 |               94.6 |  **1.26x**† |        1.29x |
| `users_paths`                | users-1mb               |          41.9 |               24.6 |  **1.70x**† |        1.65x |
| `users_paths`                | users-10mb              |         419.5 |              216.1 |  **1.94x**† |        1.14x |
| `users_tojson_roundtrip`     | users-1mb               |          30.2 |               19.2 |  **1.57x**† |        1.84x |
| `users_tojson_roundtrip`     | users-10mb              |         302.2 |              185.5 |  **1.63x**† |        1.19x |
| `users_walk`                 | users-1mb               |          98.1 |               24.4 |  **4.01x**† |        2.28x |
| `users_walk`                 | users-10mb              |         968.4 |              213.6 |  **4.53x**† |        1.75x |
| `users_slurp`                | users-1mb               |          12.2 |                3.6 |   **3.36x** |        1.25x |
| `users_slurp`                | users-10mb              |         117.4 |               21.0 |  **5.59x**† |        0.34x |
| `users_stream`               | users-1mb               |          34.7 |               40.0 |      0.87x† |        4.24x |
| `users_stream`               | users-10mb              |         335.1 |              385.3 |      0.87x† |        4.23x |
| `numbers_add`                | numbers-1mb             |          36.6 |               16.0 |  **2.29x**† |        1.64x |
| `numbers_add`                | numbers-10mb            |         353.0 |              142.6 |  **2.48x**† |        0.75x |
| `numbers_add`                | numbers-100mb           |        3542.7 |             1406.8 |   **2.52x** |        0.63x |
| `numbers_sort`               | numbers-1mb             |          59.0 |               41.4 |  **1.42x**† |        1.81x |
| `numbers_sort`               | numbers-10mb            |        1351.9 |              446.9 |   **3.02x** |        1.12x |
| `numbers_max`                | numbers-1mb             |          17.5 |               19.3 |      0.90x† |        2.48x |
| `numbers_max`                | numbers-10mb            |         165.0 |              174.8 |      0.94x† |        1.92x |
| `numbers_max`                | numbers-100mb           |        1671.2 |             1728.3 |       0.97x |        1.82x |
| `numbers_map_floor`          | numbers-1mb             |          42.1 |               19.3 |  **2.19x**† |        1.82x |
| `numbers_map_floor`          | numbers-10mb            |         407.2 |              178.6 |  **2.28x**† |        1.22x |
| `numbers_map_floor`          | numbers-100mb           |        4097.1 |             1754.1 |   **2.34x** |        1.24x |
| `strings_length`             | strings-1mb             |          12.4 |                4.3 |   **2.87x** |        3.46x |
| `strings_length`             | strings-10mb            |         114.1 |               28.3 |  **4.03x**† |        1.62x |
| `strings_length`             | strings-100mb           |        1152.8 |              267.6 |   **4.31x** |        1.21x |
| `strings_upcase`             | strings-1mb             |         422.2 |               11.8 | **35.88x**† |        2.93x |
| `strings_upcase`             | strings-10mb            |        4242.9 |              101.2 |  **41.94x** |        1.40x |
| `strings_upcase`             | strings-100mb           |       42458.4 |              977.6 |  **43.43x** |        1.11x |
| `strings_split`              | strings-1mb             |          17.7 |               10.4 |  **1.70x**† |        3.46x |
| `strings_split`              | strings-10mb            |         167.9 |               94.4 |  **1.78x**† |        1.62x |
| `strings_test`               | strings-1mb             |          18.8 |                5.4 |  **3.49x**† |        3.66x |
| `strings_test`               | strings-10mb            |         178.6 |               39.0 |  **4.58x**† |        1.68x |
| `wide_keys`                  | wide-1mb                |          37.7 |               10.1 |  **3.73x**† |        0.60x |
| `wide_keys`                  | wide-10mb               |         417.5 |               81.4 |  **5.13x**† |        0.20x |
| `wide_keys`                  | wide-100mb              |        4854.6 |              822.1 |   **5.91x** |        0.14x |
| `wide_to_entries`            | wide-1mb                |          79.6 |               70.0 |  **1.14x**† |        1.27x |
| `wide_to_entries`            | wide-10mb               |         888.4 |              666.2 |   **1.33x** |        1.18x |
| `wide_map_values`            | wide-1mb                |         161.9 |               71.3 |  **2.27x**† |        2.05x |
| `wide_map_values`            | wide-10mb               |        1631.1 |              734.4 |   **2.22x** |        1.93x |
| `wide_add_values`            | wide-1mb                |          49.1 |               42.7 |  **1.15x**† |        1.45x |
| `wide_add_values`            | wide-10mb               |         511.7 |              375.1 |  **1.36x**† |        1.12x |
| `wide_add_values`            | wide-100mb              |        5604.5 |             3474.0 |   **1.61x** |        1.16x |
| `wide_with_entries`          | wide-1mb                |         374.9 |              198.0 |  **1.89x**† |        1.41x |
| `wide_with_entries`          | wide-10mb               |        3692.9 |             1880.6 |   **1.96x** |        1.24x |
| `nested_paths`               | nested-1mb              |          10.7 |                3.7 |   **2.89x** |        2.24x |
| `nested_paths`               | nested-10mb             |          92.4 |               22.1 |  **4.19x**† |        0.93x |
| `nested_recurse`             | nested-1mb              |          10.7 |                4.9 |   **2.16x** |        2.37x |
| `nested_recurse`             | nested-10mb             |          89.4 |               34.4 |  **2.60x**† |        0.93x |
| `comprehensive_paths`        | comprehensive-1mb       |          95.2 |               50.2 |  **1.90x**† |        1.41x |
| `comprehensive_paths`        | comprehensive-10mb      |         940.6 |              475.1 |  **1.98x**† |        1.20x |
| `comprehensive_tostream`     | comprehensive-1mb       |         191.6 |               83.7 |  **2.29x**† |        1.87x |
| `comprehensive_tostream`     | comprehensive-10mb      |        1882.9 |              781.0 |   **2.41x** |        1.72x |
| `comprehensive_numbers`      | comprehensive-1mb       |          75.0 |               36.5 |  **2.06x**† |        1.83x |
| `comprehensive_numbers`      | comprehensive-10mb      |         742.8 |              346.6 |  **2.14x**† |        1.44x |
| `ndjson_identity`            | ndjson-1mb              |          40.2 |               10.4 |  **3.86x**† |        3.72x |
| `ndjson_identity`            | ndjson-10mb             |         384.7 |               88.2 |  **4.36x**† |        8.08x |
| `ndjson_identity`            | ndjson-100mb            |        3813.2 |              838.7 |   **4.55x** |       50.59x |
| `ndjson_field`               | ndjson-1mb              |          16.2 |               12.5 |  **1.30x**† |        3.84x |
| `ndjson_field`               | ndjson-10mb             |         148.5 |              109.4 |  **1.36x**† |        7.90x |
| `ndjson_field`               | ndjson-100mb            |        1458.7 |             1054.2 |   **1.38x** |       50.67x |
| `ndjson_select`              | ndjson-1mb              |          18.6 |               19.3 |      0.97x† |        4.37x |
| `ndjson_select`              | ndjson-10mb             |         171.6 |              173.2 |      0.99x† |        8.91x |
| `ndjson_select`              | ndjson-100mb            |        1688.0 |             1674.5 |   **1.01x** |       52.08x |
| `ndjson_tsv`                 | ndjson-1mb              |          28.2 |               31.3 |      0.90x† |        4.13x |
| `ndjson_tsv`                 | ndjson-10mb             |         265.3 |              288.8 |      0.92x† |        8.85x |
| `ndjson_tsv`                 | ndjson-100mb            |        2604.6 |             2804.5 |       0.93x |       51.06x |
| `ndjson_slurp`               | ndjson-1mb              |          23.4 |               13.3 |  **1.76x**† |        1.10x |
| `ndjson_slurp`               | ndjson-10mb             |         227.1 |              114.2 |  **1.99x**† |        0.38x |
| `ndjson_inputs_reduce`       | ndjson-1mb              |          17.7 |               19.1 |      0.93x† |        4.68x |
| `ndjson_inputs_reduce`       | ndjson-10mb             |         161.7 |              169.8 |      0.95x† |        8.36x |
| `ndjson_inputs_reduce`       | ndjson-100mb            |        1583.8 |             1650.0 |       0.96x |       45.23x |
| `ndjson_inputs_first`        | ndjson-1mb              |           1.3 |                1.7 |       0.75x |        4.14x |
| `ndjson_inputs_first`        | ndjson-10mb             |           1.3 |                1.7 |       0.74x |        4.04x |
| `ndjson_inputs_first`        | ndjson-100mb            |           1.3 |                1.8 |       0.72x |        4.27x |
| `ndjson_raw_lines`           | ndjson-1mb              |           9.1 |                4.9 |   **1.87x** |        3.49x |
| `ndjson_raw_lines`           | ndjson-10mb             |          78.3 |               34.7 |  **2.25x**† |        7.28x |
| `ndjson_raw_lines`           | ndjson-100mb            |         773.3 |              328.4 |   **2.35x** |       47.05x |
| `ndjson_raw_fromjson`        | ndjson-1mb              |          20.1 |               18.0 |  **1.11x**† |        3.82x |
| `ndjson_raw_fromjson`        | ndjson-10mb             |         185.3 |              161.6 |  **1.15x**† |        7.58x |
| `ndjson_raw_slurp`           | ndjson-1mb              |           6.8 |                2.1 |   **3.20x** |        2.68x |
| `ndjson_raw_slurp`           | ndjson-10mb             |          59.1 |                9.6 |  **6.15x**† |        1.43x |
| `ndjson_raw_slurp`           | ndjson-100mb            |         563.3 |               76.1 |  **7.40x**† |        1.51x |
| `ndjson_three_files`         | ndjson-1mb              |          47.5 |               34.0 |  **1.40x**† |        4.63x |
| `ndjson_three_files`         | ndjson-10mb             |         457.2 |              318.9 |  **1.43x**† |       16.42x |
| `users_three_files`          | users-1mb               |          30.6 |                7.0 |  **4.35x**† |        1.38x |
| `users_three_files`          | users-10mb              |         310.8 |               56.3 |  **5.52x**† |        0.48x |
| `seq_identity`               | jsonseq-1mb             |          40.2 |               13.9 |  **2.89x**† |        3.81x |
| `seq_identity`               | jsonseq-10mb            |         384.6 |              122.4 |  **3.14x**† |        8.08x |
| `seq_identity`               | jsonseq-100mb           |        3801.5 |             1183.3 |   **3.21x** |       50.88x |
| `seq_field`                  | jsonseq-1mb             |          16.3 |               15.7 |  **1.04x**† |        3.96x |
| `seq_field`                  | jsonseq-10mb            |         148.6 |              139.9 |  **1.06x**† |        8.01x |
| `seq_field`                  | jsonseq-100mb           |        1464.3 |             1359.6 |   **1.08x** |       50.62x |
| `seq_slurp`                  | jsonseq-1mb             |          23.5 |               17.2 |  **1.37x**† |        1.13x |
| `seq_slurp`                  | jsonseq-10mb            |         227.1 |              152.1 |  **1.49x**† |        0.38x |
| `users_seq_output`           | users-1mb               |           2.8 |                5.5 |       0.51x |        3.28x |
| `users_seq_output`           | users-10mb              |          17.7 |               44.1 |      0.40x† |        5.59x |
| `seq_field_bad_first`        | jsonseq-bad-first-1mb   |          16.3 |               19.0 |      0.86x† |        3.84x |
| `seq_field_bad_first`        | jsonseq-bad-first-10mb  |         148.1 |              171.8 |      0.86x† |        8.89x |
| `seq_field_bad_first`        | jsonseq-bad-first-100mb |        1456.5 |             1674.9 |       0.87x |       59.93x |
| `seq_field_bad_last`         | jsonseq-bad-last-1mb    |          16.2 |               19.8 |      0.82x† |        3.76x |
| `seq_field_bad_last`         | jsonseq-bad-last-10mb   |         148.3 |              179.6 |      0.83x† |        8.76x |
| `seq_field_bad_last`         | jsonseq-bad-last-100mb  |        1460.8 |             1757.8 |       0.83x |       60.59x |
| `users_slurpfile`            | users-1mb               |          12.2 |                8.1 |   **1.52x** |        1.84x |
| `users_slurpfile`            | users-10mb              |         108.7 |               66.7 |  **1.63x**† |        1.25x |
| `ndjson_slurpfile`           | ndjson-1mb              |          17.6 |               12.8 |  **1.37x**† |        1.68x |
| `ndjson_slurpfile`           | ndjson-10mb             |         163.3 |              112.6 |  **1.45x**† |        1.30x |
| `users_rawfile`              | users-1mb               |           4.2 |                1.7 |   **2.41x** |        3.36x |
| `users_rawfile`              | users-10mb              |          31.1 |                4.6 |  **6.75x**† |        1.67x |
| `users_rawfile`              | users-100mb             |         322.7 |               32.5 |  **9.93x**† |        0.59x |
| `args_many`                  | none                    |           1.3 |                1.5 |       0.88x |        3.80x |
| `corpus_npm_deps`            | corpus-npm              |           1.3 |                1.5 |       0.87x |        4.31x |
| `corpus_npm_scripts`         | corpus-npm              |           1.3 |                1.5 |       0.91x |        4.34x |
| `corpus_charts_titles`       | corpus-charts           |           1.3 |                1.4 |       0.91x |        3.78x |
| `corpus_graph_weights`       | corpus-graph            |           1.5 |                1.6 |       0.94x |        4.35x |
| `corpus_graph_by_source`     | corpus-graph            |           1.7 |                2.0 |       0.83x |        4.52x |
| `corpus_graph_select`        | corpus-graph            |           1.5 |                1.5 |       1.00x |        4.26x |
| `corpus_openapi_definitions` | corpus-openapi          |           1.8 |                1.5 |   **1.21x** |        3.77x |
| `corpus_openapi_strings`     | corpus-openapi          |           2.3 |                2.1 |   **1.12x** |        4.10x |
| `corpus_openapi_refs`        | corpus-openapi          |           2.6 |                2.6 |   **1.01x** |        4.73x |
| `corpus_openapi_paths`       | corpus-openapi          |           2.5 |                2.3 |   **1.11x** |        3.99x |
| `corpus_geojson_districts`   | corpus-geojson          |           2.7 |                1.8 |   **1.51x** |        3.09x |
| `corpus_geojson_types`       | corpus-geojson          |           2.7 |                1.9 |   **1.43x** |        3.66x |
| `corpus_geojson_points`      | corpus-geojson          |           7.4 |                4.0 |   **1.87x** |        3.68x |
| `corpus_tabular_hours`       | corpus-tabular          |           2.0 |                1.8 |   **1.09x** |        4.26x |
| `corpus_tabular_csv`         | corpus-tabular          |           2.1 |                2.3 |       0.91x |        4.33x |
| `corpus_tabular_group`       | corpus-tabular          |           2.0 |                2.5 |       0.82x |        4.74x |
| `corpus_pretty_numbers`      | corpus-pretty           |          21.7 |               11.1 |  **1.96x**† |        2.43x |
| `corpus_pretty_paths`        | corpus-pretty           |          25.9 |               14.4 |  **1.79x**† |        1.92x |
| `corpus_pretty_tostream`     | corpus-pretty           |          45.9 |               20.9 |  **2.19x**† |        2.29x |
<!-- /compare:jq-linux-aarch64 -->

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
**Method**: `bench run jq_bench`, measured before #5409 made it interleave (all of jq's runs, then all of succinctly's, against the `jq` named above). Historical: no `dev compare jq` run exists for this platform, so it has no generated *Harness comparison* block.

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
**Method**: `bench run jq_bench`, measured before #5409 made it interleave (all of jq's runs, then all of succinctly's, against the `jq` named above). Historical: no `dev compare jq` run exists for this platform, so it has no generated *Harness comparison* block.

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

**Run**: `./target/release/succinctly dev compare jq --full --huge` on Apple M4 Pro (macOS 26.5.1), succinctly `f056da9c6`, run `20261112_090945_f056da9c6`. jq-1.7.1 is built from source by the harness; the tools are interleaved within each of 21 repetitions, and a row is timed only when the outputs are identical.

**Timed binary**: the harness itself, built with `cli,simd,regex,serde`.

Geomean speedup over jq-1.7.1: **2.29x** across 221 timed rows; jq-1.7.1 is faster on 39 of them. Speedup is jq-1.7.1's time over succinctly jq's, so below 1.00x jq is faster; memory is succinctly jq's peak RSS over jq-1.7.1's. Times are medians, except where marked †: both times and the ratio are then each tool's fastest run, which run-to-run noise moves least on a long row.

| Query                        | Input                   | jq-1.7.1 (ms) | succinctly jq (ms) |     Speedup | Memory ratio |
|------------------------------|-------------------------|---------------|--------------------|-------------|--------------|
| `startup`                    | none                    |           1.8 |                2.9 |       0.61x |        3.75x |
| `identity`                   | users-1mb               |          24.9 |                8.3 |  **2.99x**† |        1.22x |
| `identity`                   | users-10mb              |         243.9 |               56.0 |  **4.36x**† |        0.46x |
| `identity`                   | users-100mb             |        2454.7 |              534.7 |   **4.59x** |        0.30x |
| `identity`                   | numbers-1mb             |          22.6 |                7.4 |  **3.05x**† |        1.05x |
| `identity`                   | numbers-10mb            |         209.9 |               48.1 |  **4.36x**† |        0.37x |
| `identity`                   | numbers-100mb           |        2113.9 |              453.6 |   **4.66x** |        0.25x |
| `identity`                   | strings-1mb             |          21.5 |                3.7 |  **5.77x**† |        3.24x |
| `identity`                   | strings-10mb            |         201.3 |               13.5 | **14.89x**† |        1.89x |
| `identity`                   | strings-100mb           |        2124.3 |              117.6 |  **18.06x** |        1.30x |
| `identity`                   | wide-1mb                |          36.0 |               11.1 |  **3.25x**† |        0.52x |
| `identity`                   | wide-10mb               |         347.5 |               78.3 |  **4.44x**† |        0.24x |
| `identity`                   | wide-100mb              |        3909.8 |              781.5 |   **5.00x** |        0.21x |
| `identity`                   | nested-1mb              |          23.1 |                4.0 |  **5.72x**† |        2.52x |
| `identity`                   | nested-10mb             |         219.4 |               17.0 | **12.94x**† |        1.28x |
| `identity`                   | nested-100mb            |        2286.9 |              164.0 |  **13.94x** |        1.13x |
| `identity`                   | comprehensive-1mb       |          42.0 |               11.2 |  **3.75x**† |        0.79x |
| `identity`                   | comprehensive-10mb      |         401.8 |               77.7 |  **5.17x**† |        0.30x |
| `identity`                   | comprehensive-100mb     |        4008.5 |              772.5 |   **5.19x** |        0.23x |
| `identity`                   | corpus-charts           |           1.7 |                2.8 |       0.61x |        3.79x |
| `identity`                   | corpus-npm              |           1.8 |                2.7 |       0.65x |        3.76x |
| `identity`                   | corpus-graph            |           2.2 |                2.9 |       0.76x |        3.45x |
| `identity`                   | corpus-openapi          |           2.9 |                3.1 |       0.94x |        3.35x |
| `identity`                   | corpus-geojson          |           5.9 |                3.6 |   **1.65x** |        2.48x |
| `identity`                   | corpus-tabular          |           2.7 |                2.9 |       0.95x |        3.48x |
| `identity`                   | corpus-pretty           |          16.5 |                5.8 |  **2.87x**† |        1.66x |
| `identity_compact`           | users-1mb               |          18.9 |                4.0 |  **4.72x**† |        1.09x |
| `identity_compact`           | users-10mb              |         183.2 |               15.7 | **11.69x**† |        0.27x |
| `identity_compact`           | users-100mb             |        1890.8 |              128.5 |  **14.72x** |        0.14x |
| `identity_compact`           | numbers-1mb             |          17.5 |                4.9 |  **3.55x**† |        0.93x |
| `identity_compact`           | numbers-10mb            |         161.0 |               25.6 |  **6.29x**† |        0.22x |
| `identity_compact`           | numbers-100mb           |        1600.9 |              229.1 |   **6.99x** |        0.13x |
| `identity_compact`           | strings-1mb             |          20.7 |                3.1 |  **6.69x**† |        2.93x |
| `identity_compact`           | strings-10mb            |         197.3 |                7.5 | **26.46x**† |        1.26x |
| `identity_compact`           | strings-100mb           |        2040.0 |               47.7 |  **42.77x** |        0.73x |
| `identity_compact`           | wide-1mb                |          32.6 |                5.7 |  **5.68x**† |        0.53x |
| `identity_compact`           | wide-10mb               |         317.4 |               30.4 | **10.45x**† |        0.22x |
| `identity_compact`           | wide-100mb              |        3586.7 |              773.0 |   **4.64x** |        0.19x |
| `identity_compact`           | nested-1mb              |          23.7 |                3.1 |  **7.76x**† |        2.26x |
| `identity_compact`           | nested-10mb             |         220.7 |                7.6 | **29.19x**† |        0.87x |
| `identity_compact`           | nested-100mb            |        2269.6 |               46.0 |  **49.30x** |        0.64x |
| `identity_compact`           | comprehensive-1mb       |          26.2 |               10.9 |  **2.40x**† |        0.72x |
| `identity_compact`           | comprehensive-10mb      |         245.1 |               74.6 |  **3.29x**† |        0.23x |
| `identity_compact`           | comprehensive-100mb     |        2445.5 |              730.8 |   **3.35x** |        0.15x |
| `identity_compact`           | corpus-charts           |           1.7 |                2.8 |       0.61x |        3.77x |
| `identity_compact`           | corpus-npm              |           1.8 |                2.7 |       0.65x |        3.77x |
| `identity_compact`           | corpus-graph            |           2.1 |                2.7 |       0.77x |        3.39x |
| `identity_compact`           | corpus-openapi          |           2.6 |                3.0 |       0.86x |        3.38x |
| `identity_compact`           | corpus-geojson          |           3.4 |                3.4 |       1.00x |        2.44x |
| `identity_compact`           | corpus-tabular          |           2.9 |                2.9 |       0.98x |        3.53x |
| `identity_compact`           | corpus-pretty           |          11.4 |                5.8 |   **1.97x** |        1.60x |
| `users_field`                | users-1mb               |           7.2 |                3.8 |   **1.89x** |        1.21x |
| `users_field`                | users-10mb              |          58.1 |               13.0 |  **4.47x**† |        0.30x |
| `users_field`                | users-100mb             |         572.4 |              101.3 |  **5.65x**† |        0.15x |
| `users_length`               | users-1mb               |           7.5 |                3.9 |   **1.91x** |        1.25x |
| `users_length`               | users-10mb              |          61.6 |               13.8 |  **4.48x**† |        0.38x |
| `users_length`               | users-100mb             |         649.9 |              105.6 |  **6.16x**† |        0.19x |
| `users_names`                | users-1mb               |           8.0 |                6.0 |   **1.32x** |        1.20x |
| `users_names`                | users-10mb              |          68.7 |               34.8 |  **1.97x**† |        0.30x |
| `users_names`                | users-100mb             |         662.1 |              324.3 |   **2.04x** |        0.15x |
| `users_map_select`           | users-1mb               |           8.5 |                7.3 |   **1.17x** |        1.38x |
| `users_map_select`           | users-10mb              |          73.0 |               43.3 |  **1.68x**† |        0.48x |
| `users_map_select`           | users-100mb             |         728.3 |              408.5 |   **1.78x** |        0.28x |
| `users_sort_by`              | users-1mb               |          12.0 |                9.4 |   **1.27x** |        1.41x |
| `users_sort_by`              | users-10mb              |         119.2 |               65.7 |  **1.81x**† |        0.62x |
| `users_group_by`             | users-1mb               |          12.4 |                9.4 |   **1.32x** |        1.36x |
| `users_group_by`             | users-10mb              |         131.5 |               64.2 |  **2.05x**† |        0.56x |
| `users_unique_by`            | users-1mb               |          12.5 |                9.5 |   **1.31x** |        1.37x |
| `users_unique_by`            | users-10mb              |         132.2 |               65.4 |  **2.02x**† |        0.60x |
| `users_reduce`               | users-1mb               |           8.3 |                8.9 |       0.93x |        1.31x |
| `users_reduce`               | users-10mb              |          70.3 |               56.6 |  **1.24x**† |        0.31x |
| `users_reduce`               | users-100mb             |         696.7 |              537.4 |   **1.30x** |        0.15x |
| `users_to_entries`           | users-1mb               |           7.2 |                4.0 |   **1.79x** |        1.33x |
| `users_to_entries`           | users-10mb              |          58.0 |               13.3 |  **4.35x**† |        0.32x |
| `users_to_entries`           | users-100mb             |         575.2 |              101.4 |  **5.67x**† |        0.15x |
| `users_csv`                  | users-1mb               |          13.6 |               13.7 |       0.99x |        1.23x |
| `users_csv`                  | users-10mb              |         121.5 |              103.8 |  **1.17x**† |        0.30x |
| `users_csv`                  | users-100mb             |        1216.8 |             1008.1 |   **1.21x** |        0.15x |
| `users_regex`                | users-1mb               |          13.7 |                7.6 |  **1.81x**† |        1.41x |
| `users_regex`                | users-10mb              |         116.7 |               44.1 |  **2.65x**† |        0.48x |
| `users_ascii_downcase`       | users-1mb               |          22.4 |                8.5 |  **2.63x**† |        1.35x |
| `users_ascii_downcase`       | users-10mb              |         214.9 |               48.6 |  **4.42x**† |        0.47x |
| `users_ascii_downcase`       | users-100mb             |        2241.5 |              455.8 |   **4.92x** |        0.31x |
| `users_first_early_exit`     | users-1mb               |           7.7 |                4.4 |   **1.74x** |        1.23x |
| `users_first_early_exit`     | users-10mb              |          59.8 |               12.3 |  **4.85x**† |        0.30x |
| `users_first_early_exit`     | users-100mb             |         592.4 |               88.0 |  **6.73x**† |        0.15x |
| `users_limit_early_exit`     | users-1mb               |           7.4 |                4.0 |   **1.84x** |        1.25x |
| `users_limit_early_exit`     | users-10mb              |          58.3 |               11.8 |  **4.96x**† |        0.31x |
| `users_limit_early_exit`     | users-100mb             |         577.4 |               86.7 |  **6.66x**† |        0.15x |
| `users_assign_update`        | users-1mb               |           7.4 |                7.1 |   **1.04x** |        1.93x |
| `users_assign_update`        | users-10mb              |          58.5 |               43.4 |  **1.35x**† |        1.22x |
| `users_del`                  | users-1mb               |           7.5 |                7.2 |   **1.04x** |        1.87x |
| `users_del`                  | users-10mb              |          60.6 |               43.1 |  **1.41x**† |        1.17x |
| `users_paths`                | users-1mb               |          23.5 |               12.2 |  **1.93x**† |        1.43x |
| `users_paths`                | users-10mb              |         219.1 |               94.0 |  **2.33x**† |        0.94x |
| `users_tojson_roundtrip`     | users-1mb               |          17.7 |               10.0 |  **1.77x**† |        1.54x |
| `users_tojson_roundtrip`     | users-10mb              |         167.6 |               75.6 |  **2.22x**† |        0.98x |
| `users_walk`                 | users-1mb               |          63.2 |               12.2 |  **5.16x**† |        1.92x |
| `users_walk`                 | users-10mb              |         603.0 |               94.1 |  **6.41x**† |        1.42x |
| `users_slurp`                | users-1mb               |           7.3 |                4.2 |   **1.76x** |        1.31x |
| `users_slurp`                | users-10mb              |          58.6 |               13.8 |  **4.23x**† |        0.39x |
| `users_stream`               | users-1mb               |          23.7 |               21.3 |  **1.12x**† |        4.22x |
| `users_stream`               | users-10mb              |         215.2 |              177.0 |  **1.22x**† |        4.21x |
| `numbers_add`                | numbers-1mb             |          20.9 |               11.6 |  **1.81x**† |        1.66x |
| `numbers_add`                | numbers-10mb            |         191.0 |               88.0 |  **2.17x**† |        0.60x |
| `numbers_add`                | numbers-100mb           |        1909.5 |              854.4 |   **2.23x** |        0.44x |
| `numbers_sort`               | numbers-1mb             |          35.8 |               24.4 |  **1.47x**† |        2.09x |
| `numbers_sort`               | numbers-10mb            |         418.1 |              238.8 |  **1.75x**† |        0.97x |
| `numbers_max`                | numbers-1mb             |          10.0 |               11.8 |       0.85x |        2.23x |
| `numbers_max`                | numbers-10mb            |          81.4 |               89.0 |      0.92x† |        1.31x |
| `numbers_max`                | numbers-100mb           |         824.0 |              867.0 |       0.95x |        1.13x |
| `numbers_map_floor`          | numbers-1mb             |          24.0 |               12.5 |  **1.93x**† |        1.96x |
| `numbers_map_floor`          | numbers-10mb            |         219.8 |               99.0 |  **2.22x**† |        1.40x |
| `numbers_map_floor`          | numbers-100mb           |        2219.5 |              962.1 |   **2.31x** |        1.24x |
| `strings_length`             | strings-1mb             |           7.5 |                4.2 |   **1.80x** |        3.27x |
| `strings_length`             | strings-10mb            |          60.0 |               15.8 |  **3.80x**† |        1.76x |
| `strings_length`             | strings-100mb           |         603.8 |              125.0 |  **4.83x**† |        1.29x |
| `strings_upcase`             | strings-1mb             |         251.3 |                8.3 | **30.37x**† |        2.71x |
| `strings_upcase`             | strings-10mb            |        2504.1 |               52.5 |  **47.74x** |        1.34x |
| `strings_upcase`             | strings-100mb           |       24933.8 |              477.9 |  **52.17x** |        1.09x |
| `strings_split`              | strings-1mb             |          11.7 |                8.2 |   **1.44x** |        3.29x |
| `strings_split`              | strings-10mb            |          99.8 |               51.4 |  **1.94x**† |        1.77x |
| `strings_test`               | strings-1mb             |          11.8 |                4.9 |   **2.43x** |        3.24x |
| `strings_test`               | strings-10mb            |          98.4 |               19.9 |  **4.96x**† |        1.77x |
| `wide_keys`                  | wide-1mb                |          26.9 |                7.2 |  **3.76x**† |        0.58x |
| `wide_keys`                  | wide-10mb               |         274.0 |               42.9 |  **6.38x**† |        0.19x |
| `wide_keys`                  | wide-100mb              |        3188.3 |              425.2 |   **7.50x** |        0.13x |
| `wide_to_entries`            | wide-1mb                |          35.1 |               29.2 |  **1.20x**† |        1.03x |
| `wide_to_entries`            | wide-10mb               |         377.7 |              248.4 |  **1.52x**† |        0.88x |
| `wide_map_values`            | wide-1mb                |          95.2 |               32.8 |  **2.90x**† |        1.53x |
| `wide_map_values`            | wide-10mb               |         890.6 |              293.5 |  **3.03x**† |        1.56x |
| `wide_add_values`            | wide-1mb                |          25.8 |               21.6 |  **1.20x**† |        1.66x |
| `wide_add_values`            | wide-10mb               |         237.9 |              170.8 |  **1.39x**† |        1.22x |
| `wide_add_values`            | wide-100mb              |        2756.5 |             1629.3 |   **1.69x** |        1.28x |
| `wide_with_entries`          | wide-1mb                |         211.6 |               82.3 |  **2.57x**† |        1.05x |
| `wide_with_entries`          | wide-10mb               |        2040.2 |              769.3 |   **2.65x** |        0.95x |
| `nested_paths`               | nested-1mb              |           6.0 |                3.8 |   **1.57x** |        2.44x |
| `nested_paths`               | nested-10mb             |          44.1 |               11.5 |  **3.82x**† |        0.90x |
| `nested_recurse`             | nested-1mb              |           5.9 |                4.4 |   **1.35x** |        2.45x |
| `nested_recurse`             | nested-10mb             |          44.2 |               17.3 |  **2.55x**† |        0.90x |
| `comprehensive_paths`        | comprehensive-1mb       |          51.1 |               23.2 |  **2.20x**† |        1.34x |
| `comprehensive_paths`        | comprehensive-10mb      |         479.1 |              186.8 |  **2.57x**† |        0.97x |
| `comprehensive_tostream`     | comprehensive-1mb       |         112.0 |               29.9 |  **3.75x**† |        1.70x |
| `comprehensive_tostream`     | comprehensive-10mb      |        1090.8 |              260.4 |   **4.19x** |        1.43x |
| `comprehensive_numbers`      | comprehensive-1mb       |          41.8 |               21.9 |  **1.91x**† |        1.98x |
| `comprehensive_numbers`      | comprehensive-10mb      |         398.0 |              181.6 |  **2.19x**† |        1.57x |
| `ndjson_identity`            | ndjson-1mb              |          30.7 |                7.3 |  **4.21x**† |        4.04x |
| `ndjson_identity`            | ndjson-10mb             |         287.8 |               44.2 |  **6.51x**† |        4.14x |
| `ndjson_identity`            | ndjson-100mb            |        2907.2 |              403.2 |   **7.21x** |        4.12x |
| `ndjson_field`               | ndjson-1mb              |          12.8 |                9.0 |   **1.42x** |        4.09x |
| `ndjson_field`               | ndjson-10mb             |         108.2 |               59.7 |  **1.81x**† |        4.22x |
| `ndjson_field`               | ndjson-100mb            |        1050.9 |              558.7 |   **1.88x** |        4.26x |
| `ndjson_select`              | ndjson-1mb              |          14.0 |               12.0 |  **1.17x**† |        4.35x |
| `ndjson_select`              | ndjson-10mb             |         121.6 |               88.6 |  **1.37x**† |        4.38x |
| `ndjson_select`              | ndjson-100mb            |        1184.6 |              833.1 |   **1.42x** |        4.41x |
| `ndjson_tsv`                 | ndjson-1mb              |          20.7 |               18.8 |  **1.10x**† |        4.23x |
| `ndjson_tsv`                 | ndjson-10mb             |         186.3 |              151.5 |  **1.23x**† |        4.34x |
| `ndjson_tsv`                 | ndjson-100mb            |        1857.5 |             1439.4 |   **1.29x** |        4.33x |
| `ndjson_slurp`               | ndjson-1mb              |          13.9 |                9.5 |  **1.47x**† |        1.12x |
| `ndjson_slurp`               | ndjson-10mb             |         125.7 |               63.9 |  **1.97x**† |        0.49x |
| `ndjson_inputs_reduce`       | ndjson-1mb              |          13.4 |               13.1 |   **1.02x** |        4.83x |
| `ndjson_inputs_reduce`       | ndjson-10mb             |         112.5 |               93.5 |  **1.20x**† |        8.37x |
| `ndjson_inputs_reduce`       | ndjson-100mb            |        1086.4 |              891.3 |   **1.22x** |       42.61x |
| `ndjson_inputs_first`        | ndjson-1mb              |           1.7 |                3.0 |       0.55x |        4.20x |
| `ndjson_inputs_first`        | ndjson-10mb             |           1.7 |                3.1 |       0.55x |        4.25x |
| `ndjson_inputs_first`        | ndjson-100mb            |           1.7 |                3.2 |       0.54x |        4.23x |
| `ndjson_raw_lines`           | ndjson-1mb              |           7.6 |                4.5 |   **1.71x** |        3.92x |
| `ndjson_raw_lines`           | ndjson-10mb             |          58.9 |               18.3 |  **3.22x**† |        7.19x |
| `ndjson_raw_lines`           | ndjson-100mb            |         566.8 |              155.4 |  **3.65x**† |       39.44x |
| `ndjson_raw_fromjson`        | ndjson-1mb              |          15.5 |               11.3 |  **1.37x**† |        4.04x |
| `ndjson_raw_fromjson`        | ndjson-10mb             |         132.9 |               82.5 |  **1.61x**† |        7.23x |
| `ndjson_raw_slurp`           | ndjson-1mb              |           5.3 |                2.9 |   **1.85x** |        2.43x |
| `ndjson_raw_slurp`           | ndjson-10mb             |          38.7 |                5.0 |  **7.69x**† |        1.00x |
| `ndjson_raw_slurp`           | ndjson-100mb            |         364.7 |               23.6 | **15.46x**† |        0.88x |
| `ndjson_three_files`         | ndjson-1mb              |          35.1 |               19.5 |  **1.80x**† |        4.81x |
| `ndjson_three_files`         | ndjson-10mb             |         327.0 |              164.9 |  **1.98x**† |       15.51x |
| `users_three_files`          | users-1mb               |          17.1 |                5.8 |  **2.94x**† |        1.36x |
| `users_three_files`          | users-10mb              |         162.9 |               34.0 |  **4.79x**† |        0.48x |
| `seq_identity`               | jsonseq-1mb             |          30.7 |                9.3 |  **3.30x**† |        4.12x |
| `seq_identity`               | jsonseq-10mb            |         286.8 |               63.1 |  **4.55x**† |        8.04x |
| `seq_identity`               | jsonseq-100mb           |        2943.8 |              587.0 |   **5.01x** |       47.67x |
| `seq_field`                  | jsonseq-1mb             |          12.9 |               10.8 |   **1.19x** |        4.11x |
| `seq_field`                  | jsonseq-10mb            |         108.8 |               75.7 |  **1.44x**† |        8.05x |
| `seq_field`                  | jsonseq-100mb           |        1055.0 |              714.8 |   **1.48x** |       47.95x |
| `seq_slurp`                  | jsonseq-1mb             |          13.8 |               11.5 |  **1.20x**† |        1.13x |
| `seq_slurp`                  | jsonseq-10mb            |         126.5 |               85.2 |  **1.49x**† |        0.50x |
| `users_seq_output`           | users-1mb               |           3.0 |                5.1 |       0.59x |        4.02x |
| `users_seq_output`           | users-10mb              |          15.8 |               25.5 |      0.62x† |        6.36x |
| `seq_field_bad_first`        | jsonseq-bad-first-1mb   |          12.9 |               13.8 |       0.93x |        4.26x |
| `seq_field_bad_first`        | jsonseq-bad-first-10mb  |         108.5 |              104.1 |  **1.04x**† |        9.38x |
| `seq_field_bad_first`        | jsonseq-bad-first-100mb |        1057.8 |              998.2 |   **1.06x** |       59.49x |
| `seq_field_bad_last`         | jsonseq-bad-last-1mb    |          13.0 |               14.4 |       0.90x |        4.28x |
| `seq_field_bad_last`         | jsonseq-bad-last-10mb   |         109.1 |              107.4 |  **1.02x**† |        9.37x |
| `seq_field_bad_last`         | jsonseq-bad-last-100mb  |        1056.9 |             1036.5 |   **1.02x** |       59.48x |
| `users_slurpfile`            | users-1mb               |           7.2 |                5.2 |   **1.38x** |        1.84x |
| `users_slurpfile`            | users-10mb              |          57.8 |               25.1 |  **2.30x**† |        1.13x |
| `ndjson_slurpfile`           | ndjson-1mb              |          10.3 |                7.3 |   **1.42x** |        1.67x |
| `ndjson_slurpfile`           | ndjson-10mb             |          88.0 |               42.7 |  **2.06x**† |        1.15x |
| `users_rawfile`              | users-1mb               |           3.1 |                2.9 |   **1.08x** |        2.52x |
| `users_rawfile`              | users-10mb              |          16.6 |                3.8 |  **4.41x**† |        0.97x |
| `users_rawfile`              | users-100mb             |         160.9 |               12.2 | **13.16x**† |        0.39x |
| `args_many`                  | none                    |           1.7 |                2.8 |       0.59x |        4.02x |
| `corpus_npm_deps`            | corpus-npm              |           1.7 |                2.7 |       0.62x |        4.24x |
| `corpus_npm_scripts`         | corpus-npm              |           1.7 |                2.7 |       0.63x |        4.20x |
| `corpus_charts_titles`       | corpus-charts           |           1.7 |                2.9 |       0.58x |        3.94x |
| `corpus_graph_weights`       | corpus-graph            |           1.8 |                2.9 |       0.64x |        3.86x |
| `corpus_graph_by_source`     | corpus-graph            |           1.9 |                3.1 |       0.61x |        3.81x |
| `corpus_graph_select`        | corpus-graph            |           1.8 |                2.9 |       0.63x |        3.77x |
| `corpus_openapi_definitions` | corpus-openapi          |           2.2 |                2.9 |       0.74x |        3.50x |
| `corpus_openapi_strings`     | corpus-openapi          |           2.5 |                3.2 |       0.78x |        3.67x |
| `corpus_openapi_refs`        | corpus-openapi          |           2.6 |                3.4 |       0.77x |        3.90x |
| `corpus_openapi_paths`       | corpus-openapi          |           2.6 |                3.2 |       0.79x |        3.45x |
| `corpus_geojson_districts`   | corpus-geojson          |           2.4 |                3.0 |       0.81x |        2.64x |
| `corpus_geojson_types`       | corpus-geojson          |           2.4 |                3.0 |       0.81x |        2.85x |
| `corpus_geojson_points`      | corpus-geojson          |           5.5 |                4.3 |   **1.27x** |        2.98x |
| `corpus_tabular_hours`       | corpus-tabular          |           2.3 |                3.2 |       0.71x |        3.93x |
| `corpus_tabular_csv`         | corpus-tabular          |           2.4 |                3.3 |       0.70x |        3.79x |
| `corpus_tabular_group`       | corpus-tabular          |           2.2 |                3.4 |       0.67x |        4.09x |
| `corpus_pretty_numbers`      | corpus-pretty           |          15.8 |                7.6 |  **2.07x**† |        2.34x |
| `corpus_pretty_paths`        | corpus-pretty           |          17.6 |                8.4 |  **2.08x**† |        1.74x |
| `corpus_pretty_tostream`     | corpus-pretty           |          30.8 |               10.3 |  **2.99x**† |        2.04x |
<!-- /compare:jq-macos-aarch64 -->

The sections below are `bench run jq_bench` and `dev bench jq` results, measured before #5409 made those runners interleave: all jq runs, then all succinctly runs, against the `jq` on `PATH`. That is not the interleaved method of [benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method), so where a figure below disagrees with the block above, the block is the measurement.

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

These summarise the hand-written tables above, which predate #5409's interleaving and are not checked against a committed run. The generated *Harness comparison* blocks and [docs/compare/jq.md](../compare/jq.md) are the current measurement.

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
