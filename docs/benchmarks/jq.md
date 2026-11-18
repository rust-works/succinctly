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

**Run**: `./target/release/succinctly dev compare jq --full --huge` on AMD Ryzen 9 7950X 16-Core Processor (Ubuntu 22.04.5 LTS), succinctly `73f049b2c`, run `20261118_164711_73f049b2c`. jq-1.7.1 is built from source by the harness; the tools are interleaved within each of 21 repetitions, and a row is timed only when the outputs are identical.

**Timed binary**: the harness itself (timed as a copy stripped like the release archive's binary), built with `cli,simd,regex,serde`.

Geomean speedup over jq-1.7.1: **2.74x** across 221 timed rows; jq-1.7.1 is faster on 9 of them. Speedup is jq-1.7.1's time over succinctly jq's, so below 1.00x jq is faster; memory is succinctly jq's peak RSS over jq-1.7.1's. Times are medians, except where marked †: both times and the ratio are then each tool's fastest run, which run-to-run noise moves least on a long row.

| Query                        | Input                   | jq-1.7.1 (ms) | succinctly jq (ms) |     Speedup | Memory ratio |
|------------------------------|-------------------------|---------------|--------------------|-------------|--------------|
| `startup`                    | none                    |           1.2 |                1.1 |   **1.03x** |        3.03x |
| `identity`                   | users-1mb               |          16.4 |                3.2 |  **5.15x**† |        1.03x |
| `identity`                   | users-10mb              |         160.1 |               20.9 |  **7.68x**† |        0.25x |
| `identity`                   | users-100mb             |        1633.7 |              199.6 |   **8.19x** |        0.14x |
| `identity`                   | numbers-1mb             |          21.4 |                3.9 |  **5.50x**† |        0.92x |
| `identity`                   | numbers-10mb            |         212.0 |               26.9 |  **7.88x**† |        0.23x |
| `identity`                   | numbers-100mb           |        2190.3 |              258.5 |   **8.47x** |        0.14x |
| `identity`                   | strings-1mb             |          12.0 |                2.5 |   **4.83x** |        2.63x |
| `identity`                   | strings-10mb            |         111.5 |               12.7 |  **8.75x**† |        1.29x |
| `identity`                   | strings-100mb           |        1169.5 |              122.5 |   **9.54x** |        0.87x |
| `identity`                   | wide-1mb                |          29.9 |                6.1 |  **4.89x**† |        0.63x |
| `identity`                   | wide-10mb               |         310.2 |               45.7 |  **6.78x**† |        0.24x |
| `identity`                   | wide-100mb              |        3684.2 |              398.3 |   **9.25x** |        0.14x |
| `identity`                   | nested-1mb              |          13.3 |                2.2 |   **5.94x** |        1.97x |
| `identity`                   | nested-10mb             |         124.4 |               11.1 | **11.24x**† |        0.89x |
| `identity`                   | nested-100mb            |        1252.3 |              102.5 |  **12.21x** |        0.63x |
| `identity`                   | comprehensive-1mb       |          30.3 |               10.3 |  **2.93x**† |        0.70x |
| `identity`                   | comprehensive-10mb      |         293.7 |               86.3 |  **3.40x**† |        0.26x |
| `identity`                   | comprehensive-100mb     |        3028.2 |              891.3 |   **3.40x** |        0.21x |
| `identity`                   | corpus-charts           |           1.2 |                1.1 |   **1.04x** |        3.06x |
| `identity`                   | corpus-npm              |           1.2 |                1.1 |   **1.05x** |        3.12x |
| `identity`                   | corpus-graph            |           1.4 |                1.1 |   **1.25x** |        3.00x |
| `identity`                   | corpus-openapi          |           1.8 |                1.4 |   **1.27x** |        3.17x |
| `identity`                   | corpus-geojson          |           3.9 |                2.0 |   **1.94x** |        2.61x |
| `identity`                   | corpus-tabular          |           1.7 |                1.3 |   **1.28x** |        2.95x |
| `identity`                   | corpus-pretty           |          10.8 |                3.9 |   **2.75x** |        1.59x |
| `identity_compact`           | users-1mb               |          14.6 |                2.4 |  **6.12x**† |        1.01x |
| `identity_compact`           | users-10mb              |         139.4 |               13.8 | **10.09x**† |        0.25x |
| `identity_compact`           | users-100mb             |        1440.3 |              128.1 |  **11.24x** |        0.14x |
| `identity_compact`           | numbers-1mb             |          19.8 |                2.8 |  **7.03x**† |        0.88x |
| `identity_compact`           | numbers-10mb            |         195.0 |               17.4 | **11.23x**† |        0.23x |
| `identity_compact`           | numbers-100mb           |        2001.3 |              166.5 |  **12.02x** |        0.14x |
| `identity_compact`           | strings-1mb             |          11.7 |                1.9 |   **6.18x** |        2.62x |
| `identity_compact`           | strings-10mb            |         108.1 |                9.2 | **11.82x**† |        1.29x |
| `identity_compact`           | strings-100mb           |        1137.0 |               83.5 |  **13.62x** |        0.87x |
| `identity_compact`           | wide-1mb                |          28.9 |                5.1 |  **5.62x**† |        0.62x |
| `identity_compact`           | wide-10mb               |         296.4 |               36.0 |  **8.23x**† |        0.25x |
| `identity_compact`           | wide-100mb              |        3604.2 |              300.9 |  **11.98x** |        0.14x |
| `identity_compact`           | nested-1mb              |          13.2 |                1.9 |   **7.05x** |        1.95x |
| `identity_compact`           | nested-10mb             |         123.0 |                8.2 | **15.02x**† |        0.88x |
| `identity_compact`           | nested-100mb            |        1242.0 |               72.6 |  **17.11x** |        0.63x |
| `identity_compact`           | comprehensive-1mb       |          24.2 |                9.5 |  **2.55x**† |        0.64x |
| `identity_compact`           | comprehensive-10mb      |         242.5 |               79.3 |  **3.06x**† |        0.19x |
| `identity_compact`           | comprehensive-100mb     |        2498.3 |              825.4 |   **3.03x** |        0.14x |
| `identity_compact`           | corpus-charts           |           1.2 |                1.1 |   **1.02x** |        3.10x |
| `identity_compact`           | corpus-npm              |           1.2 |                1.1 |   **1.08x** |        2.92x |
| `identity_compact`           | corpus-graph            |           1.4 |                1.1 |   **1.25x** |        2.85x |
| `identity_compact`           | corpus-openapi          |           1.7 |                1.4 |   **1.21x** |        3.23x |
| `identity_compact`           | corpus-geojson          |           2.9 |                1.8 |   **1.59x** |        2.49x |
| `identity_compact`           | corpus-tabular          |           1.7 |                1.3 |   **1.26x** |        3.05x |
| `identity_compact`           | corpus-pretty           |           8.8 |                3.7 |   **2.37x** |        1.53x |
| `users_field`                | users-1mb               |           8.8 |                2.4 |   **3.67x** |        1.26x |
| `users_field`                | users-10mb              |          87.5 |               12.3 |  **7.14x**† |        0.29x |
| `users_field`                | users-100mb             |         925.5 |              107.7 |   **8.59x** |        0.16x |
| `users_length`               | users-1mb               |           9.2 |                2.7 |   **3.45x** |        1.31x |
| `users_length`               | users-10mb              |          93.7 |               14.0 |  **6.69x**† |        0.32x |
| `users_length`               | users-100mb             |         986.8 |              127.8 |   **7.72x** |        0.19x |
| `users_names`                | users-1mb               |           9.6 |                4.7 |   **2.05x** |        1.24x |
| `users_names`                | users-10mb              |          97.5 |               34.6 |  **2.82x**† |        0.28x |
| `users_names`                | users-100mb             |        1022.1 |              335.4 |   **3.05x** |        0.16x |
| `users_map_select`           | users-1mb               |          11.0 |                6.2 |   **1.77x** |        1.53x |
| `users_map_select`           | users-10mb              |         130.4 |               47.1 |  **2.77x**† |        0.44x |
| `users_map_select`           | users-100mb             |        1437.4 |              465.0 |   **3.09x** |        0.28x |
| `users_sort_by`              | users-1mb               |          15.0 |                9.3 |  **1.61x**† |        1.42x |
| `users_sort_by`              | users-10mb              |         179.0 |               83.7 |  **2.14x**† |        0.55x |
| `users_group_by`             | users-1mb               |          15.6 |                9.1 |  **1.71x**† |        1.50x |
| `users_group_by`             | users-10mb              |         197.3 |               82.5 |  **2.39x**† |        0.55x |
| `users_unique_by`            | users-1mb               |          15.4 |                9.1 |  **1.69x**† |        1.45x |
| `users_unique_by`            | users-10mb              |         196.1 |               84.8 |  **2.31x**† |        0.56x |
| `users_reduce`               | users-1mb               |          10.3 |                7.6 |   **1.35x** |        1.57x |
| `users_reduce`               | users-10mb              |         104.6 |               62.2 |  **1.68x**† |        0.33x |
| `users_reduce`               | users-100mb             |        1086.3 |              607.3 |   **1.79x** |        0.16x |
| `users_to_entries`           | users-1mb               |           8.7 |                2.5 |   **3.50x** |        1.51x |
| `users_to_entries`           | users-10mb              |          88.8 |               12.4 |  **7.16x**† |        0.33x |
| `users_to_entries`           | users-100mb             |         924.5 |              110.9 |   **8.33x** |        0.16x |
| `users_csv`                  | users-1mb               |          15.2 |               11.1 |  **1.37x**† |        1.32x |
| `users_csv`                  | users-10mb              |         147.2 |               99.7 |  **1.48x**† |        0.29x |
| `users_csv`                  | users-100mb             |        1522.0 |              997.9 |   **1.53x** |        0.15x |
| `users_regex`                | users-1mb               |          14.9 |                6.0 |  **2.48x**† |        1.66x |
| `users_regex`                | users-10mb              |         156.3 |               48.0 |  **3.26x**† |        0.45x |
| `users_ascii_downcase`       | users-1mb               |          26.1 |                6.6 |  **3.96x**† |        1.57x |
| `users_ascii_downcase`       | users-10mb              |         283.7 |               55.2 |  **5.14x**† |        0.39x |
| `users_ascii_downcase`       | users-100mb             |        3115.1 |              546.7 |   **5.70x** |        0.25x |
| `users_first_early_exit`     | users-1mb               |           8.9 |                2.3 |   **3.91x** |        1.36x |
| `users_first_early_exit`     | users-10mb              |          75.3 |               10.2 |  **7.35x**† |        0.30x |
| `users_first_early_exit`     | users-100mb             |         774.6 |               91.3 |   **8.48x** |        0.16x |
| `users_limit_early_exit`     | users-1mb               |           8.8 |                2.3 |   **3.84x** |        1.38x |
| `users_limit_early_exit`     | users-10mb              |          88.8 |               10.5 |  **8.48x**† |        0.30x |
| `users_limit_early_exit`     | users-100mb             |         927.5 |               92.2 |  **10.06x** |        0.16x |
| `users_assign_update`        | users-1mb               |           8.7 |                8.4 |   **1.04x** |        2.27x |
| `users_assign_update`        | users-10mb              |          88.1 |               66.1 |  **1.33x**† |        1.40x |
| `users_del`                  | users-1mb               |           9.2 |                7.9 |   **1.17x** |        2.19x |
| `users_del`                  | users-10mb              |          93.3 |               64.3 |  **1.45x**† |        1.32x |
| `users_paths`                | users-1mb               |          32.7 |               16.0 |  **2.05x**† |        1.73x |
| `users_paths`                | users-10mb              |         347.7 |              150.3 |  **2.31x**† |        1.15x |
| `users_tojson_roundtrip`     | users-1mb               |          21.6 |               12.7 |  **1.70x**† |        1.92x |
| `users_tojson_roundtrip`     | users-10mb              |         236.4 |              131.2 |  **1.80x**† |        1.20x |
| `users_walk`                 | users-1mb               |          76.2 |               16.1 |  **4.74x**† |        2.36x |
| `users_walk`                 | users-10mb              |         761.8 |              151.5 |  **5.03x**† |        1.76x |
| `users_slurp`                | users-1mb               |           8.8 |                2.9 |   **3.06x** |        1.32x |
| `users_slurp`                | users-10mb              |          90.7 |               16.6 |  **5.48x**† |        0.35x |
| `users_stream`               | users-1mb               |          26.1 |               23.1 |  **1.13x**† |        4.35x |
| `users_stream`               | users-10mb              |         252.5 |              219.5 |  **1.15x**† |        4.27x |
| `numbers_add`                | numbers-1mb             |          27.2 |                6.8 |  **3.99x**† |        1.41x |
| `numbers_add`                | numbers-10mb            |         264.2 |               57.0 |  **4.64x**† |        0.39x |
| `numbers_add`                | numbers-100mb           |        2748.7 |              555.5 |   **4.95x** |        0.23x |
| `numbers_sort`               | numbers-1mb             |          44.1 |               26.5 |  **1.66x**† |        1.90x |
| `numbers_sort`               | numbers-10mb            |         881.0 |              316.8 |  **2.78x**† |        1.13x |
| `numbers_max`                | numbers-1mb             |          12.6 |                6.2 |   **2.02x** |        1.42x |
| `numbers_max`                | numbers-10mb            |         117.0 |               50.5 |  **2.32x**† |        0.39x |
| `numbers_max`                | numbers-100mb           |        1246.4 |              492.6 |   **2.53x** |        0.23x |
| `numbers_map_floor`          | numbers-1mb             |          32.5 |               10.3 |  **3.17x**† |        1.52x |
| `numbers_map_floor`          | numbers-10mb            |         317.1 |               90.2 |  **3.52x**† |        0.82x |
| `numbers_map_floor`          | numbers-100mb           |        3297.0 |              887.4 |   **3.72x** |        0.79x |
| `strings_length`             | strings-1mb             |           7.8 |                3.2 |   **2.46x** |        3.28x |
| `strings_length`             | strings-10mb            |          68.4 |               18.9 |  **3.62x**† |        1.44x |
| `strings_length`             | strings-100mb           |         714.2 |              183.9 |   **3.88x** |        1.00x |
| `strings_upcase`             | strings-1mb             |         323.0 |                8.0 | **40.40x**† |        2.89x |
| `strings_upcase`             | strings-10mb            |        3320.5 |               66.9 |  **49.61x** |        1.30x |
| `strings_upcase`             | strings-100mb           |       33707.3 |              648.6 |  **51.97x** |        0.99x |
| `strings_split`              | strings-1mb             |          11.8 |                6.3 |   **1.88x** |        3.35x |
| `strings_split`              | strings-10mb            |         108.0 |               50.8 |  **2.12x**† |        1.48x |
| `strings_test`               | strings-1mb             |          12.1 |                3.8 |   **3.17x** |        3.59x |
| `strings_test`               | strings-10mb            |         110.7 |               25.5 |  **4.35x**† |        1.54x |
| `wide_keys`                  | wide-1mb                |          29.3 |                6.6 |  **4.45x**† |        0.65x |
| `wide_keys`                  | wide-10mb               |         313.6 |               50.2 |  **6.25x**† |        0.21x |
| `wide_keys`                  | wide-100mb              |        3941.7 |              515.6 |   **7.65x** |        0.14x |
| `wide_to_entries`            | wide-1mb                |          59.9 |               48.5 |  **1.23x**† |        1.28x |
| `wide_to_entries`            | wide-10mb               |         664.9 |              441.3 |  **1.51x**† |        1.18x |
| `wide_map_values`            | wide-1mb                |         124.1 |               48.8 |  **2.54x**† |        2.08x |
| `wide_map_values`            | wide-10mb               |        1243.1 |              515.6 |   **2.41x** |        1.93x |
| `wide_add_values`            | wide-1mb                |          36.5 |               13.5 |  **2.71x**† |        0.78x |
| `wide_add_values`            | wide-10mb               |         376.7 |              113.6 |  **3.32x**† |        0.35x |
| `wide_add_values`            | wide-100mb              |        4297.9 |             1066.4 |   **4.03x** |        0.30x |
| `wide_with_entries`          | wide-1mb                |         296.9 |              143.9 |  **2.06x**† |        1.43x |
| `wide_with_entries`          | wide-10mb               |        3025.6 |             1342.0 |   **2.25x** |        1.24x |
| `nested_paths`               | nested-1mb              |           6.1 |                2.5 |   **2.40x** |        2.28x |
| `nested_paths`               | nested-10mb             |          50.8 |               13.4 |  **3.79x**† |        0.98x |
| `nested_recurse`             | nested-1mb              |           6.1 |                3.2 |   **1.90x** |        2.40x |
| `nested_recurse`             | nested-10mb             |          50.4 |               19.7 |  **2.55x**† |        0.97x |
| `comprehensive_paths`        | comprehensive-1mb       |          75.9 |               32.3 |  **2.35x**† |        1.43x |
| `comprehensive_paths`        | comprehensive-10mb      |         798.0 |              308.3 |  **2.59x**† |        1.21x |
| `comprehensive_tostream`     | comprehensive-1mb       |         157.3 |               56.6 |  **2.78x**† |        1.88x |
| `comprehensive_tostream`     | comprehensive-10mb      |        1648.4 |              494.0 |   **3.34x** |        1.72x |
| `comprehensive_numbers`      | comprehensive-1mb       |          56.8 |               27.2 |  **2.09x**† |        1.82x |
| `comprehensive_numbers`      | comprehensive-10mb      |         573.7 |              259.3 |  **2.21x**† |        1.44x |
| `ndjson_identity`            | ndjson-1mb              |          19.8 |                5.8 |  **3.42x**† |        3.20x |
| `ndjson_identity`            | ndjson-10mb             |         184.6 |               41.9 |  **4.41x**† |        3.37x |
| `ndjson_identity`            | ndjson-100mb            |        1830.5 |              394.5 |   **4.64x** |        3.16x |
| `ndjson_field`               | ndjson-1mb              |          11.0 |                8.0 |   **1.38x** |        3.27x |
| `ndjson_field`               | ndjson-10mb             |          95.2 |               60.0 |  **1.59x**† |        3.28x |
| `ndjson_field`               | ndjson-100mb            |         945.6 |              577.5 |   **1.64x** |        3.50x |
| `ndjson_select`              | ndjson-1mb              |          13.2 |               12.5 |   **1.06x** |        4.26x |
| `ndjson_select`              | ndjson-10mb             |         116.7 |              103.0 |  **1.13x**† |        4.01x |
| `ndjson_select`              | ndjson-100mb            |        1146.1 |              995.4 |   **1.15x** |        4.18x |
| `ndjson_tsv`                 | ndjson-1mb              |          20.3 |               19.4 |  **1.05x**† |        3.62x |
| `ndjson_tsv`                 | ndjson-10mb             |         189.8 |              173.0 |  **1.10x**† |        4.00x |
| `ndjson_tsv`                 | ndjson-100mb            |        1895.7 |             1686.0 |   **1.12x** |        3.69x |
| `ndjson_slurp`               | ndjson-1mb              |          16.7 |                8.8 |  **1.90x**† |        1.26x |
| `ndjson_slurp`               | ndjson-10mb             |         185.1 |               73.5 |  **2.52x**† |        0.40x |
| `ndjson_inputs_reduce`       | ndjson-1mb              |          12.3 |               11.3 |   **1.09x** |        4.68x |
| `ndjson_inputs_reduce`       | ndjson-10mb             |         108.9 |               92.7 |  **1.17x**† |        4.83x |
| `ndjson_inputs_reduce`       | ndjson-100mb            |        1076.2 |              882.2 |   **1.22x** |        4.96x |
| `ndjson_inputs_first`        | ndjson-1mb              |           1.2 |                1.3 |       0.94x |        3.79x |
| `ndjson_inputs_first`        | ndjson-10mb             |           1.2 |                1.3 |       0.93x |        3.96x |
| `ndjson_inputs_first`        | ndjson-100mb            |           1.2 |                1.3 |       0.91x |        4.03x |
| `ndjson_raw_lines`           | ndjson-1mb              |           6.4 |                3.4 |   **1.91x** |        3.12x |
| `ndjson_raw_lines`           | ndjson-10mb             |          51.0 |               18.4 |  **2.77x**† |        3.26x |
| `ndjson_raw_lines`           | ndjson-100mb            |         502.8 |              164.2 |  **3.06x**† |        3.43x |
| `ndjson_raw_fromjson`        | ndjson-1mb              |          13.9 |               11.9 |   **1.17x** |        3.52x |
| `ndjson_raw_fromjson`        | ndjson-10mb             |         124.1 |               97.3 |  **1.28x**† |        3.48x |
| `ndjson_raw_slurp`           | ndjson-1mb              |           5.0 |                2.1 |   **2.40x** |        2.48x |
| `ndjson_raw_slurp`           | ndjson-10mb             |          41.7 |                9.5 |  **4.40x**† |        1.40x |
| `ndjson_raw_slurp`           | ndjson-100mb            |         393.1 |               82.8 |  **4.74x**† |        1.51x |
| `ndjson_three_files`         | ndjson-1mb              |          31.3 |               20.3 |  **1.54x**† |        3.82x |
| `ndjson_three_files`         | ndjson-10mb             |         300.2 |              169.4 |  **1.77x**† |        3.30x |
| `users_three_files`          | users-1mb               |          19.2 |                4.4 |  **4.37x**† |        1.44x |
| `users_three_files`          | users-10mb              |         218.4 |               32.1 |  **6.80x**† |        0.30x |
| `seq_identity`               | jsonseq-1mb             |          19.9 |                7.6 |  **2.61x**† |        3.37x |
| `seq_identity`               | jsonseq-10mb            |         185.1 |               59.6 |  **3.10x**† |        3.30x |
| `seq_identity`               | jsonseq-100mb           |        1847.2 |              567.0 |   **3.26x** |        3.18x |
| `seq_field`                  | jsonseq-1mb             |          11.2 |                9.4 |   **1.19x** |        3.26x |
| `seq_field`                  | jsonseq-10mb            |          98.1 |               74.3 |  **1.32x**† |        3.55x |
| `seq_field`                  | jsonseq-100mb           |         962.2 |              717.3 |   **1.34x** |        3.48x |
| `seq_slurp`                  | jsonseq-1mb             |          16.8 |               11.1 |  **1.52x**† |        1.24x |
| `seq_slurp`                  | jsonseq-10mb            |         186.3 |               95.2 |  **1.96x**† |        0.40x |
| `users_seq_output`           | users-1mb               |           2.2 |                1.6 |   **1.43x** |        2.87x |
| `users_seq_output`           | users-10mb              |          12.7 |                5.3 |   **2.39x** |        5.07x |
| `seq_field_bad_first`        | jsonseq-bad-first-1mb   |          11.1 |               11.2 |       0.99x |        3.37x |
| `seq_field_bad_first`        | jsonseq-bad-first-10mb  |          97.0 |               73.8 |  **1.31x**† |        3.48x |
| `seq_field_bad_first`        | jsonseq-bad-first-100mb |         958.3 |              714.9 |   **1.34x** |        3.29x |
| `seq_field_bad_last`         | jsonseq-bad-last-1mb    |          11.2 |               11.3 |       0.99x |        3.28x |
| `seq_field_bad_last`         | jsonseq-bad-last-10mb   |          96.3 |               74.6 |  **1.29x**† |        3.42x |
| `seq_field_bad_last`         | jsonseq-bad-last-100mb  |         961.5 |              720.3 |   **1.33x** |        3.68x |
| `users_slurpfile`            | users-1mb               |           9.0 |                6.7 |   **1.36x** |        1.92x |
| `users_slurpfile`            | users-10mb              |          87.0 |               49.6 |  **1.75x**† |        1.25x |
| `ndjson_slurpfile`           | ndjson-1mb              |          13.4 |               10.5 |   **1.28x** |        1.76x |
| `ndjson_slurpfile`           | ndjson-10mb             |         136.3 |               84.1 |  **1.62x**† |        1.30x |
| `users_rawfile`              | users-1mb               |           3.3 |                1.5 |   **2.30x** |        3.16x |
| `users_rawfile`              | users-10mb              |          22.5 |                4.0 |  **5.61x**† |        1.67x |
| `users_rawfile`              | users-100mb             |         246.5 |               32.5 |  **7.57x**† |        0.60x |
| `args_many`                  | none                    |           1.2 |                1.3 |       0.96x |        3.50x |
| `corpus_npm_deps`            | corpus-npm              |           1.2 |                1.3 |       0.94x |        4.10x |
| `corpus_npm_scripts`         | corpus-npm              |           1.2 |                1.2 |       0.99x |        4.11x |
| `corpus_charts_titles`       | corpus-charts           |           1.2 |                1.2 |       1.00x |        3.57x |
| `corpus_graph_weights`       | corpus-graph            |           1.4 |                1.3 |   **1.07x** |        3.80x |
| `corpus_graph_by_source`     | corpus-graph            |           1.5 |                1.4 |   **1.04x** |        4.04x |
| `corpus_graph_select`        | corpus-graph            |           1.3 |                1.3 |   **1.04x** |        3.78x |
| `corpus_openapi_definitions` | corpus-openapi          |           1.5 |                1.2 |   **1.23x** |        3.38x |
| `corpus_openapi_strings`     | corpus-openapi          |           1.9 |                1.7 |   **1.14x** |        3.83x |
| `corpus_openapi_refs`        | corpus-openapi          |           2.2 |                2.0 |   **1.07x** |        4.85x |
| `corpus_openapi_paths`       | corpus-openapi          |           2.1 |                1.8 |   **1.12x** |        3.63x |
| `corpus_geojson_districts`   | corpus-geojson          |           2.1 |                1.3 |   **1.54x** |        3.02x |
| `corpus_geojson_types`       | corpus-geojson          |           2.1 |                1.5 |   **1.44x** |        3.39x |
| `corpus_geojson_points`      | corpus-geojson          |           6.0 |                2.8 |   **2.13x** |        3.60x |
| `corpus_tabular_hours`       | corpus-tabular          |           1.6 |                1.4 |   **1.11x** |        4.35x |
| `corpus_tabular_csv`         | corpus-tabular          |           1.7 |                1.8 |       0.94x |        3.91x |
| `corpus_tabular_group`       | corpus-tabular          |           1.7 |                1.5 |   **1.08x** |        3.83x |
| `corpus_pretty_numbers`      | corpus-pretty           |          15.5 |                7.1 |  **2.17x**† |        2.36x |
| `corpus_pretty_paths`        | corpus-pretty           |          19.8 |                9.0 |  **2.20x**† |        1.97x |
| `corpus_pretty_tostream`     | corpus-pretty           |          36.7 |               12.9 |  **2.85x**† |        2.34x |
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

**Run**: `./target/release/succinctly dev compare jq --full --huge` on Apple M4 Pro (macOS 26.5.1), succinctly `73f049b2c`, run `20261118_162852_73f049b2c`. jq-1.7.1 is built from source by the harness; the tools are interleaved within each of 21 repetitions, and a row is timed only when the outputs are identical.

**Timed binary**: the harness itself (timed as a copy stripped like the release archive's binary), built with `cli,simd,regex,serde`.

Geomean speedup over jq-1.7.1: **2.56x** across 221 timed rows; jq-1.7.1 is faster on 30 of them. Speedup is jq-1.7.1's time over succinctly jq's, so below 1.00x jq is faster; memory is succinctly jq's peak RSS over jq-1.7.1's. Times are medians, except where marked †: both times and the ratio are then each tool's fastest run, which run-to-run noise moves least on a long row.

| Query                        | Input                   | jq-1.7.1 (ms) | succinctly jq (ms) |     Speedup | Memory ratio |
|------------------------------|-------------------------|---------------|--------------------|-------------|--------------|
| `startup`                    | none                    |           1.7 |                2.7 |       0.65x |        3.35x |
| `identity`                   | users-1mb               |          24.9 |                4.7 |  **5.27x**† |        1.00x |
| `identity`                   | users-10mb              |         239.1 |               23.3 | **10.28x**† |        0.26x |
| `identity`                   | users-100mb             |        2446.1 |              204.8 |  **11.94x** |        0.14x |
| `identity`                   | numbers-1mb             |          22.4 |                5.6 |  **3.99x**† |        0.85x |
| `identity`                   | numbers-10mb            |         208.7 |               34.9 |  **5.98x**† |        0.21x |
| `identity`                   | numbers-100mb           |        2142.9 |              325.2 |   **6.59x** |        0.12x |
| `identity`                   | strings-1mb             |          21.3 |                3.1 |  **6.79x**† |        2.70x |
| `identity`                   | strings-10mb            |         203.8 |               10.1 | **20.12x**† |        1.21x |
| `identity`                   | strings-100mb           |        2106.2 |               73.4 |  **28.70x** |        0.72x |
| `identity`                   | wide-1mb                |          35.6 |                6.4 |  **5.58x**† |        0.49x |
| `identity`                   | wide-10mb               |         342.4 |               39.0 |  **8.78x**† |        0.21x |
| `identity`                   | wide-100mb              |        3870.6 |              362.3 |  **10.68x** |        0.12x |
| `identity`                   | nested-1mb              |          22.9 |                3.1 |  **7.31x**† |        2.07x |
| `identity`                   | nested-10mb             |         219.5 |                9.6 | **22.94x**† |        0.84x |
| `identity`                   | nested-100mb            |        2269.1 |               70.3 |  **32.28x** |        0.63x |
| `identity`                   | comprehensive-1mb       |          41.8 |               11.0 |  **3.79x**† |        0.73x |
| `identity`                   | comprehensive-10mb      |         398.6 |               82.0 |  **4.86x**† |        0.30x |
| `identity`                   | comprehensive-100mb     |        4008.1 |              814.6 |   **4.92x** |        0.23x |
| `identity`                   | corpus-charts           |           1.7 |                2.4 |       0.69x |        3.38x |
| `identity`                   | corpus-npm              |           1.7 |                2.4 |       0.71x |        3.38x |
| `identity`                   | corpus-graph            |           2.3 |                2.5 |       0.93x |        3.02x |
| `identity`                   | corpus-openapi          |           3.1 |                2.7 |   **1.12x** |        3.05x |
| `identity`                   | corpus-geojson          |           7.0 |                3.2 |   **2.21x** |        2.24x |
| `identity`                   | corpus-tabular          |           3.0 |                2.6 |   **1.15x** |        3.15x |
| `identity`                   | corpus-pretty           |          16.3 |                5.4 |  **3.02x**† |        1.52x |
| `identity_compact`           | users-1mb               |          18.9 |                3.6 |  **5.28x**† |        0.98x |
| `identity_compact`           | users-10mb              |         182.1 |               14.5 | **12.54x**† |        0.25x |
| `identity_compact`           | users-100mb             |        1885.8 |              121.7 |  **15.49x** |        0.14x |
| `identity_compact`           | numbers-1mb             |          17.0 |                4.6 |  **3.70x**† |        0.84x |
| `identity_compact`           | numbers-10mb            |         157.0 |               24.5 |  **6.41x**† |        0.21x |
| `identity_compact`           | numbers-100mb           |        1590.1 |              219.4 |   **7.25x** |        0.12x |
| `identity_compact`           | strings-1mb             |          20.6 |                2.8 |  **7.42x**† |        2.64x |
| `identity_compact`           | strings-10mb            |         194.9 |                6.8 | **28.46x**† |        1.21x |
| `identity_compact`           | strings-100mb           |        2000.9 |               44.2 |  **45.31x** |        0.72x |
| `identity_compact`           | wide-1mb                |          32.0 |                5.3 |  **6.00x**† |        0.49x |
| `identity_compact`           | wide-10mb               |         311.8 |               29.2 | **10.69x**† |        0.21x |
| `identity_compact`           | wide-100mb              |        3590.9 |              265.1 |  **13.55x** |        0.12x |
| `identity_compact`           | nested-1mb              |          22.8 |                2.7 |  **8.30x**† |        2.06x |
| `identity_compact`           | nested-10mb             |         215.3 |                6.5 | **32.91x**† |        0.84x |
| `identity_compact`           | nested-100mb            |        2255.8 |               42.5 |  **53.09x** |        0.63x |
| `identity_compact`           | comprehensive-1mb       |          25.5 |               10.4 |  **2.45x**† |        0.66x |
| `identity_compact`           | comprehensive-10mb      |         243.3 |               76.1 |  **3.20x**† |        0.22x |
| `identity_compact`           | comprehensive-100mb     |        2430.5 |              746.9 |   **3.25x** |        0.16x |
| `identity_compact`           | corpus-charts           |           1.6 |                2.4 |       0.68x |        3.38x |
| `identity_compact`           | corpus-npm              |           1.7 |                2.4 |       0.71x |        3.35x |
| `identity_compact`           | corpus-graph            |           2.0 |                2.4 |       0.83x |        3.01x |
| `identity_compact`           | corpus-openapi          |           2.7 |                2.7 |       1.00x |        3.05x |
| `identity_compact`           | corpus-geojson          |           3.4 |                3.1 |   **1.09x** |        2.21x |
| `identity_compact`           | corpus-tabular          |           2.8 |                2.6 |   **1.08x** |        3.16x |
| `identity_compact`           | corpus-pretty           |          11.2 |                5.4 |   **2.07x** |        1.46x |
| `users_field`                | users-1mb               |           7.0 |                3.5 |   **2.00x** |        1.10x |
| `users_field`                | users-10mb              |          57.1 |               12.5 |  **4.58x**† |        0.28x |
| `users_field`                | users-100mb             |         567.1 |               98.3 |  **5.77x**† |        0.15x |
| `users_length`               | users-1mb               |           7.3 |                3.6 |   **2.01x** |        1.15x |
| `users_length`               | users-10mb              |          60.4 |               13.2 |  **4.57x**† |        0.36x |
| `users_length`               | users-100mb             |         640.2 |              103.3 |  **6.19x**† |        0.19x |
| `users_names`                | users-1mb               |           7.9 |                5.7 |   **1.37x** |        1.09x |
| `users_names`                | users-10mb              |          66.5 |               34.2 |  **1.94x**† |        0.28x |
| `users_names`                | users-100mb             |         655.0 |              327.7 |   **2.00x** |        0.15x |
| `users_map_select`           | users-1mb               |           8.4 |                6.9 |   **1.22x** |        1.28x |
| `users_map_select`           | users-10mb              |          71.8 |               43.3 |  **1.66x**† |        0.46x |
| `users_map_select`           | users-100mb             |         721.3 |              406.9 |   **1.77x** |        0.28x |
| `users_sort_by`              | users-1mb               |          11.7 |                9.0 |   **1.29x** |        1.32x |
| `users_sort_by`              | users-10mb              |         116.4 |               65.2 |  **1.79x**† |        0.60x |
| `users_group_by`             | users-1mb               |          12.2 |                9.0 |   **1.34x** |        1.27x |
| `users_group_by`             | users-10mb              |         128.9 |               63.6 |  **2.03x**† |        0.54x |
| `users_unique_by`            | users-1mb               |          12.1 |                9.0 |   **1.35x** |        1.28x |
| `users_unique_by`            | users-10mb              |         130.7 |               64.4 |  **2.03x**† |        0.58x |
| `users_reduce`               | users-1mb               |           8.1 |                8.5 |       0.95x |        1.22x |
| `users_reduce`               | users-10mb              |          69.0 |               57.6 |  **1.20x**† |        0.30x |
| `users_reduce`               | users-100mb             |         686.2 |              542.3 |   **1.27x** |        0.15x |
| `users_to_entries`           | users-1mb               |           7.0 |                3.6 |   **1.95x** |        1.22x |
| `users_to_entries`           | users-10mb              |          56.9 |               12.6 |  **4.52x**† |        0.30x |
| `users_to_entries`           | users-100mb             |         566.7 |               98.4 |  **5.76x**† |        0.15x |
| `users_csv`                  | users-1mb               |          13.2 |               13.5 |       0.98x |        1.12x |
| `users_csv`                  | users-10mb              |         119.7 |              105.0 |  **1.14x**† |        0.28x |
| `users_csv`                  | users-100mb             |        1200.3 |             1014.2 |   **1.18x** |        0.15x |
| `users_regex`                | users-1mb               |          13.0 |                6.8 |   **1.92x** |        1.31x |
| `users_regex`                | users-10mb              |         114.2 |               42.4 |  **2.70x**† |        0.46x |
| `users_ascii_downcase`       | users-1mb               |          21.0 |                7.0 |  **3.01x**† |        1.22x |
| `users_ascii_downcase`       | users-10mb              |         212.6 |               47.4 |  **4.49x**† |        0.40x |
| `users_ascii_downcase`       | users-100mb             |        2183.4 |              454.4 |   **4.81x** |        0.23x |
| `users_first_early_exit`     | users-1mb               |           6.9 |                3.3 |   **2.08x** |        1.13x |
| `users_first_early_exit`     | users-10mb              |          57.1 |               10.9 |  **5.25x**† |        0.29x |
| `users_first_early_exit`     | users-100mb             |         568.7 |               83.0 |  **6.85x**† |        0.15x |
| `users_limit_early_exit`     | users-1mb               |           7.0 |                3.4 |   **2.06x** |        1.15x |
| `users_limit_early_exit`     | users-10mb              |          56.9 |               11.0 |  **5.19x**† |        0.29x |
| `users_limit_early_exit`     | users-100mb             |         563.2 |               83.0 |  **6.79x**† |        0.15x |
| `users_assign_update`        | users-1mb               |           7.1 |                6.5 |   **1.08x** |        1.82x |
| `users_assign_update`        | users-10mb              |          57.1 |               42.5 |  **1.34x**† |        1.20x |
| `users_del`                  | users-1mb               |           7.2 |                6.5 |   **1.11x** |        1.77x |
| `users_del`                  | users-10mb              |          59.9 |               41.8 |  **1.44x**† |        1.15x |
| `users_paths`                | users-1mb               |          22.9 |               11.8 |  **1.95x**† |        1.36x |
| `users_paths`                | users-10mb              |         218.9 |               93.7 |  **2.34x**† |        0.93x |
| `users_tojson_roundtrip`     | users-1mb               |          17.2 |                9.5 |  **1.81x**† |        1.46x |
| `users_tojson_roundtrip`     | users-10mb              |         165.2 |               73.9 |  **2.24x**† |        0.96x |
| `users_walk`                 | users-1mb               |          61.7 |               11.8 |  **5.24x**† |        1.85x |
| `users_walk`                 | users-10mb              |         596.6 |               92.9 |  **6.42x**† |        1.41x |
| `users_slurp`                | users-1mb               |           7.0 |                3.6 |   **1.97x** |        1.20x |
| `users_slurp`                | users-10mb              |          56.9 |               13.0 |  **4.37x**† |        0.38x |
| `users_stream`               | users-1mb               |          23.2 |               21.0 |  **1.10x**† |        3.95x |
| `users_stream`               | users-10mb              |         213.7 |              179.8 |  **1.19x**† |        3.90x |
| `numbers_add`                | numbers-1mb             |          20.3 |                8.1 |  **2.51x**† |        1.10x |
| `numbers_add`                | numbers-10mb            |         187.3 |               59.9 |  **3.12x**† |        0.28x |
| `numbers_add`                | numbers-100mb           |        1873.9 |              578.3 |   **3.24x** |        0.16x |
| `numbers_sort`               | numbers-1mb             |          35.0 |               22.9 |  **1.53x**† |        2.01x |
| `numbers_sort`               | numbers-10mb            |         410.0 |              232.7 |  **1.76x**† |        0.96x |
| `numbers_max`                | numbers-1mb             |           9.7 |                8.0 |   **1.22x** |        1.09x |
| `numbers_max`                | numbers-10mb            |          80.2 |               56.4 |  **1.42x**† |        0.28x |
| `numbers_max`                | numbers-100mb           |         815.2 |              542.6 |   **1.50x** |        0.16x |
| `numbers_map_floor`          | numbers-1mb             |          23.4 |               10.0 |  **2.35x**† |        1.58x |
| `numbers_map_floor`          | numbers-10mb            |         218.5 |               74.3 |  **2.94x**† |        0.77x |
| `numbers_map_floor`          | numbers-100mb           |        2187.2 |              739.9 |   **2.96x** |        0.63x |
| `strings_length`             | strings-1mb             |           7.3 |                3.8 |   **1.94x** |        2.95x |
| `strings_length`             | strings-10mb            |          58.5 |               14.4 |  **4.05x**† |        1.52x |
| `strings_length`             | strings-100mb           |         590.3 |              117.3 |  **5.03x**† |        0.95x |
| `strings_upcase`             | strings-1mb             |         247.1 |                7.5 | **32.75x**† |        2.46x |
| `strings_upcase`             | strings-10mb            |        2471.8 |               51.7 |  **47.79x** |        1.24x |
| `strings_upcase`             | strings-100mb           |       24609.7 |              487.8 |  **50.45x** |        0.88x |
| `strings_split`              | strings-1mb             |          11.4 |                7.6 |   **1.50x** |        3.00x |
| `strings_split`              | strings-10mb            |          98.6 |               49.9 |  **1.97x**† |        1.54x |
| `strings_test`               | strings-1mb             |          11.5 |                4.3 |   **2.70x** |        2.93x |
| `strings_test`               | strings-10mb            |          97.1 |               19.1 |  **5.07x**† |        1.54x |
| `wide_keys`                  | wide-1mb                |          26.4 |                6.8 |  **3.87x**† |        0.53x |
| `wide_keys`                  | wide-10mb               |         269.1 |               42.4 |  **6.34x**† |        0.18x |
| `wide_keys`                  | wide-100mb              |        3127.8 |              421.4 |   **7.42x** |        0.13x |
| `wide_to_entries`            | wide-1mb                |          34.7 |               28.8 |  **1.21x**† |        1.01x |
| `wide_to_entries`            | wide-10mb               |         370.9 |              247.6 |  **1.50x**† |        0.88x |
| `wide_map_values`            | wide-1mb                |          93.4 |               32.4 |  **2.88x**† |        1.53x |
| `wide_map_values`            | wide-10mb               |         884.7 |              290.3 |  **3.05x**† |        1.56x |
| `wide_add_values`            | wide-1mb                |          25.3 |               11.4 |  **2.22x**† |        0.76x |
| `wide_add_values`            | wide-10mb               |         232.5 |               85.5 |  **2.72x**† |        0.43x |
| `wide_add_values`            | wide-100mb              |        2704.7 |              815.8 |   **3.32x** |        0.30x |
| `wide_with_entries`          | wide-1mb                |         207.8 |               81.4 |  **2.55x**† |        1.04x |
| `wide_with_entries`          | wide-10mb               |        2020.4 |              767.7 |   **2.63x** |        0.95x |
| `nested_paths`               | nested-1mb              |           5.8 |                3.5 |   **1.65x** |        2.24x |
| `nested_paths`               | nested-10mb             |          43.2 |               11.1 |  **3.90x**† |        0.86x |
| `nested_recurse`             | nested-1mb              |           5.7 |                4.1 |   **1.39x** |        2.25x |
| `nested_recurse`             | nested-10mb             |          43.2 |               16.6 |  **2.60x**† |        0.86x |
| `comprehensive_paths`        | comprehensive-1mb       |          50.4 |               22.6 |  **2.23x**† |        1.31x |
| `comprehensive_paths`        | comprehensive-10mb      |         476.0 |              182.6 |  **2.61x**† |        0.97x |
| `comprehensive_tostream`     | comprehensive-1mb       |         111.3 |               29.2 |  **3.81x**† |        1.68x |
| `comprehensive_tostream`     | comprehensive-10mb      |        1083.3 |              254.8 |   **4.25x** |        1.43x |
| `comprehensive_numbers`      | comprehensive-1mb       |          41.5 |               21.4 |  **1.94x**† |        1.93x |
| `comprehensive_numbers`      | comprehensive-10mb      |         395.6 |              180.1 |  **2.20x**† |        1.57x |
| `ndjson_identity`            | ndjson-1mb              |          30.2 |                6.9 |  **4.37x**† |        3.68x |
| `ndjson_identity`            | ndjson-10mb             |         282.6 |               42.8 |  **6.60x**† |        3.82x |
| `ndjson_identity`            | ndjson-100mb            |        2825.0 |              399.0 |   **7.08x** |        3.85x |
| `ndjson_field`               | ndjson-1mb              |          12.7 |                9.0 |   **1.41x** |        3.76x |
| `ndjson_field`               | ndjson-10mb             |         106.0 |               59.7 |  **1.77x**† |        3.91x |
| `ndjson_field`               | ndjson-100mb            |        1031.6 |              568.5 |   **1.81x** |        3.92x |
| `ndjson_select`              | ndjson-1mb              |          13.6 |               11.4 |  **1.20x**† |        4.03x |
| `ndjson_select`              | ndjson-10mb             |         118.7 |               87.0 |  **1.36x**† |        4.13x |
| `ndjson_select`              | ndjson-100mb            |        1166.4 |              826.7 |   **1.41x** |        4.12x |
| `ndjson_tsv`                 | ndjson-1mb              |          20.2 |               18.1 |  **1.12x**† |        3.89x |
| `ndjson_tsv`                 | ndjson-10mb             |         182.6 |              148.6 |  **1.23x**† |        4.02x |
| `ndjson_tsv`                 | ndjson-100mb            |        1789.9 |             1440.0 |   **1.24x** |        4.04x |
| `ndjson_slurp`               | ndjson-1mb              |          13.7 |                9.1 |   **1.50x** |        1.04x |
| `ndjson_slurp`               | ndjson-10mb             |         122.7 |               62.4 |  **1.97x**† |        0.48x |
| `ndjson_inputs_reduce`       | ndjson-1mb              |          13.2 |               12.6 |   **1.05x** |        4.70x |
| `ndjson_inputs_reduce`       | ndjson-10mb             |         109.7 |               91.4 |  **1.20x**† |        5.30x |
| `ndjson_inputs_reduce`       | ndjson-100mb            |        1067.5 |              875.2 |   **1.22x** |        5.28x |
| `ndjson_inputs_first`        | ndjson-1mb              |           1.7 |                2.5 |       0.65x |        3.73x |
| `ndjson_inputs_first`        | ndjson-10mb             |           1.7 |                2.6 |       0.63x |        3.76x |
| `ndjson_inputs_first`        | ndjson-100mb            |           1.6 |                2.6 |       0.64x |        3.73x |
| `ndjson_raw_lines`           | ndjson-1mb              |           7.7 |                4.3 |   **1.78x** |        3.63x |
| `ndjson_raw_lines`           | ndjson-10mb             |          57.5 |               17.8 |  **3.23x**† |        3.57x |
| `ndjson_raw_lines`           | ndjson-100mb            |         553.1 |              150.8 |  **3.67x**† |        3.55x |
| `ndjson_raw_fromjson`        | ndjson-1mb              |          15.1 |               11.4 |  **1.33x**† |        3.71x |
| `ndjson_raw_fromjson`        | ndjson-10mb             |         130.4 |               83.1 |  **1.57x**† |        3.65x |
| `ndjson_raw_slurp`           | ndjson-1mb              |           5.6 |                2.8 |   **2.03x** |        2.23x |
| `ndjson_raw_slurp`           | ndjson-10mb             |          37.4 |                4.8 |  **7.72x**† |        0.97x |
| `ndjson_raw_slurp`           | ndjson-100mb            |         359.6 |               22.8 | **15.77x**† |        0.88x |
| `ndjson_three_files`         | ndjson-1mb              |          34.7 |               19.4 |  **1.79x**† |        4.47x |
| `ndjson_three_files`         | ndjson-10mb             |         320.8 |              161.4 |  **1.99x**† |        3.84x |
| `users_three_files`          | users-1mb               |          16.6 |                5.4 |  **3.06x**† |        1.24x |
| `users_three_files`          | users-10mb              |         160.8 |               34.2 |  **4.71x**† |        0.51x |
| `seq_identity`               | jsonseq-1mb             |          29.8 |                8.7 |  **3.44x**† |        3.76x |
| `seq_identity`               | jsonseq-10mb            |         282.0 |               60.8 |  **4.64x**† |        3.88x |
| `seq_identity`               | jsonseq-100mb           |        2916.5 |              581.2 |   **5.02x** |        3.89x |
| `seq_field`                  | jsonseq-1mb             |          12.8 |               10.5 |   **1.21x** |        3.75x |
| `seq_field`                  | jsonseq-10mb            |         107.0 |               74.4 |  **1.44x**† |        3.91x |
| `seq_field`                  | jsonseq-100mb           |        1043.5 |              716.4 |   **1.46x** |        3.93x |
| `seq_slurp`                  | jsonseq-1mb             |          14.0 |               11.3 |   **1.24x** |        1.05x |
| `seq_slurp`                  | jsonseq-10mb            |         123.6 |               83.7 |  **1.48x**† |        0.49x |
| `users_seq_output`           | users-1mb               |           2.9 |                2.6 |   **1.11x** |        3.60x |
| `users_seq_output`           | users-10mb              |          15.3 |                3.9 |  **3.93x**† |        6.05x |
| `seq_field_bad_first`        | jsonseq-bad-first-1mb   |          12.6 |               12.6 |       1.00x |        3.94x |
| `seq_field_bad_first`        | jsonseq-bad-first-10mb  |         106.6 |               73.7 |  **1.45x**† |        3.96x |
| `seq_field_bad_first`        | jsonseq-bad-first-100mb |        1034.5 |              707.5 |   **1.46x** |        3.98x |
| `seq_field_bad_last`         | jsonseq-bad-last-1mb    |          12.7 |               12.9 |       0.98x |        3.94x |
| `seq_field_bad_last`         | jsonseq-bad-last-10mb   |         106.7 |               73.8 |  **1.45x**† |        4.00x |
| `seq_field_bad_last`         | jsonseq-bad-last-100mb  |        1035.7 |              711.1 |   **1.46x** |        4.34x |
| `users_slurpfile`            | users-1mb               |           7.0 |                4.8 |   **1.46x** |        1.75x |
| `users_slurpfile`            | users-10mb              |          56.7 |               23.7 |  **2.39x**† |        1.12x |
| `ndjson_slurpfile`           | ndjson-1mb              |          10.0 |                6.9 |   **1.45x** |        1.59x |
| `ndjson_slurpfile`           | ndjson-10mb             |          86.5 |               42.2 |  **2.05x**† |        1.14x |
| `users_rawfile`              | users-1mb               |           3.0 |                2.6 |   **1.18x** |        2.32x |
| `users_rawfile`              | users-10mb              |          16.2 |                3.6 |  **4.50x**† |        0.92x |
| `users_rawfile`              | users-100mb             |         156.4 |               11.7 | **13.39x**† |        0.39x |
| `args_many`                  | none                    |           1.6 |                2.6 |       0.63x |        3.72x |
| `corpus_npm_deps`            | corpus-npm              |           1.7 |                2.5 |       0.65x |        3.90x |
| `corpus_npm_scripts`         | corpus-npm              |           1.7 |                2.5 |       0.66x |        3.80x |
| `corpus_charts_titles`       | corpus-charts           |           1.6 |                2.4 |       0.67x |        3.52x |
| `corpus_graph_weights`       | corpus-graph            |           1.8 |                2.5 |       0.71x |        3.30x |
| `corpus_graph_by_source`     | corpus-graph            |           1.8 |                2.7 |       0.69x |        3.41x |
| `corpus_graph_select`        | corpus-graph            |           1.8 |                2.5 |       0.70x |        3.46x |
| `corpus_openapi_definitions` | corpus-openapi          |           2.1 |                2.6 |       0.82x |        3.16x |
| `corpus_openapi_strings`     | corpus-openapi          |           2.4 |                2.9 |       0.83x |        3.37x |
| `corpus_openapi_refs`        | corpus-openapi          |           2.6 |                3.2 |       0.82x |        3.70x |
| `corpus_openapi_paths`       | corpus-openapi          |           2.5 |                3.0 |       0.85x |        3.15x |
| `corpus_geojson_districts`   | corpus-geojson          |           2.4 |                2.7 |       0.86x |        2.41x |
| `corpus_geojson_types`       | corpus-geojson          |           2.4 |                2.8 |       0.85x |        2.60x |
| `corpus_geojson_points`      | corpus-geojson          |           5.3 |                4.0 |   **1.31x** |        2.78x |
| `corpus_tabular_hours`       | corpus-tabular          |           2.2 |                2.8 |       0.78x |        3.63x |
| `corpus_tabular_csv`         | corpus-tabular          |           2.3 |                3.1 |       0.74x |        3.45x |
| `corpus_tabular_group`       | corpus-tabular          |           2.2 |                2.8 |       0.79x |        3.46x |
| `corpus_pretty_numbers`      | corpus-pretty           |          15.4 |                7.3 |  **2.11x**† |        2.21x |
| `corpus_pretty_paths`        | corpus-pretty           |          17.3 |                8.1 |  **2.14x**† |        1.66x |
| `corpus_pretty_tostream`     | corpus-pretty           |          30.4 |                9.9 |  **3.06x**† |        1.97x |
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
