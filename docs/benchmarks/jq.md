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
# or time a build without the harness (`bench-runner`) instead, passed as the subject (#5551)
cargo build --release --features cli,simd,regex,serde --target-dir target/subject
./target/release/succinctly dev compare jq --quick --build-jq --write-docs \
  --subject target/subject/release/succinctly --subject-features cli,simd,regex,serde
```

Each block names the binary it timed. All three committed blocks timed the harness's own build (`cli,simd,regex,serde` plus `bench-runner`), and none measured the binary of a release archive, which `release.yml` builds with `cli` alone: [Which binary a run measures](../guides/reproducing-the-jq-comparison.md#which-binary-a-run-measures) says what differs and how to measure another build. `--write-docs` refuses, before measuring anything, on a machine whose CPU is not the one named on the `**CPU**:` line of the section holding its platform's block: the M1 Max and the M4 Pro are both `macos-aarch64`.

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

**Run**: `./target/release/succinctly dev compare jq --full --huge` on AMD Ryzen 9 7950X 16-Core Processor (Ubuntu 22.04.5 LTS), succinctly `955a984`, run `20261125_131838_955a984`. jq-1.7.1 is built from source by the harness; the tools are interleaved within each of 21 repetitions, and a row is timed only when the outputs are identical.

**Timed binary**: the harness itself (timed as a copy stripped like the release archive's binary), built with `cli,simd,regex,serde` plus `bench-runner`. The release archives are built with `cli` alone, and no figure here is of that build ([reproducing-the-jq-comparison.md](../guides/reproducing-the-jq-comparison.md#which-binary-a-run-measures)).

Geomean speedup over jq-1.7.1: **3.18x** across 223 timed rows; jq-1.7.1 is faster on 0 of them. Speedup is jq-1.7.1's time over succinctly jq's, so below 1.00x jq is faster; memory is succinctly jq's peak RSS over jq-1.7.1's. Times are medians, except where marked †: both times and the ratio are then each tool's fastest run, which run-to-run noise moves least on a long row.

| Query                        | Input                   | jq-1.7.1 (ms) | succinctly jq (ms) |     Speedup | Memory ratio |
|------------------------------|-------------------------|---------------|--------------------|-------------|--------------|
| `startup`                    | none                    |           1.2 |                1.0 |   **1.23x** |        1.36x |
| `identity`                   | users-1mb               |          16.7 |                3.1 |  **5.32x**† |        0.57x |
| `identity`                   | users-10mb              |         159.7 |               20.7 |  **7.70x**† |        0.18x |
| `identity`                   | users-100mb             |        1631.8 |              205.3 |   **7.95x** |        0.13x |
| `identity`                   | numbers-1mb             |          21.8 |                3.7 |  **5.83x**† |        0.50x |
| `identity`                   | numbers-10mb            |         213.5 |               26.7 |  **7.99x**† |        0.18x |
| `identity`                   | numbers-100mb           |        2191.9 |              261.3 |   **8.39x** |        0.14x |
| `identity`                   | strings-1mb             |          12.2 |                2.2 |   **5.51x** |        1.50x |
| `identity`                   | strings-10mb            |         111.5 |               12.5 |  **8.90x**† |        0.98x |
| `identity`                   | strings-100mb           |        1174.3 |              118.7 |   **9.90x** |        0.84x |
| `identity`                   | wide-1mb                |          30.0 |                6.0 |  **4.97x**† |        0.39x |
| `identity`                   | wide-10mb               |         314.8 |               46.2 |  **6.81x**† |        0.21x |
| `identity`                   | wide-100mb              |        3734.0 |              403.3 |   **9.26x** |        0.14x |
| `identity`                   | nested-1mb              |          13.4 |                2.2 |   **6.13x** |        1.10x |
| `identity`                   | nested-10mb             |         124.0 |               10.4 | **11.97x**† |        0.69x |
| `identity`                   | nested-100mb            |        1249.6 |               95.7 |  **13.06x** |        0.61x |
| `identity`                   | comprehensive-1mb       |          30.2 |               10.1 |  **3.00x**† |        0.44x |
| `identity`                   | comprehensive-10mb      |         295.6 |               85.8 |  **3.45x**† |        0.23x |
| `identity`                   | comprehensive-100mb     |        3075.2 |              882.0 |   **3.49x** |        0.20x |
| `identity`                   | corpus-charts           |           1.2 |                1.0 |   **1.19x** |        1.43x |
| `identity`                   | corpus-npm              |           1.2 |                1.0 |   **1.24x** |        1.41x |
| `identity`                   | corpus-graph            |           1.5 |                1.1 |   **1.41x** |        1.43x |
| `identity`                   | corpus-openapi          |           1.9 |                1.3 |   **1.41x** |        1.43x |
| `identity`                   | corpus-geojson          |           3.9 |                1.8 |   **2.15x** |        1.25x |
| `identity`                   | corpus-tabular          |           1.8 |                1.3 |   **1.42x** |        1.42x |
| `identity`                   | corpus-pretty           |          10.9 |                3.8 |   **2.85x** |        0.88x |
| `identity_compact`           | users-1mb               |          14.6 |                2.3 |  **6.30x**† |        0.54x |
| `identity_compact`           | users-10mb              |         138.6 |               13.6 | **10.17x**† |        0.18x |
| `identity_compact`           | users-100mb             |        1438.9 |              130.8 |  **11.00x** |        0.13x |
| `identity_compact`           | numbers-1mb             |          20.2 |                2.7 |  **7.47x**† |        0.47x |
| `identity_compact`           | numbers-10mb            |         195.0 |               16.9 | **11.54x**† |        0.18x |
| `identity_compact`           | numbers-100mb           |        2035.3 |              164.1 |  **12.40x** |        0.14x |
| `identity_compact`           | strings-1mb             |          11.9 |                1.8 |   **6.49x** |        1.47x |
| `identity_compact`           | strings-10mb            |         108.9 |                8.8 | **12.40x**† |        0.98x |
| `identity_compact`           | strings-100mb           |        1143.1 |               80.5 |  **14.21x** |        0.83x |
| `identity_compact`           | wide-1mb                |          29.0 |                4.8 |  **5.99x**† |        0.39x |
| `identity_compact`           | wide-10mb               |         305.0 |               36.1 |  **8.44x**† |        0.22x |
| `identity_compact`           | wide-100mb              |        3663.3 |              299.2 |  **12.24x** |        0.14x |
| `identity_compact`           | nested-1mb              |          13.4 |                1.8 |   **7.52x** |        1.08x |
| `identity_compact`           | nested-10mb             |         123.1 |                7.8 | **15.83x**† |        0.70x |
| `identity_compact`           | nested-100mb            |        1247.9 |               68.7 |  **18.16x** |        0.61x |
| `identity_compact`           | comprehensive-1mb       |          24.7 |                9.4 |  **2.63x**† |        0.38x |
| `identity_compact`           | comprehensive-10mb      |         240.6 |               78.6 |  **3.06x**† |        0.16x |
| `identity_compact`           | comprehensive-100mb     |        2515.9 |              823.2 |   **3.06x** |        0.13x |
| `identity_compact`           | corpus-charts           |           1.5 |                1.4 |   **1.10x** |        1.39x |
| `identity_compact`           | corpus-npm              |           1.2 |                1.0 |   **1.22x** |        1.42x |
| `identity_compact`           | corpus-graph            |           1.5 |                1.1 |   **1.38x** |        1.38x |
| `identity_compact`           | corpus-openapi          |           1.8 |                1.3 |   **1.32x** |        1.40x |
| `identity_compact`           | corpus-geojson          |           3.0 |                1.7 |   **1.77x** |        1.17x |
| `identity_compact`           | corpus-tabular          |           1.7 |                1.3 |   **1.39x** |        1.43x |
| `identity_compact`           | corpus-pretty           |           9.0 |                3.6 |   **2.48x** |        0.81x |
| `users_field`                | users-1mb               |           8.9 |                2.3 |   **3.84x** |        0.62x |
| `users_field`                | users-10mb              |          88.3 |               11.7 |  **7.58x**† |        0.20x |
| `users_field`                | users-100mb             |         923.7 |              106.6 |   **8.67x** |        0.15x |
| `users_length`               | users-1mb               |           9.4 |                2.6 |   **3.64x** |        0.63x |
| `users_length`               | users-10mb              |          94.5 |               13.4 |  **7.05x**† |        0.23x |
| `users_length`               | users-100mb             |         989.7 |              127.1 |   **7.79x** |        0.18x |
| `users_names`                | users-1mb               |           9.7 |                4.2 |   **2.29x** |        0.61x |
| `users_names`                | users-10mb              |          95.9 |               30.6 |  **3.13x**† |        0.20x |
| `users_names`                | users-100mb             |        1023.6 |              297.2 |   **3.44x** |        0.15x |
| `users_map_select`           | users-1mb               |          11.0 |                5.5 |   **1.98x** |        0.74x |
| `users_map_select`           | users-10mb              |         131.3 |               41.8 |  **3.14x**† |        0.32x |
| `users_map_select`           | users-100mb             |        1439.0 |              417.7 |   **3.45x** |        0.27x |
| `users_sort_by`              | users-1mb               |          15.0 |                8.6 |  **1.75x**† |        0.82x |
| `users_sort_by`              | users-10mb              |         182.0 |               78.7 |  **2.31x**† |        0.47x |
| `users_group_by`             | users-1mb               |          15.5 |                8.4 |  **1.85x**† |        0.82x |
| `users_group_by`             | users-10mb              |         197.7 |               77.8 |  **2.54x**† |        0.46x |
| `users_unique_by`            | users-1mb               |          16.0 |                8.4 |  **1.89x**† |        0.86x |
| `users_unique_by`            | users-10mb              |         201.0 |               78.1 |  **2.58x**† |        0.48x |
| `users_reduce`               | users-1mb               |          10.3 |                7.1 |   **1.46x** |        0.71x |
| `users_reduce`               | users-10mb              |         103.6 |               58.5 |  **1.77x**† |        0.21x |
| `users_reduce`               | users-100mb             |        1088.6 |              581.9 |   **1.87x** |        0.15x |
| `users_to_entries`           | users-1mb               |           9.0 |                2.3 |   **3.88x** |        0.71x |
| `users_to_entries`           | users-10mb              |          89.1 |               11.9 |  **7.49x**† |        0.21x |
| `users_to_entries`           | users-100mb             |         928.5 |              108.5 |   **8.56x** |        0.15x |
| `users_csv`                  | users-1mb               |          15.5 |                7.7 |  **2.03x**† |        0.62x |
| `users_csv`                  | users-10mb              |         148.6 |               67.0 |  **2.22x**† |        0.20x |
| `users_csv`                  | users-100mb             |        1536.0 |              667.9 |   **2.30x** |        0.14x |
| `users_regex`                | users-1mb               |          15.2 |                5.4 |  **2.80x**† |        0.82x |
| `users_regex`                | users-10mb              |         155.5 |               43.5 |  **3.57x**† |        0.34x |
| `users_ascii_downcase`       | users-1mb               |          26.4 |                5.8 |  **4.55x**† |        0.69x |
| `users_ascii_downcase`       | users-10mb              |         286.0 |               48.9 |  **5.85x**† |        0.25x |
| `users_ascii_downcase`       | users-100mb             |        3113.7 |              484.3 |   **6.43x** |        0.20x |
| `users_first_early_exit`     | users-1mb               |           8.9 |                2.1 |   **4.24x** |        0.65x |
| `users_first_early_exit`     | users-10mb              |          76.0 |                9.9 |  **7.64x**† |        0.20x |
| `users_first_early_exit`     | users-100mb             |         773.9 |               90.9 |   **8.51x** |        0.15x |
| `users_limit_early_exit`     | users-1mb               |           9.0 |                2.1 |   **4.31x** |        0.68x |
| `users_limit_early_exit`     | users-10mb              |          89.0 |               10.3 |  **8.63x**† |        0.21x |
| `users_limit_early_exit`     | users-100mb             |         931.8 |               91.7 |  **10.17x** |        0.15x |
| `users_assign_update`        | users-1mb               |           8.8 |                5.8 |   **1.52x** |        1.18x |
| `users_assign_update`        | users-10mb              |          89.2 |               46.0 |  **1.94x**† |        0.85x |
| `users_del`                  | users-1mb               |           9.1 |                5.9 |   **1.55x** |        1.14x |
| `users_del`                  | users-10mb              |          93.3 |               46.4 |  **2.01x**† |        0.81x |
| `users_paths`                | users-1mb               |          32.8 |               13.1 |  **2.51x**† |        1.05x |
| `users_paths`                | users-10mb              |         352.2 |              130.7 |  **2.69x**† |        0.85x |
| `users_tojson_roundtrip`     | users-1mb               |          21.4 |                9.2 |  **2.34x**† |        1.09x |
| `users_tojson_roundtrip`     | users-10mb              |         237.8 |               85.5 |  **2.78x**† |        0.79x |
| `users_walk`                 | users-1mb               |          76.3 |                7.3 | **10.45x**† |        0.89x |
| `users_walk`                 | users-10mb              |         767.5 |               62.0 | **12.38x**† |        0.59x |
| `users_slurp`                | users-1mb               |           9.0 |                2.7 |   **3.39x** |        0.70x |
| `users_slurp`                | users-10mb              |          88.1 |               16.3 |  **5.41x**† |        0.28x |
| `users_stream`               | users-1mb               |          26.3 |               21.6 |  **1.22x**† |        1.75x |
| `users_stream`               | users-10mb              |         252.4 |              206.0 |  **1.23x**† |        1.83x |
| `numbers_add`                | numbers-1mb             |          27.3 |                5.5 |  **4.95x**† |        0.74x |
| `numbers_add`                | numbers-10mb            |         267.7 |               45.8 |  **5.84x**† |        0.29x |
| `numbers_add`                | numbers-100mb           |        2765.4 |              446.5 |   **6.19x** |        0.22x |
| `numbers_sort`               | numbers-1mb             |          44.4 |               14.1 |  **3.14x**† |        0.82x |
| `numbers_sort`               | numbers-10mb            |         884.2 |              164.1 |  **5.39x**† |        0.57x |
| `numbers_max`                | numbers-1mb             |          12.7 |                5.9 |   **2.15x** |        0.70x |
| `numbers_max`                | numbers-10mb            |         116.9 |               48.2 |  **2.42x**† |        0.29x |
| `numbers_max`                | numbers-100mb           |        1256.4 |              474.3 |   **2.65x** |        0.22x |
| `numbers_map_floor`          | numbers-1mb             |          32.9 |                9.3 |  **3.55x**† |        0.77x |
| `numbers_map_floor`          | numbers-10mb            |         320.4 |               82.8 |  **3.87x**† |        0.48x |
| `numbers_map_floor`          | numbers-100mb           |        3314.8 |              818.0 |   **4.05x** |        0.47x |
| `strings_length`             | strings-1mb             |           7.9 |                3.0 |   **2.61x** |        1.61x |
| `strings_length`             | strings-10mb            |          69.0 |               18.0 |  **3.83x**† |        1.00x |
| `strings_length`             | strings-100mb           |         727.2 |              176.6 |   **4.12x** |        0.82x |
| `strings_upcase`             | strings-1mb             |         324.3 |                7.3 | **44.57x**† |        1.48x |
| `strings_upcase`             | strings-10mb            |        3357.3 |               62.9 |  **53.38x** |        0.96x |
| `strings_upcase`             | strings-100mb           |       33327.6 |              610.8 |  **54.56x** |        0.87x |
| `strings_split`              | strings-1mb             |          12.0 |                6.0 |   **2.00x** |        1.60x |
| `strings_split`              | strings-10mb            |         110.0 |               48.9 |  **2.25x**† |        1.02x |
| `strings_test`               | strings-1mb             |          12.1 |                3.7 |   **3.30x** |        1.77x |
| `strings_test`               | strings-10mb            |         112.4 |               24.7 |  **4.55x**† |        1.04x |
| `wide_keys`                  | wide-1mb                |          29.2 |                6.3 |  **4.64x**† |        0.35x |
| `wide_keys`                  | wide-10mb               |         316.1 |               49.2 |  **6.42x**† |        0.17x |
| `wide_keys`                  | wide-100mb              |        3988.8 |              505.1 |   **7.90x** |        0.14x |
| `wide_to_entries`            | wide-1mb                |          60.1 |               32.3 |  **1.86x**† |        0.55x |
| `wide_to_entries`            | wide-10mb               |         670.3 |              295.1 |  **2.27x**† |        0.50x |
| `wide_map_values`            | wide-1mb                |         124.0 |               38.3 |  **3.24x**† |        0.66x |
| `wide_map_values`            | wide-10mb               |        1240.3 |              372.4 |   **3.33x** |        0.54x |
| `wide_add_values`            | wide-1mb                |          36.5 |               13.3 |  **2.75x**† |        0.50x |
| `wide_add_values`            | wide-10mb               |         376.2 |              114.6 |  **3.28x**† |        0.31x |
| `wide_add_values`            | wide-100mb              |        4350.8 |             1081.9 |   **4.02x** |        0.30x |
| `wide_with_entries`          | wide-1mb                |         298.7 |               95.8 |  **3.12x**† |        0.75x |
| `wide_with_entries`          | wide-10mb               |        3019.6 |              876.0 |   **3.45x** |        0.67x |
| `nested_paths`               | nested-1mb              |           6.1 |                2.4 |   **2.58x** |        1.21x |
| `nested_paths`               | nested-10mb             |          50.6 |               12.5 |  **4.04x**† |        0.71x |
| `nested_recurse`             | nested-1mb              |           6.2 |                3.0 |   **2.07x** |        1.21x |
| `nested_recurse`             | nested-10mb             |          50.8 |               19.2 |  **2.64x**† |        0.72x |
| `comprehensive_paths`        | comprehensive-1mb       |          77.4 |               23.6 |  **3.28x**† |        0.90x |
| `comprehensive_paths`        | comprehensive-10mb      |         796.8 |              224.9 |  **3.54x**† |        0.82x |
| `comprehensive_tostream`     | comprehensive-1mb       |         158.5 |               34.2 |  **4.64x**† |        1.08x |
| `comprehensive_tostream`     | comprehensive-10mb      |        1654.0 |              364.8 |   **4.53x** |        1.02x |
| `comprehensive_numbers`      | comprehensive-1mb       |          57.0 |               19.8 |  **2.88x**† |        0.57x |
| `comprehensive_numbers`      | comprehensive-10mb      |         572.7 |              181.7 |  **3.15x**† |        0.36x |
| `ndjson_identity`            | ndjson-1mb              |          19.9 |                5.5 |  **3.63x**† |        1.53x |
| `ndjson_identity`            | ndjson-10mb             |         185.5 |               41.8 |  **4.43x**† |        1.52x |
| `ndjson_identity`            | ndjson-100mb            |        1838.1 |              396.7 |   **4.63x** |        1.49x |
| `ndjson_field`               | ndjson-1mb              |          11.1 |                6.8 |   **1.65x** |        1.50x |
| `ndjson_field`               | ndjson-10mb             |          95.2 |               52.7 |  **1.81x**† |        1.55x |
| `ndjson_field`               | ndjson-100mb            |         956.3 |              512.4 |   **1.87x** |        1.58x |
| `ndjson_select`              | ndjson-1mb              |          13.2 |               10.4 |   **1.27x** |        1.70x |
| `ndjson_select`              | ndjson-10mb             |         116.8 |               86.9 |  **1.34x**† |        1.72x |
| `ndjson_select`              | ndjson-100mb            |        1154.6 |              848.8 |   **1.36x** |        1.73x |
| `ndjson_tsv`                 | ndjson-1mb              |          20.5 |               12.9 |  **1.59x**† |        1.65x |
| `ndjson_tsv`                 | ndjson-10mb             |         190.8 |              113.4 |  **1.68x**† |        1.70x |
| `ndjson_tsv`                 | ndjson-100mb            |        1890.4 |             1100.9 |   **1.72x** |        1.62x |
| `ndjson_slurp`               | ndjson-1mb              |          17.1 |                7.9 |  **2.16x**† |        0.62x |
| `ndjson_slurp`               | ndjson-10mb             |         186.5 |               65.2 |  **2.86x**† |        0.33x |
| `ndjson_inputs_reduce`       | ndjson-1mb              |          12.6 |                8.0 |   **1.57x** |        1.97x |
| `ndjson_inputs_reduce`       | ndjson-10mb             |         108.9 |               64.7 |  **1.68x**† |        1.95x |
| `ndjson_inputs_reduce`       | ndjson-100mb            |        1084.8 |              623.5 |   **1.74x** |        1.95x |
| `ndjson_inputs_first`        | ndjson-1mb              |           1.3 |                1.1 |   **1.17x** |        1.82x |
| `ndjson_inputs_first`        | ndjson-10mb             |           1.2 |                1.1 |   **1.16x** |        1.80x |
| `ndjson_inputs_first`        | ndjson-100mb            |           1.3 |                1.1 |   **1.17x** |        1.81x |
| `ndjson_raw_lines`           | ndjson-1mb              |           6.4 |                3.0 |   **2.14x** |        1.71x |
| `ndjson_raw_lines`           | ndjson-10mb             |          51.5 |               16.6 |  **3.09x**† |        1.74x |
| `ndjson_raw_lines`           | ndjson-100mb            |         503.2 |              151.8 |  **3.31x**† |        1.74x |
| `ndjson_raw_fromjson`        | ndjson-1mb              |          14.1 |                9.6 |   **1.46x** |        1.81x |
| `ndjson_raw_fromjson`        | ndjson-10mb             |         124.5 |               80.9 |  **1.54x**† |        1.84x |
| `ndjson_raw_slurp`           | ndjson-1mb              |           5.1 |                1.7 |   **3.04x** |        1.49x |
| `ndjson_raw_slurp`           | ndjson-10mb             |          41.6 |                5.8 |  **7.18x**† |        0.75x |
| `ndjson_raw_slurp`           | ndjson-100mb            |         393.9 |               46.3 |  **8.51x**† |        0.76x |
| `ndjson_three_files`         | ndjson-1mb              |          31.5 |               16.1 |  **1.96x**† |        1.60x |
| `ndjson_three_files`         | ndjson-10mb             |         301.5 |              141.7 |  **2.13x**† |        1.63x |
| `users_three_files`          | users-1mb               |          19.4 |                4.0 |  **4.83x**† |        0.67x |
| `users_three_files`          | users-10mb              |         219.4 |               31.4 |  **6.98x**† |        0.21x |
| `seq_identity`               | jsonseq-1mb             |          19.9 |                7.3 |  **2.73x**† |        1.56x |
| `seq_identity`               | jsonseq-10mb            |         185.9 |               59.3 |  **3.14x**† |        1.59x |
| `seq_identity`               | jsonseq-100mb           |        1850.5 |              568.6 |   **3.25x** |        1.68x |
| `seq_field`                  | jsonseq-1mb             |          11.2 |                8.5 |   **1.32x** |        1.62x |
| `seq_field`                  | jsonseq-10mb            |          97.2 |               68.1 |  **1.43x**† |        1.60x |
| `seq_field`                  | jsonseq-100mb           |         964.7 |              663.8 |   **1.45x** |        1.57x |
| `seq_slurp`                  | jsonseq-1mb             |          17.1 |               10.2 |  **1.68x**† |        0.66x |
| `seq_slurp`                  | jsonseq-10mb            |         188.5 |               87.8 |  **2.15x**† |        0.33x |
| `users_seq_output`           | users-1mb               |           2.4 |                1.2 |   **1.98x** |        1.67x |
| `users_seq_output`           | users-10mb              |          12.7 |                2.0 |   **6.48x** |        1.57x |
| `users_seq_records`          | users-seq-1mb           |          10.2 |                5.4 |   **1.90x** |        0.65x |
| `users_seq_records`          | users-seq-10mb          |          90.4 |               41.8 |  **2.17x**† |        0.20x |
| `seq_field_bad_first`        | jsonseq-bad-first-1mb   |          11.2 |                8.6 |   **1.31x** |        1.83x |
| `seq_field_bad_first`        | jsonseq-bad-first-10mb  |          96.1 |               68.3 |  **1.41x**† |        1.79x |
| `seq_field_bad_first`        | jsonseq-bad-first-100mb |         968.8 |              663.6 |   **1.46x** |        1.78x |
| `seq_field_bad_last`         | jsonseq-bad-last-1mb    |          11.3 |                8.4 |   **1.34x** |        1.88x |
| `seq_field_bad_last`         | jsonseq-bad-last-10mb   |          97.1 |               68.7 |  **1.41x**† |        1.86x |
| `seq_field_bad_last`         | jsonseq-bad-last-100mb  |         963.8 |              671.5 |   **1.44x** |        1.87x |
| `users_slurpfile`            | users-1mb               |           9.1 |                4.8 |   **1.88x** |        1.04x |
| `users_slurpfile`            | users-10mb              |          88.7 |               34.7 |  **2.56x**† |        0.80x |
| `ndjson_slurpfile`           | ndjson-1mb              |          13.5 |                6.7 |   **2.02x** |        0.92x |
| `ndjson_slurpfile`           | ndjson-10mb             |         136.3 |               53.0 |  **2.57x**† |        0.79x |
| `users_rawfile`              | users-1mb               |           3.4 |                1.6 |   **2.19x** |        1.57x |
| `users_rawfile`              | users-10mb              |          22.8 |                4.0 |  **5.67x**† |        1.12x |
| `users_rawfile`              | users-100mb             |         245.4 |               32.1 |  **7.66x**† |        0.56x |
| `args_many`                  | none                    |           1.2 |                1.1 |   **1.14x** |        1.73x |
| `corpus_npm_deps`            | corpus-npm              |           1.2 |                1.1 |   **1.16x** |        1.77x |
| `corpus_npm_scripts`         | corpus-npm              |           1.3 |                1.1 |   **1.16x** |        1.80x |
| `corpus_charts_titles`       | corpus-charts           |           1.2 |                1.0 |   **1.18x** |        1.47x |
| `corpus_graph_weights`       | corpus-graph            |           1.4 |                1.1 |   **1.27x** |        1.59x |
| `corpus_graph_by_source`     | corpus-graph            |           1.5 |                1.3 |   **1.22x** |        1.68x |
| `corpus_graph_select`        | corpus-graph            |           1.4 |                1.1 |   **1.26x** |        1.64x |
| `corpus_openapi_definitions` | corpus-openapi          |           1.6 |                1.1 |   **1.40x** |        1.63x |
| `corpus_openapi_strings`     | corpus-openapi          |           2.0 |                1.5 |   **1.35x** |        1.61x |
| `corpus_openapi_refs`        | corpus-openapi          |           2.2 |                1.7 |   **1.29x** |        1.82x |
| `corpus_openapi_paths`       | corpus-openapi          |           2.1 |                1.6 |   **1.35x** |        1.69x |
| `corpus_geojson_districts`   | corpus-geojson          |           2.1 |                1.2 |   **1.78x** |        1.32x |
| `corpus_geojson_types`       | corpus-geojson          |           2.2 |                1.3 |   **1.76x** |        1.43x |
| `corpus_geojson_points`      | corpus-geojson          |           6.0 |                2.5 |   **2.38x** |        1.48x |
| `corpus_tabular_hours`       | corpus-tabular          |           1.7 |                1.3 |   **1.31x** |        1.80x |
| `corpus_tabular_csv`         | corpus-tabular          |           1.8 |                1.5 |   **1.20x** |        1.68x |
| `corpus_tabular_group`       | corpus-tabular          |           1.7 |                1.3 |   **1.28x** |        1.78x |
| `corpus_pretty_numbers`      | corpus-pretty           |          15.7 |                5.4 |  **2.89x**† |        0.97x |
| `corpus_pretty_paths`        | corpus-pretty           |          20.2 |                6.9 |  **2.91x**† |        1.16x |
| `corpus_pretty_tostream`     | corpus-pretty           |          36.2 |                8.7 |  **4.15x**† |        1.26x |
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

**Run**: `./target/release/succinctly dev compare jq --full --huge` on ARM Neoverse-V2 (Amazon Linux 2023.12.20261112), succinctly `73f049b2c`, run `20261119_011805_73f049b2c`. jq-1.7.1 is built from source by the harness; the tools are interleaved within each of 21 repetitions, and a row is timed only when the outputs are identical.

**Timed binary**: the harness itself (timed as a copy stripped like the release archive's binary), built with `cli,simd,regex,serde` plus `bench-runner`. The release archives are built with `cli` alone, and no figure here is of that build ([reproducing-the-jq-comparison.md](../guides/reproducing-the-jq-comparison.md#which-binary-a-run-measures)).

Geomean speedup over jq-1.7.1: **2.71x** across 221 timed rows; jq-1.7.1 is faster on 28 of them. Speedup is jq-1.7.1's time over succinctly jq's, so below 1.00x jq is faster; memory is succinctly jq's peak RSS over jq-1.7.1's. Times are medians, except where marked †: both times and the ratio are then each tool's fastest run, which run-to-run noise moves least on a long row.

| Query                        | Input                   | jq-1.7.1 (ms) | succinctly jq (ms) |     Speedup | Memory ratio |
|------------------------------|-------------------------|---------------|--------------------|-------------|--------------|
| `startup`                    | none                    |           1.3 |                1.4 |       0.92x |        3.45x |
| `identity`                   | users-1mb               |          35.9 |                4.5 |  **7.93x**† |        1.07x |
| `identity`                   | users-10mb              |         346.6 |               32.4 | **10.71x**† |        0.24x |
| `identity`                   | users-100mb             |        3520.5 |              308.7 |  **11.41x** |        0.14x |
| `identity`                   | numbers-1mb             |          35.5 |                6.5 |  **5.49x**† |        0.90x |
| `identity`                   | numbers-10mb            |         341.4 |               50.4 |  **6.78x**† |        0.22x |
| `identity`                   | numbers-100mb           |        3429.7 |              490.2 |   **7.00x** |        0.14x |
| `identity`                   | strings-1mb             |          32.1 |                3.0 | **10.74x**† |        2.86x |
| `identity`                   | strings-10mb            |         310.6 |               17.1 | **18.13x**† |        1.29x |
| `identity`                   | strings-100mb           |        3118.6 |              159.7 |  **19.53x** |        0.87x |
| `identity`                   | wide-1mb                |          58.2 |                8.2 |  **7.08x**† |        0.60x |
| `identity`                   | wide-10mb               |         655.9 |               70.5 |  **9.30x**† |        0.24x |
| `identity`                   | wide-100mb              |        6838.8 |              631.9 |  **10.82x** |        0.14x |
| `identity`                   | nested-1mb              |          35.5 |                3.0 | **11.86x**† |        2.06x |
| `identity`                   | nested-10mb             |         342.3 |               17.9 | **19.09x**† |        0.89x |
| `identity`                   | nested-100mb            |        3427.2 |              162.7 |  **21.07x** |        0.63x |
| `identity`                   | comprehensive-1mb       |          63.4 |               15.7 |  **4.03x**† |        0.68x |
| `identity`                   | comprehensive-10mb      |         610.1 |              136.6 |  **4.47x**† |        0.25x |
| `identity`                   | comprehensive-100mb     |        6118.6 |             1404.6 |   **4.36x** |        0.20x |
| `identity`                   | corpus-charts           |           1.3 |                1.4 |       0.95x |        3.44x |
| `identity`                   | corpus-npm              |           1.4 |                1.4 |       0.98x |        3.23x |
| `identity`                   | corpus-graph            |           2.0 |                1.5 |   **1.38x** |        3.23x |
| `identity`                   | corpus-openapi          |           2.8 |                1.9 |   **1.44x** |        3.39x |
| `identity`                   | corpus-geojson          |           7.4 |                2.7 |   **2.79x** |        2.74x |
| `identity`                   | corpus-tabular          |           2.7 |                1.8 |   **1.51x** |        3.40x |
| `identity`                   | corpus-pretty           |          20.2 |                6.6 |  **3.05x**† |        1.66x |
| `identity_compact`           | users-1mb               |          28.5 |                3.6 |  **8.04x**† |        1.03x |
| `identity_compact`           | users-10mb              |         273.9 |               22.5 | **12.17x**† |        0.24x |
| `identity_compact`           | users-100mb             |        2799.3 |              212.3 |  **13.19x** |        0.14x |
| `identity_compact`           | numbers-1mb             |          29.0 |                5.0 |  **5.77x**† |        0.86x |
| `identity_compact`           | numbers-10mb            |         278.4 |               36.2 |  **7.68x**† |        0.22x |
| `identity_compact`           | numbers-100mb           |        2783.4 |              349.0 |   **7.98x** |        0.14x |
| `identity_compact`           | strings-1mb             |          31.4 |                2.4 | **13.14x**† |        2.83x |
| `identity_compact`           | strings-10mb            |         303.6 |               11.6 | **26.27x**† |        1.28x |
| `identity_compact`           | strings-100mb           |        3047.4 |              103.2 |  **29.52x** |        0.87x |
| `identity_compact`           | wide-1mb                |          54.7 |                6.7 |  **8.15x**† |        0.59x |
| `identity_compact`           | wide-10mb               |         622.8 |               55.4 | **11.23x**† |        0.24x |
| `identity_compact`           | wide-100mb              |        6476.4 |              487.9 |  **13.27x** |        0.14x |
| `identity_compact`           | nested-1mb              |          35.5 |                2.4 | **14.63x**† |        2.05x |
| `identity_compact`           | nested-10mb             |         342.7 |               11.6 | **29.42x**† |        0.88x |
| `identity_compact`           | nested-100mb            |        3427.1 |              104.8 |  **32.69x** |        0.63x |
| `identity_compact`           | comprehensive-1mb       |          41.8 |               15.0 |  **2.80x**† |        0.63x |
| `identity_compact`           | comprehensive-10mb      |         401.3 |              128.9 |  **3.11x**† |        0.19x |
| `identity_compact`           | comprehensive-100mb     |        4029.1 |             1318.6 |   **3.06x** |        0.14x |
| `identity_compact`           | corpus-charts           |           1.3 |                1.4 |       0.94x |        3.62x |
| `identity_compact`           | corpus-npm              |           1.4 |                1.4 |       0.97x |        3.28x |
| `identity_compact`           | corpus-graph            |           1.8 |                1.4 |   **1.27x** |        3.10x |
| `identity_compact`           | corpus-openapi          |           2.5 |                1.9 |   **1.29x** |        3.45x |
| `identity_compact`           | corpus-geojson          |           4.0 |                2.6 |   **1.57x** |        2.75x |
| `identity_compact`           | corpus-tabular          |           2.5 |                1.8 |   **1.42x** |        3.34x |
| `identity_compact`           | corpus-pretty           |          13.0 |                6.5 |   **2.01x** |        1.58x |
| `users_field`                | users-1mb               |          12.3 |                3.3 |   **3.69x** |        1.29x |
| `users_field`                | users-10mb              |         113.6 |               18.6 |  **6.12x**† |        0.28x |
| `users_field`                | users-100mb             |        1164.5 |              175.6 |   **6.63x** |        0.15x |
| `users_length`               | users-1mb               |          12.8 |                3.5 |   **3.61x** |        1.29x |
| `users_length`               | users-10mb              |         119.4 |               19.9 |  **6.01x**† |        0.31x |
| `users_length`               | users-100mb             |        1223.6 |              184.6 |   **6.63x** |        0.18x |
| `users_names`                | users-1mb               |          13.3 |                7.2 |   **1.85x** |        1.22x |
| `users_names`                | users-10mb              |         124.0 |               56.3 |  **2.20x**† |        0.28x |
| `users_names`                | users-100mb             |        1261.2 |              552.6 |   **2.28x** |        0.15x |
| `users_map_select`           | users-1mb               |          14.8 |                9.1 |  **1.63x**† |        1.46x |
| `users_map_select`           | users-10mb              |         144.3 |               74.8 |  **1.93x**† |        0.42x |
| `users_map_select`           | users-100mb             |        1474.6 |              733.9 |   **2.01x** |        0.28x |
| `users_sort_by`              | users-1mb               |          19.1 |               14.0 |  **1.37x**† |        1.42x |
| `users_sort_by`              | users-10mb              |         262.4 |              138.4 |  **1.90x**† |        0.54x |
| `users_group_by`             | users-1mb               |          21.4 |               13.6 |  **1.58x**† |        1.43x |
| `users_group_by`             | users-10mb              |         285.1 |              136.7 |  **2.09x**† |        0.53x |
| `users_unique_by`            | users-1mb               |          21.7 |               13.9 |  **1.56x**† |        1.44x |
| `users_unique_by`            | users-10mb              |         288.8 |              138.2 |  **2.09x**† |        0.55x |
| `users_reduce`               | users-1mb               |          14.2 |               12.0 |  **1.18x**† |        1.47x |
| `users_reduce`               | users-10mb              |         134.9 |              104.8 |  **1.29x**† |        0.31x |
| `users_reduce`               | users-100mb             |        1342.7 |             1029.4 |   **1.30x** |        0.16x |
| `users_to_entries`           | users-1mb               |          12.2 |                3.5 |   **3.47x** |        1.54x |
| `users_to_entries`           | users-10mb              |         114.2 |               18.9 |  **6.06x**† |        0.31x |
| `users_to_entries`           | users-100mb             |        1163.7 |              177.1 |   **6.57x** |        0.16x |
| `users_csv`                  | users-1mb               |          21.1 |               17.8 |  **1.19x**† |        1.29x |
| `users_csv`                  | users-10mb              |         197.1 |              162.7 |  **1.21x**† |        0.28x |
| `users_csv`                  | users-100mb             |        2019.9 |             1624.5 |   **1.24x** |        0.15x |
| `users_regex`                | users-1mb               |          20.9 |                9.3 |  **2.24x**† |        1.61x |
| `users_regex`                | users-10mb              |         200.0 |               76.4 |  **2.62x**† |        0.43x |
| `users_ascii_downcase`       | users-1mb               |          35.6 |               10.2 |  **3.48x**† |        1.48x |
| `users_ascii_downcase`       | users-10mb              |         369.0 |               87.8 |  **4.20x**† |        0.38x |
| `users_ascii_downcase`       | users-100mb             |        3949.1 |              872.6 |   **4.53x** |        0.25x |
| `users_first_early_exit`     | users-1mb               |          12.3 |                3.1 |   **4.01x** |        1.30x |
| `users_first_early_exit`     | users-10mb              |         106.1 |               15.7 |  **6.77x**† |        0.29x |
| `users_first_early_exit`     | users-100mb             |        1087.5 |              147.2 |   **7.39x** |        0.16x |
| `users_limit_early_exit`     | users-1mb               |          12.4 |                3.1 |   **4.00x** |        1.37x |
| `users_limit_early_exit`     | users-10mb              |         116.6 |               16.1 |  **7.25x**† |        0.29x |
| `users_limit_early_exit`     | users-100mb             |        1163.6 |              148.1 |   **7.86x** |        0.16x |
| `users_assign_update`        | users-1mb               |          12.2 |               10.9 |   **1.12x** |        2.23x |
| `users_assign_update`        | users-10mb              |         114.3 |               91.4 |  **1.25x**† |        1.39x |
| `users_del`                  | users-1mb               |          13.0 |               11.4 |   **1.14x** |        2.16x |
| `users_del`                  | users-10mb              |         118.1 |               96.7 |  **1.22x**† |        1.30x |
| `users_paths`                | users-1mb               |          42.3 |               22.5 |  **1.88x**† |        1.71x |
| `users_paths`                | users-10mb              |         421.9 |              211.9 |  **1.99x**† |        1.14x |
| `users_tojson_roundtrip`     | users-1mb               |          30.7 |               19.9 |  **1.54x**† |        1.85x |
| `users_tojson_roundtrip`     | users-10mb              |         300.0 |              183.7 |  **1.63x**† |        1.20x |
| `users_walk`                 | users-1mb               |          98.4 |               22.8 |  **4.31x**† |        2.33x |
| `users_walk`                 | users-10mb              |         981.4 |              210.7 |  **4.66x**† |        1.75x |
| `users_slurp`                | users-1mb               |          12.5 |                3.6 |   **3.48x** |        1.33x |
| `users_slurp`                | users-10mb              |         115.7 |               21.1 |  **5.49x**† |        0.34x |
| `users_stream`               | users-1mb               |          34.6 |               40.1 |      0.86x† |        4.50x |
| `users_stream`               | users-10mb              |         335.1 |              388.0 |      0.86x† |        4.38x |
| `numbers_add`                | numbers-1mb             |          36.7 |               11.8 |  **3.10x**† |        1.40x |
| `numbers_add`                | numbers-10mb            |         352.3 |              103.7 |  **3.40x**† |        0.37x |
| `numbers_add`                | numbers-100mb           |        3549.3 |             1021.2 |   **3.48x** |        0.23x |
| `numbers_sort`               | numbers-1mb             |          82.9 |               40.7 |  **2.04x**† |        1.84x |
| `numbers_sort`               | numbers-10mb            |        1602.9 |              438.7 |   **3.65x** |        1.13x |
| `numbers_max`                | numbers-1mb             |          17.6 |               10.9 |  **1.62x**† |        1.37x |
| `numbers_max`                | numbers-10mb            |         164.0 |               94.1 |  **1.74x**† |        0.37x |
| `numbers_max`                | numbers-100mb           |        1660.5 |              924.5 |   **1.80x** |        0.23x |
| `numbers_map_floor`          | numbers-1mb             |          42.0 |               15.3 |  **2.75x**† |        1.50x |
| `numbers_map_floor`          | numbers-10mb            |         405.9 |              138.1 |  **2.94x**† |        0.82x |
| `numbers_map_floor`          | numbers-100mb           |        4093.5 |             1375.2 |   **2.98x** |        0.79x |
| `strings_length`             | strings-1mb             |          12.5 |                4.2 |   **2.99x** |        3.56x |
| `strings_length`             | strings-10mb            |         115.1 |               26.5 |  **4.35x**† |        1.44x |
| `strings_length`             | strings-100mb           |        1152.8 |              244.1 |   **4.72x** |        0.99x |
| `strings_upcase`             | strings-1mb             |         421.4 |               11.6 | **36.23x**† |        3.06x |
| `strings_upcase`             | strings-10mb            |        4242.0 |              102.1 |  **41.56x** |        1.30x |
| `strings_upcase`             | strings-100mb           |       42543.7 |              990.5 |  **42.95x** |        0.98x |
| `strings_split`              | strings-1mb             |          17.8 |               10.3 |  **1.73x**† |        3.49x |
| `strings_split`              | strings-10mb            |         168.4 |               87.3 |  **1.93x**† |        1.46x |
| `strings_test`               | strings-1mb             |          18.9 |                5.4 |  **3.50x**† |        3.60x |
| `strings_test`               | strings-10mb            |         179.2 |               38.2 |  **4.69x**† |        1.50x |
| `wide_keys`                  | wide-1mb                |          39.5 |               10.0 |  **3.97x**† |        0.62x |
| `wide_keys`                  | wide-10mb               |         472.6 |               80.9 |  **5.84x**† |        0.21x |
| `wide_keys`                  | wide-100mb              |        5095.1 |              830.8 |   **6.13x** |        0.14x |
| `wide_to_entries`            | wide-1mb                |          86.5 |               67.5 |  **1.28x**† |        1.28x |
| `wide_to_entries`            | wide-10mb               |         956.5 |              626.2 |   **1.53x** |        1.18x |
| `wide_map_values`            | wide-1mb                |         165.2 |               71.9 |  **2.30x**† |        2.09x |
| `wide_map_values`            | wide-10mb               |        1687.4 |              798.2 |   **2.11x** |        1.93x |
| `wide_add_values`            | wide-1mb                |          50.5 |               19.1 |  **2.65x**† |        0.74x |
| `wide_add_values`            | wide-10mb               |         577.1 |              164.8 |  **3.50x**† |        0.34x |
| `wide_add_values`            | wide-100mb              |        5815.5 |             1588.3 |   **3.66x** |        0.30x |
| `wide_with_entries`          | wide-1mb                |         385.1 |              197.7 |  **1.95x**† |        1.42x |
| `wide_with_entries`          | wide-10mb               |        3843.1 |             1858.8 |   **2.07x** |        1.24x |
| `nested_paths`               | nested-1mb              |          10.9 |                3.7 |   **2.92x** |        2.33x |
| `nested_paths`               | nested-10mb             |          91.1 |               22.1 |  **4.12x**† |        0.95x |
| `nested_recurse`             | nested-1mb              |          10.8 |                4.9 |   **2.19x** |        2.38x |
| `nested_recurse`             | nested-10mb             |          91.5 |               34.5 |  **2.65x**† |        0.95x |
| `comprehensive_paths`        | comprehensive-1mb       |          96.0 |               44.9 |  **2.14x**† |        1.42x |
| `comprehensive_paths`        | comprehensive-10mb      |         944.6 |              428.0 |  **2.21x**† |        1.21x |
| `comprehensive_tostream`     | comprehensive-1mb       |         193.8 |               80.1 |  **2.42x**† |        1.88x |
| `comprehensive_tostream`     | comprehensive-10mb      |        1894.7 |              679.5 |   **2.79x** |        1.72x |
| `comprehensive_numbers`      | comprehensive-1mb       |          75.4 |               36.9 |  **2.04x**† |        1.84x |
| `comprehensive_numbers`      | comprehensive-10mb      |         744.0 |              341.1 |  **2.18x**† |        1.44x |
| `ndjson_identity`            | ndjson-1mb              |          40.1 |                9.7 |  **4.13x**† |        3.67x |
| `ndjson_identity`            | ndjson-10mb             |         384.4 |               81.0 |  **4.75x**† |        3.62x |
| `ndjson_identity`            | ndjson-100mb            |        3800.1 |              769.8 |   **4.94x** |        3.75x |
| `ndjson_field`               | ndjson-1mb              |          16.2 |               12.7 |  **1.28x**† |        3.76x |
| `ndjson_field`               | ndjson-10mb             |         147.9 |              108.7 |  **1.36x**† |        3.95x |
| `ndjson_field`               | ndjson-100mb            |        1457.4 |             1046.7 |   **1.39x** |        3.81x |
| `ndjson_select`              | ndjson-1mb              |          19.0 |               19.4 |      0.98x† |        4.32x |
| `ndjson_select`              | ndjson-10mb             |         174.6 |              172.0 |  **1.02x**† |        4.39x |
| `ndjson_select`              | ndjson-100mb            |        1713.0 |             1662.9 |   **1.03x** |        4.28x |
| `ndjson_tsv`                 | ndjson-1mb              |          28.2 |               31.6 |      0.89x† |        4.20x |
| `ndjson_tsv`                 | ndjson-10mb             |         264.8 |              291.8 |      0.91x† |        4.32x |
| `ndjson_tsv`                 | ndjson-100mb            |        2610.0 |             2841.7 |       0.92x |        4.23x |
| `ndjson_slurp`               | ndjson-1mb              |          24.0 |               13.6 |  **1.77x**† |        1.18x |
| `ndjson_slurp`               | ndjson-10mb             |         226.3 |              113.8 |  **1.99x**† |        0.39x |
| `ndjson_inputs_reduce`       | ndjson-1mb              |          17.7 |               19.3 |      0.92x† |        4.97x |
| `ndjson_inputs_reduce`       | ndjson-10mb             |         161.6 |              169.1 |      0.96x† |        5.39x |
| `ndjson_inputs_reduce`       | ndjson-100mb            |        1592.4 |             1627.7 |       0.98x |        5.40x |
| `ndjson_inputs_first`        | ndjson-1mb              |           1.4 |                1.6 |       0.86x |        4.32x |
| `ndjson_inputs_first`        | ndjson-10mb             |           1.4 |                1.7 |       0.83x |        4.17x |
| `ndjson_inputs_first`        | ndjson-100mb            |           1.4 |                1.6 |       0.84x |        4.29x |
| `ndjson_raw_lines`           | ndjson-1mb              |           9.3 |                4.9 |   **1.87x** |        3.47x |
| `ndjson_raw_lines`           | ndjson-10mb             |          78.7 |               33.0 |  **2.39x**† |        3.47x |
| `ndjson_raw_lines`           | ndjson-100mb            |         776.5 |              305.1 |   **2.54x** |        3.57x |
| `ndjson_raw_fromjson`        | ndjson-1mb              |          20.1 |               18.2 |  **1.11x**† |        3.61x |
| `ndjson_raw_fromjson`        | ndjson-10mb             |         185.5 |              159.6 |  **1.16x**† |        3.87x |
| `ndjson_raw_slurp`           | ndjson-1mb              |           7.0 |                2.2 |   **3.10x** |        2.64x |
| `ndjson_raw_slurp`           | ndjson-10mb             |          59.2 |                9.4 |  **6.33x**† |        1.41x |
| `ndjson_raw_slurp`           | ndjson-100mb            |         558.0 |               69.9 |  **7.98x**† |        1.51x |
| `ndjson_three_files`         | ndjson-1mb              |          47.6 |               33.8 |  **1.41x**† |        4.36x |
| `ndjson_three_files`         | ndjson-10mb             |         456.8 |              305.3 |  **1.50x**† |        3.97x |
| `users_three_files`          | users-1mb               |          31.0 |                7.0 |  **4.40x**† |        1.41x |
| `users_three_files`          | users-10mb              |         309.5 |               56.1 |  **5.52x**† |        0.28x |
| `seq_identity`               | jsonseq-1mb             |          40.1 |               13.1 |  **3.06x**† |        3.83x |
| `seq_identity`               | jsonseq-10mb            |         384.4 |              113.6 |  **3.38x**† |        3.66x |
| `seq_identity`               | jsonseq-100mb           |        3802.7 |             1093.9 |   **3.48x** |        3.90x |
| `seq_field`                  | jsonseq-1mb             |          16.3 |               15.5 |  **1.06x**† |        3.86x |
| `seq_field`                  | jsonseq-10mb            |         148.5 |              136.7 |  **1.09x**† |        3.69x |
| `seq_field`                  | jsonseq-100mb           |        1461.9 |             1326.0 |   **1.10x** |        3.76x |
| `seq_slurp`                  | jsonseq-1mb             |          24.2 |               17.5 |  **1.38x**† |        1.20x |
| `seq_slurp`                  | jsonseq-10mb            |         226.5 |              152.6 |  **1.48x**† |        0.39x |
| `users_seq_output`           | users-1mb               |           3.0 |                2.0 |   **1.51x** |        3.30x |
| `users_seq_output`           | users-10mb              |          17.8 |                5.5 |  **3.22x**† |        6.19x |
| `seq_field_bad_first`        | jsonseq-bad-first-1mb   |          16.3 |               17.9 |      0.91x† |        3.67x |
| `seq_field_bad_first`        | jsonseq-bad-first-10mb  |         148.3 |              135.5 |  **1.09x**† |        4.05x |
| `seq_field_bad_first`        | jsonseq-bad-first-100mb |        1458.9 |             1315.9 |   **1.11x** |        3.94x |
| `seq_field_bad_last`         | jsonseq-bad-last-1mb    |          16.3 |               18.1 |      0.90x† |        3.95x |
| `seq_field_bad_last`         | jsonseq-bad-last-10mb   |         147.8 |              136.3 |  **1.08x**† |        4.17x |
| `seq_field_bad_last`         | jsonseq-bad-last-100mb  |        1461.0 |             1321.7 |   **1.11x** |        4.23x |
| `users_slurpfile`            | users-1mb               |          12.8 |                8.3 |   **1.55x** |        1.89x |
| `users_slurpfile`            | users-10mb              |         107.2 |               63.0 |  **1.70x**† |        1.25x |
| `ndjson_slurpfile`           | ndjson-1mb              |          17.6 |               12.3 |  **1.42x**† |        1.72x |
| `ndjson_slurpfile`           | ndjson-10mb             |         161.8 |              104.8 |  **1.54x**† |        1.30x |
| `users_rawfile`              | users-1mb               |           4.3 |                1.8 |   **2.32x** |        3.31x |
| `users_rawfile`              | users-10mb              |          31.1 |                4.4 |  **7.14x**† |        1.67x |
| `users_rawfile`              | users-100mb             |         323.3 |               29.3 | **11.02x**† |        0.59x |
| `args_many`                  | none                    |           1.3 |                1.6 |       0.85x |        4.00x |
| `corpus_npm_deps`            | corpus-npm              |           1.4 |                1.7 |       0.84x |        4.36x |
| `corpus_npm_scripts`         | corpus-npm              |           1.4 |                1.6 |       0.86x |        4.49x |
| `corpus_charts_titles`       | corpus-charts           |           1.3 |                1.5 |       0.90x |        3.93x |
| `corpus_graph_weights`       | corpus-graph            |           1.6 |                1.7 |       0.98x |        3.99x |
| `corpus_graph_by_source`     | corpus-graph            |           1.7 |                1.9 |       0.90x |        4.55x |
| `corpus_graph_select`        | corpus-graph            |           1.6 |                1.7 |       0.95x |        4.27x |
| `corpus_openapi_definitions` | corpus-openapi          |           1.9 |                1.7 |   **1.17x** |        3.61x |
| `corpus_openapi_strings`     | corpus-openapi          |           2.4 |                2.2 |   **1.09x** |        4.31x |
| `corpus_openapi_refs`        | corpus-openapi          |           2.7 |                2.8 |       0.98x |        5.02x |
| `corpus_openapi_paths`       | corpus-openapi          |           2.6 |                2.4 |   **1.07x** |        4.10x |
| `corpus_geojson_districts`   | corpus-geojson          |           2.8 |                1.9 |   **1.46x** |        3.09x |
| `corpus_geojson_types`       | corpus-geojson          |           2.8 |                2.1 |   **1.36x** |        3.72x |
| `corpus_geojson_points`      | corpus-geojson          |           7.6 |                4.3 |   **1.76x** |        3.89x |
| `corpus_tabular_hours`       | corpus-tabular          |           2.1 |                2.1 |   **1.02x** |        4.41x |
| `corpus_tabular_csv`         | corpus-tabular          |           2.2 |                2.6 |       0.87x |        4.34x |
| `corpus_tabular_group`       | corpus-tabular          |           2.1 |                2.1 |   **1.01x** |        4.16x |
| `corpus_pretty_numbers`      | corpus-pretty           |          22.0 |               11.1 |  **1.99x**† |        2.50x |
| `corpus_pretty_paths`        | corpus-pretty           |          26.1 |               13.8 |  **1.89x**† |        1.98x |
| `corpus_pretty_tostream`     | corpus-pretty           |          46.3 |               19.6 |  **2.37x**† |        2.36x |
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

**Run**: `./target/release/succinctly dev compare jq --full --huge` on Apple M4 Pro (macOS 26.5.1), succinctly `955a984`, run `20261125_125910_955a984`. jq-1.7.1 is built from source by the harness; the tools are interleaved within each of 21 repetitions, and a row is timed only when the outputs are identical.

**Timed binary**: the harness itself (timed as a copy stripped like the release archive's binary), built with `cli,simd,regex,serde` plus `bench-runner`. The release archives are built with `cli` alone, and no figure here is of that build ([reproducing-the-jq-comparison.md](../guides/reproducing-the-jq-comparison.md#which-binary-a-run-measures)).

Geomean speedup over jq-1.7.1: **2.84x** across 223 timed rows; jq-1.7.1 is faster on 26 of them. Speedup is jq-1.7.1's time over succinctly jq's, so below 1.00x jq is faster; memory is succinctly jq's peak RSS over jq-1.7.1's. Times are medians, except where marked †: both times and the ratio are then each tool's fastest run, which run-to-run noise moves least on a long row.

| Query                        | Input                   | jq-1.7.1 (ms) | succinctly jq (ms) |     Speedup | Memory ratio |
|------------------------------|-------------------------|---------------|--------------------|-------------|--------------|
| `startup`                    | none                    |           1.8 |                2.5 |       0.70x |        2.74x |
| `identity`                   | users-1mb               |          24.8 |                4.5 |  **5.45x**† |        0.87x |
| `identity`                   | users-10mb              |         238.4 |               22.6 | **10.53x**† |        0.24x |
| `identity`                   | users-100mb             |        2491.0 |              198.4 |  **12.55x** |        0.14x |
| `identity`                   | numbers-1mb             |          22.4 |                5.4 |  **4.15x**† |        0.74x |
| `identity`                   | numbers-10mb            |         208.9 |               33.1 |  **6.30x**† |        0.20x |
| `identity`                   | numbers-100mb           |        2119.6 |              304.1 |   **6.97x** |        0.12x |
| `identity`                   | strings-1mb             |          21.8 |                3.2 |  **6.74x**† |        2.34x |
| `identity`                   | strings-10mb            |         203.9 |               10.3 | **19.80x**† |        1.13x |
| `identity`                   | strings-100mb           |        2183.3 |               80.1 |  **27.25x** |        0.72x |
| `identity`                   | wide-1mb                |          36.0 |                6.8 |  **5.29x**† |        0.44x |
| `identity`                   | wide-10mb               |         346.3 |               39.4 |  **8.78x**† |        0.20x |
| `identity`                   | wide-100mb              |        3984.6 |              356.1 |  **11.19x** |        0.12x |
| `identity`                   | nested-1mb              |          23.5 |                3.1 |  **7.54x**† |        1.79x |
| `identity`                   | nested-10mb             |         220.6 |                9.6 | **23.00x**† |        0.78x |
| `identity`                   | nested-100mb            |        2366.3 |               67.0 |  **35.34x** |        0.63x |
| `identity`                   | comprehensive-1mb       |          42.7 |               11.7 |  **3.66x**† |        0.65x |
| `identity`                   | comprehensive-10mb      |         401.2 |               81.6 |  **4.92x**† |        0.29x |
| `identity`                   | comprehensive-100mb     |        4008.3 |              819.6 |   **4.89x** |        0.23x |
| `identity`                   | corpus-charts           |           1.8 |                2.5 |       0.69x |        2.84x |
| `identity`                   | corpus-npm              |           1.8 |                2.5 |       0.71x |        2.83x |
| `identity`                   | corpus-graph            |           2.4 |                2.5 |       0.95x |        2.57x |
| `identity`                   | corpus-openapi          |           3.0 |                2.9 |   **1.05x** |        2.58x |
| `identity`                   | corpus-geojson          |           6.3 |                3.3 |   **1.92x** |        1.92x |
| `identity`                   | corpus-tabular          |           3.1 |                2.7 |   **1.15x** |        2.67x |
| `identity`                   | corpus-pretty           |          16.9 |                5.5 |  **3.06x**† |        1.30x |
| `identity_compact`           | users-1mb               |          19.3 |                3.7 |  **5.21x**† |        0.85x |
| `identity_compact`           | users-10mb              |         183.9 |               14.6 | **12.64x**† |        0.23x |
| `identity_compact`           | users-100mb             |        1895.4 |              118.9 |  **15.94x** |        0.14x |
| `identity_compact`           | numbers-1mb             |          17.8 |                4.6 |  **3.83x**† |        0.73x |
| `identity_compact`           | numbers-10mb            |         159.3 |               23.1 |  **6.88x**† |        0.20x |
| `identity_compact`           | numbers-100mb           |        1599.7 |              203.5 |   **7.86x** |        0.12x |
| `identity_compact`           | strings-1mb             |          20.8 |                2.9 |  **7.19x**† |        2.29x |
| `identity_compact`           | strings-10mb            |         196.9 |                7.0 | **28.00x**† |        1.13x |
| `identity_compact`           | strings-100mb           |        2026.1 |               41.9 |  **48.36x** |        0.72x |
| `identity_compact`           | wide-1mb                |          32.9 |                5.7 |  **5.79x**† |        0.43x |
| `identity_compact`           | wide-10mb               |         322.5 |               29.3 | **11.01x**† |        0.20x |
| `identity_compact`           | wide-100mb              |        3730.2 |              262.2 |  **14.23x** |        0.12x |
| `identity_compact`           | nested-1mb              |          23.3 |                2.9 |  **8.05x**† |        1.78x |
| `identity_compact`           | nested-10mb             |         219.9 |                6.6 | **33.11x**† |        0.78x |
| `identity_compact`           | nested-100mb            |        2302.3 |               39.8 |  **57.77x** |        0.63x |
| `identity_compact`           | comprehensive-1mb       |          26.1 |               10.5 |  **2.48x**† |        0.59x |
| `identity_compact`           | comprehensive-10mb      |         244.0 |               75.7 |  **3.22x**† |        0.21x |
| `identity_compact`           | comprehensive-100mb     |        2452.6 |              750.3 |   **3.27x** |        0.16x |
| `identity_compact`           | corpus-charts           |           1.7 |                2.4 |       0.71x |        2.85x |
| `identity_compact`           | corpus-npm              |           1.8 |                2.4 |       0.74x |        2.83x |
| `identity_compact`           | corpus-graph            |           2.1 |                2.4 |       0.89x |        2.57x |
| `identity_compact`           | corpus-openapi          |           2.9 |                2.7 |   **1.06x** |        2.56x |
| `identity_compact`           | corpus-geojson          |           3.6 |                3.1 |   **1.17x** |        1.89x |
| `identity_compact`           | corpus-tabular          |           2.9 |                2.6 |   **1.11x** |        2.68x |
| `identity_compact`           | corpus-pretty           |          11.5 |                5.4 |   **2.12x** |        1.25x |
| `users_field`                | users-1mb               |           7.2 |                3.4 |   **2.09x** |        0.92x |
| `users_field`                | users-10mb              |          57.7 |               12.2 |  **4.72x**† |        0.26x |
| `users_field`                | users-100mb             |         575.5 |               96.8 |  **5.94x**† |        0.15x |
| `users_length`               | users-1mb               |           7.6 |                3.5 |   **2.14x** |        0.95x |
| `users_length`               | users-10mb              |          61.6 |               13.0 |  **4.74x**† |        0.33x |
| `users_length`               | users-100mb             |         650.4 |              101.2 |  **6.42x**† |        0.18x |
| `users_names`                | users-1mb               |           8.1 |                5.1 |   **1.59x** |        0.93x |
| `users_names`                | users-10mb              |          67.3 |               28.3 |  **2.38x**† |        0.26x |
| `users_names`                | users-100mb             |         651.4 |              258.7 |  **2.52x**† |        0.15x |
| `users_map_select`           | users-1mb               |           8.6 |                6.3 |   **1.37x** |        1.06x |
| `users_map_select`           | users-10mb              |          73.1 |               38.0 |  **1.92x**† |        0.42x |
| `users_map_select`           | users-100mb             |         727.6 |              350.8 |   **2.07x** |        0.27x |
| `users_sort_by`              | users-1mb               |          12.1 |                8.5 |   **1.43x** |        1.12x |
| `users_sort_by`              | users-10mb              |         118.9 |               61.4 |  **1.93x**† |        0.57x |
| `users_group_by`             | users-1mb               |          12.4 |                8.3 |   **1.50x** |        1.09x |
| `users_group_by`             | users-10mb              |         131.8 |               57.8 |  **2.28x**† |        0.51x |
| `users_unique_by`            | users-1mb               |          12.4 |                8.3 |   **1.50x** |        1.09x |
| `users_unique_by`            | users-10mb              |         131.0 |               59.1 |  **2.22x**† |        0.56x |
| `users_reduce`               | users-1mb               |           8.4 |                8.3 |   **1.01x** |        1.00x |
| `users_reduce`               | users-10mb              |          70.4 |               55.2 |  **1.28x**† |        0.27x |
| `users_reduce`               | users-100mb             |         692.3 |              522.1 |   **1.33x** |        0.15x |
| `users_to_entries`           | users-1mb               |           7.2 |                3.5 |   **2.05x** |        0.97x |
| `users_to_entries`           | users-10mb              |          58.0 |               12.4 |  **4.69x**† |        0.27x |
| `users_to_entries`           | users-100mb             |         574.2 |               96.8 |  **5.93x**† |        0.15x |
| `users_csv`                  | users-1mb               |          13.6 |                9.6 |   **1.41x** |        0.94x |
| `users_csv`                  | users-10mb              |         121.0 |               67.4 |  **1.79x**† |        0.26x |
| `users_csv`                  | users-100mb             |        1212.2 |              650.3 |   **1.86x** |        0.14x |
| `users_regex`                | users-1mb               |          13.4 |                6.2 |   **2.16x** |        1.08x |
| `users_regex`                | users-10mb              |         115.1 |               37.2 |  **3.10x**† |        0.43x |
| `users_ascii_downcase`       | users-1mb               |          21.5 |                6.3 |  **3.42x**† |        0.96x |
| `users_ascii_downcase`       | users-10mb              |         213.8 |               41.3 |  **5.18x**† |        0.34x |
| `users_ascii_downcase`       | users-100mb             |        2194.7 |              389.6 |   **5.63x** |        0.19x |
| `users_first_early_exit`     | users-1mb               |           7.3 |                3.3 |   **2.21x** |        0.93x |
| `users_first_early_exit`     | users-10mb              |          57.7 |               10.7 |  **5.39x**† |        0.26x |
| `users_first_early_exit`     | users-100mb             |         573.8 |               80.7 |  **7.11x**† |        0.15x |
| `users_limit_early_exit`     | users-1mb               |           7.2 |                3.3 |   **2.19x** |        0.95x |
| `users_limit_early_exit`     | users-10mb              |          57.9 |               10.6 |  **5.44x**† |        0.26x |
| `users_limit_early_exit`     | users-100mb             |         575.4 |               80.5 |  **7.14x**† |        0.15x |
| `users_assign_update`        | users-1mb               |           7.3 |                5.6 |   **1.29x** |        1.43x |
| `users_assign_update`        | users-10mb              |          58.2 |               32.6 |  **1.79x**† |        0.91x |
| `users_del`                  | users-1mb               |           7.4 |                5.6 |   **1.32x** |        1.40x |
| `users_del`                  | users-10mb              |          60.6 |               32.7 |  **1.85x**† |        0.89x |
| `users_paths`                | users-1mb               |          23.4 |               10.5 |  **2.23x**† |        1.13x |
| `users_paths`                | users-10mb              |         219.7 |               80.6 |  **2.73x**† |        0.80x |
| `users_tojson_roundtrip`     | users-1mb               |          17.6 |                7.9 |  **2.21x**† |        1.19x |
| `users_tojson_roundtrip`     | users-10mb              |         167.6 |               58.0 |  **2.89x**† |        0.76x |
| `users_walk`                 | users-1mb               |          62.6 |                7.3 |  **8.53x**† |        1.06x |
| `users_walk`                 | users-10mb              |         605.1 |               49.8 | **12.15x**† |        0.65x |
| `users_slurp`                | users-1mb               |           7.2 |                3.5 |   **2.06x** |        1.01x |
| `users_slurp`                | users-10mb              |          58.1 |               12.7 |  **4.58x**† |        0.35x |
| `users_stream`               | users-1mb               |          23.4 |               19.9 |  **1.18x**† |        3.10x |
| `users_stream`               | users-10mb              |         214.1 |              168.1 |  **1.27x**† |        3.11x |
| `numbers_add`                | numbers-1mb             |          20.8 |                7.4 |  **2.82x**† |        0.93x |
| `numbers_add`                | numbers-10mb            |         190.1 |               52.2 |  **3.64x**† |        0.26x |
| `numbers_add`                | numbers-100mb           |        1893.8 |              505.2 |   **3.75x** |        0.16x |
| `numbers_sort`               | numbers-1mb             |          35.5 |               13.8 |  **2.56x**† |        1.06x |
| `numbers_sort`               | numbers-10mb            |         416.0 |              147.4 |  **2.82x**† |        0.57x |
| `numbers_max`                | numbers-1mb             |           9.9 |                7.8 |   **1.26x** |        0.91x |
| `numbers_max`                | numbers-10mb            |          81.7 |               55.3 |  **1.48x**† |        0.25x |
| `numbers_max`                | numbers-100mb           |         819.7 |              535.5 |   **1.53x** |        0.16x |
| `numbers_map_floor`          | numbers-1mb             |          24.1 |                9.8 |  **2.45x**† |        1.20x |
| `numbers_map_floor`          | numbers-10mb            |         220.6 |               74.5 |  **2.96x**† |        0.50x |
| `numbers_map_floor`          | numbers-100mb           |        2198.6 |              730.5 |   **3.01x** |        0.38x |
| `strings_length`             | strings-1mb             |           7.5 |                3.7 |   **2.04x** |        2.37x |
| `strings_length`             | strings-10mb            |          59.9 |               14.4 |  **4.17x**† |        1.28x |
| `strings_length`             | strings-100mb           |         602.9 |              117.7 |  **5.12x**† |        0.80x |
| `strings_upcase`             | strings-1mb             |         251.1 |                7.7 | **32.72x**† |        1.98x |
| `strings_upcase`             | strings-10mb            |        2504.5 |               52.5 |  **47.71x** |        1.09x |
| `strings_upcase`             | strings-100mb           |       24911.0 |              497.9 |  **50.03x** |        0.80x |
| `strings_split`              | strings-1mb             |          11.7 |                7.5 |   **1.56x** |        2.41x |
| `strings_split`              | strings-10mb            |          99.8 |               49.5 |  **2.02x**† |        1.29x |
| `strings_test`               | strings-1mb             |          11.9 |                4.3 |   **2.79x** |        2.34x |
| `strings_test`               | strings-10mb            |          98.5 |               19.4 |  **5.07x**† |        1.29x |
| `wide_keys`                  | wide-1mb                |          27.0 |                6.7 |  **4.01x**† |        0.46x |
| `wide_keys`                  | wide-10mb               |         273.5 |               43.0 |  **6.37x**† |        0.17x |
| `wide_keys`                  | wide-100mb              |        3170.0 |              431.8 |   **7.34x** |        0.13x |
| `wide_to_entries`            | wide-1mb                |          34.9 |               23.9 |  **1.46x**† |        0.48x |
| `wide_to_entries`            | wide-10mb               |         376.5 |              199.7 |  **1.88x**† |        0.38x |
| `wide_map_values`            | wide-1mb                |          95.4 |               33.8 |  **2.82x**† |        0.60x |
| `wide_map_values`            | wide-10mb               |         894.4 |              295.4 |  **3.03x**† |        0.44x |
| `wide_add_values`            | wide-1mb                |          25.6 |               11.5 |  **2.22x**† |        0.68x |
| `wide_add_values`            | wide-10mb               |         236.0 |               85.8 |  **2.75x**† |        0.42x |
| `wide_add_values`            | wide-100mb              |        2713.3 |              817.6 |   **3.32x** |        0.30x |
| `wide_with_entries`          | wide-1mb                |         209.8 |               69.8 |  **3.01x**† |        0.57x |
| `wide_with_entries`          | wide-10mb               |        2029.1 |              638.0 |   **3.18x** |        0.47x |
| `nested_paths`               | nested-1mb              |           6.0 |                3.4 |   **1.73x** |        1.87x |
| `nested_paths`               | nested-10mb             |          44.3 |               11.0 |  **4.04x**† |        0.79x |
| `nested_recurse`             | nested-1mb              |           5.9 |                4.0 |   **1.46x** |        1.88x |
| `nested_recurse`             | nested-10mb             |          44.3 |               16.5 |  **2.68x**† |        0.80x |
| `comprehensive_paths`        | comprehensive-1mb       |          51.0 |               17.5 |  **2.92x**† |        1.10x |
| `comprehensive_paths`        | comprehensive-10mb      |         479.7 |              131.6 |  **3.64x**† |        0.80x |
| `comprehensive_tostream`     | comprehensive-1mb       |         111.6 |               18.0 |  **6.18x**† |        1.34x |
| `comprehensive_tostream`     | comprehensive-10mb      |        1085.7 |              145.5 |   **7.46x** |        1.11x |
| `comprehensive_numbers`      | comprehensive-1mb       |          41.9 |               19.7 |  **2.13x**† |        0.85x |
| `comprehensive_numbers`      | comprehensive-10mb      |         397.0 |              162.8 |  **2.44x**† |        0.40x |
| `ndjson_identity`            | ndjson-1mb              |          30.8 |                7.0 |  **4.42x**† |        2.96x |
| `ndjson_identity`            | ndjson-10mb             |         284.1 |               42.3 |  **6.71x**† |        2.95x |
| `ndjson_identity`            | ndjson-100mb            |        2906.8 |              396.5 |   **7.33x** |        2.98x |
| `ndjson_field`               | ndjson-1mb              |          13.0 |                7.8 |   **1.66x** |        2.97x |
| `ndjson_field`               | ndjson-10mb             |         108.3 |               50.6 |  **2.14x**† |        3.01x |
| `ndjson_field`               | ndjson-100mb            |        1055.4 |              469.2 |   **2.25x** |        3.04x |
| `ndjson_select`              | ndjson-1mb              |          14.2 |               10.0 |  **1.41x**† |        3.11x |
| `ndjson_select`              | ndjson-10mb             |         121.9 |               72.5 |  **1.68x**† |        3.09x |
| `ndjson_select`              | ndjson-100mb            |        1191.0 |              689.9 |   **1.73x** |        3.11x |
| `ndjson_tsv`                 | ndjson-1mb              |          20.8 |               12.3 |  **1.69x**† |        3.00x |
| `ndjson_tsv`                 | ndjson-10mb             |         187.1 |               92.5 |  **2.02x**† |        3.05x |
| `ndjson_tsv`                 | ndjson-100mb            |        1836.4 |              889.0 |   **2.07x** |        3.05x |
| `ndjson_slurp`               | ndjson-1mb              |          13.9 |                8.0 |  **1.74x**† |        0.87x |
| `ndjson_slurp`               | ndjson-10mb             |         125.0 |               54.7 |  **2.28x**† |        0.46x |
| `ndjson_inputs_reduce`       | ndjson-1mb              |          13.6 |                9.8 |   **1.39x** |        3.37x |
| `ndjson_inputs_reduce`       | ndjson-10mb             |         112.1 |               67.7 |  **1.66x**† |        3.38x |
| `ndjson_inputs_reduce`       | ndjson-100mb            |        1090.4 |              638.5 |   **1.71x** |        3.39x |
| `ndjson_inputs_first`        | ndjson-1mb              |           1.7 |                2.4 |       0.69x |        3.05x |
| `ndjson_inputs_first`        | ndjson-10mb             |           1.7 |                2.4 |       0.71x |        3.03x |
| `ndjson_inputs_first`        | ndjson-100mb            |           1.7 |                2.5 |       0.68x |        3.04x |
| `ndjson_raw_lines`           | ndjson-1mb              |           7.7 |                4.1 |   **1.86x** |        2.95x |
| `ndjson_raw_lines`           | ndjson-10mb             |          59.4 |               16.9 |  **3.52x**† |        2.90x |
| `ndjson_raw_lines`           | ndjson-100mb            |         565.7 |              143.4 |  **3.94x**† |        2.86x |
| `ndjson_raw_fromjson`        | ndjson-1mb              |          15.5 |                9.6 |  **1.60x**† |        2.95x |
| `ndjson_raw_fromjson`        | ndjson-10mb             |         131.9 |               68.5 |  **1.93x**† |        2.90x |
| `ndjson_raw_slurp`           | ndjson-1mb              |           5.6 |                2.5 |   **2.25x** |        1.80x |
| `ndjson_raw_slurp`           | ndjson-10mb             |          38.4 |                4.2 |  **9.19x**† |        0.59x |
| `ndjson_raw_slurp`           | ndjson-100mb            |         365.6 |               17.8 | **20.52x**† |        0.45x |
| `ndjson_three_files`         | ndjson-1mb              |          34.9 |               15.9 |  **2.19x**† |        2.98x |
| `ndjson_three_files`         | ndjson-10mb             |         325.0 |              130.3 |  **2.49x**† |        2.96x |
| `users_three_files`          | users-1mb               |          17.1 |                5.4 |  **3.17x**† |        0.94x |
| `users_three_files`          | users-10mb              |         163.3 |               33.3 |  **4.90x**† |        0.30x |
| `seq_identity`               | jsonseq-1mb             |          30.6 |                8.7 |  **3.51x**† |        3.04x |
| `seq_identity`               | jsonseq-10mb            |         285.5 |               60.5 |  **4.72x**† |        3.05x |
| `seq_identity`               | jsonseq-100mb           |        2979.5 |              572.4 |   **5.21x** |        3.06x |
| `seq_field`                  | jsonseq-1mb             |          13.3 |                9.5 |   **1.41x** |        3.02x |
| `seq_field`                  | jsonseq-10mb            |         108.4 |               66.0 |  **1.64x**† |        3.05x |
| `seq_field`                  | jsonseq-100mb           |        1052.3 |              620.3 |   **1.70x** |        3.07x |
| `seq_slurp`                  | jsonseq-1mb             |          13.8 |               10.2 |  **1.36x**† |        0.89x |
| `seq_slurp`                  | jsonseq-10mb            |         125.6 |               76.4 |  **1.64x**† |        0.46x |
| `users_seq_output`           | users-1mb               |           3.0 |                2.5 |   **1.22x** |        3.03x |
| `users_seq_output`           | users-10mb              |          15.8 |                3.1 |  **5.06x**† |        3.06x |
| `users_seq_records`          | users-seq-1mb           |           8.6 |                6.0 |   **1.43x** |        0.94x |
| `users_seq_records`          | users-seq-10mb          |          73.1 |               38.0 |  **1.92x**† |        0.26x |
| `seq_field_bad_first`        | jsonseq-bad-first-1mb   |          12.9 |                9.6 |   **1.35x** |        3.15x |
| `seq_field_bad_first`        | jsonseq-bad-first-10mb  |         108.4 |               65.8 |  **1.65x**† |        3.18x |
| `seq_field_bad_first`        | jsonseq-bad-first-100mb |        1053.6 |              615.6 |   **1.71x** |        3.19x |
| `seq_field_bad_last`         | jsonseq-bad-last-1mb    |          13.0 |                9.5 |   **1.37x** |        3.24x |
| `seq_field_bad_last`         | jsonseq-bad-last-10mb   |         108.1 |               65.7 |  **1.64x**† |        3.28x |
| `seq_field_bad_last`         | jsonseq-bad-last-100mb  |        1051.0 |              616.5 |   **1.70x** |        3.28x |
| `users_slurpfile`            | users-1mb               |           7.2 |                4.2 |   **1.71x** |        1.39x |
| `users_slurpfile`            | users-10mb              |          57.3 |               19.0 |  **3.01x**† |        0.86x |
| `ndjson_slurpfile`           | ndjson-1mb              |          10.3 |                5.3 |   **1.97x** |        1.21x |
| `ndjson_slurpfile`           | ndjson-10mb             |          87.8 |               26.8 |  **3.27x**† |        0.81x |
| `users_rawfile`              | users-1mb               |           3.1 |                2.5 |   **1.26x** |        1.93x |
| `users_rawfile`              | users-10mb              |          16.7 |                3.5 |  **4.82x**† |        0.82x |
| `users_rawfile`              | users-100mb             |         160.7 |               11.7 | **13.78x**† |        0.38x |
| `args_many`                  | none                    |           1.7 |                2.5 |       0.68x |        3.02x |
| `corpus_npm_deps`            | corpus-npm              |           1.7 |                2.4 |       0.70x |        3.05x |
| `corpus_npm_scripts`         | corpus-npm              |           1.7 |                2.4 |       0.71x |        3.08x |
| `corpus_charts_titles`       | corpus-charts           |           1.7 |                2.4 |       0.70x |        2.92x |
| `corpus_graph_weights`       | corpus-graph            |           1.8 |                2.4 |       0.76x |        2.66x |
| `corpus_graph_by_source`     | corpus-graph            |           1.9 |                2.6 |       0.74x |        2.70x |
| `corpus_graph_select`        | corpus-graph            |           1.8 |                2.4 |       0.75x |        2.74x |
| `corpus_openapi_definitions` | corpus-openapi          |           2.2 |                2.4 |       0.89x |        2.61x |
| `corpus_openapi_strings`     | corpus-openapi          |           2.5 |                2.8 |       0.90x |        2.66x |
| `corpus_openapi_refs`        | corpus-openapi          |           2.7 |                3.0 |       0.88x |        2.76x |
| `corpus_openapi_paths`       | corpus-openapi          |           2.6 |                2.8 |       0.92x |        2.58x |
| `corpus_geojson_districts`   | corpus-geojson          |           2.4 |                2.6 |       0.93x |        1.99x |
| `corpus_geojson_types`       | corpus-geojson          |           2.4 |                2.6 |       0.93x |        2.03x |
| `corpus_geojson_points`      | corpus-geojson          |           5.4 |                3.9 |   **1.40x** |        2.18x |
| `corpus_tabular_hours`       | corpus-tabular          |           2.3 |                2.6 |       0.86x |        2.88x |
| `corpus_tabular_csv`         | corpus-tabular          |           2.4 |                2.9 |       0.83x |        2.86x |
| `corpus_tabular_group`       | corpus-tabular          |           2.3 |                2.7 |       0.86x |        2.86x |
| `corpus_pretty_numbers`      | corpus-pretty           |          15.8 |                7.0 |  **2.27x**† |        1.37x |
| `corpus_pretty_paths`        | corpus-pretty           |          17.6 |                6.7 |  **2.64x**† |        1.37x |
| `corpus_pretty_tostream`     | corpus-pretty           |          30.9 |                7.1 |  **4.34x**† |        1.57x |
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
- **Slightly higher on small files and streams**: a fixed 0.2-2.9 MB over jq, mostly the pages of our own binary (see below)
- **Streaming output**: Uses lazy cursor evaluation - only materializes values when needed

#### Where succinctly jq still holds more memory than jq ([#7412](https://github.com/rust-works/succinctly/issues/7412))

Re-measured for [#7894](https://github.com/rust-works/succinctly/issues/7894) at main `a9da4e814` with the link-order list regenerated, on the 231 output-gated rows of the last `succinctly dev compare jq --full --huge` run for #7412 and with peak RSS read as that harness reads it (the median of three runs below 100 MB), succinctly jq's peak RSS is above jq 1.7.1's on 86 rows on an AMD Ryzen 9 7950X and on 76 rows on an Apple M4 Pro, the counts of the #7412 run at `7b7ca8880`.
Both measured a separate build with `cli,simd,regex,serde` (passed to the #7412 run with `--subject`): the features of the *Harness comparison* blocks without `bench-runner`, so neither the harness's own build that those blocks measure nor the `cli` build of the release archives ([Which binary a run measures](../guides/reproducing-the-jq-comparison.md#which-binary-a-run-measures)).
The geomean of the memory ratio is 0.626 and 0.553 (0.616 and 0.547 in the #7412 run, where jq was faster on none of the rows).
With the list as it had gone stale, the same commit read 92 and 79 rows and 0.720 and 0.578 (#7894).
The gaps that remain are accepted, for these reasons:

- **The binary's own pages.** Apart from `comprehensive_tostream`, every row above jq is over by at most 2.9 MB (7950X) or 2.2 MB (M4), and its private memory by at most 0.59 MiB (7950X) or 0.66 MiB (M4).
  The rest is file-backed pages of a 23 MB binary that the process touches (0.9-2.4 MiB on the 7950X, 0.7-1.6 MiB on the M4), against jq's 0.4 MB one.
  Link order (#7397, #7417; regenerated in #7894, whose CI check fails once the list goes stale) and releasing the relocation tables' pages after start-up (#7417, Linux) already cut it.
  Not taken:
  - packed (RELR) relocations, which need glibc 2.36;
  - a non-PIE build, which gives up ASLR;
  - ordering `.rodata`, which has no symbols to order by;
  - a static libgcc, since std's unwinder needs `libgcc_s`;
  - shrinking `.data.rel.ro`, a large refactor.
- **Stream windows.** The `ndjson_*`, `seq_*` and `users_stream` rows hold 1.0-1.35 MiB private against jq's 0.84 MiB, whatever the input size.
  Most of it is the 256 KiB input window and what is built over it (#7415).
  128 KiB windows were measured and rejected: the M4 was 5% slower on those rows.
- **`comprehensive_tostream`.** At 10 MB it is 375 MB against jq's 367 on the 7950X and 406 MB against 371 on the M4.
  Every element of every event's path is a 32-byte `OwnedValue`, where jq's `jv` is 16 bytes.
  Shrinking the value type would touch every evaluator path, for one row.

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
