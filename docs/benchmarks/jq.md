# jq vs succinctly Comparison Benchmarks

[Home](../../) > [Docs](../) > [Benchmarks](./) > jq

Comprehensive benchmarks comparing `succinctly jq .` vs `jq .` for JSON formatting/printing.

**See also**: [JsonIndex wiki page](../parsing/json-index.md) for architecture overview and academic references.

Results are provided for ARM (Apple M4 Pro, Apple M1 Max, AWS Graviton) and x86_64 (AMD Zen 4) platforms. The AMD Ryzen 9 7950X and Apple M4 Pro sections were re-run against jq 1.7.1 in #5336 and include a query mix beyond `.`; the other platforms' sections were measured against the jq version named in each, so do not compare speedups across sections whose jq version differs.

The AMD Ryzen 9 7950X and Apple M4 Pro sections open with a generated *Harness comparison* block: the pinned jq 1.7.1, interleaved repetitions and output-gated rows of `succinctly dev compare jq`, quoted from the committed run in [docs/compare/](../compare/jq.md) and checked against it by a test. Where a hand-written figure below disagrees with one of those blocks, the block is the measurement.

Every other table and figure in this page is a `bench run jq_bench` or `dev bench jq` result measured before #5409 made those runners interleave, so none of it is generated from or checked against a committed run. The M1 Max and Neoverse sections have no harness run to replace them, and the per-pattern tables cover shapes the harness does not generate (`arrays`, `literals`, `mixed`, `pathological`, `unicode`), so all of it is kept as history. For a current figure, quote a *Harness comparison* block or the table in [CLAUDE.md](../../CLAUDE.md#performance-summary).

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
- **Peak memory**: Maximum resident set size (RSS). On a file of many top-level values (NDJSON) succinctly's grows with the input (about 1.0 to 1.2 times the file for `ndjson_identity`, `ndjson_field`, `ndjson_select`, `ndjson_inputs_reduce` and `ndjson_raw_lines`; `ndjson_inputs_first` is flat) where jq's stays at a few MB, so the memory ratios of those rows grow with the size and jq is the tool for a file too large to hold: [#6913](https://github.com/rust-works/succinctly/issues/6913), measured in [jq limitations](../compliance/jq/limitations.md#peak-memory-on-a-stream-of-many-values-grows-with-the-input-6913)
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

**Run**: `./target/release/succinctly dev compare jq --full` on AMD Ryzen 9 7950X 16-Core Processor (Ubuntu 22.04.5 LTS), succinctly `968a5fa84`, run `20261110_135359_968a5fa84`. jq-1.7.1 is built from source by the harness; the tools are interleaved within each of 21 repetitions, and a row is timed only when the outputs are identical.

**Timed binary**: the harness itself, built with `cli,simd,regex,serde`.

Geomean speedup over jq-1.7.1: **2.25x** across 201 timed rows; jq-1.7.1 is faster on 25 of them. Speedup is jq-1.7.1's time over succinctly jq's, so below 1.00x jq is faster; memory is succinctly jq's peak RSS over jq-1.7.1's. Times are medians, except where marked †: both times and the ratio are then each tool's fastest run, which run-to-run noise moves least on a long row.

| Query                        | Input                   | jq-1.7.1 (ms) | succinctly jq (ms) |     Speedup | Memory ratio |
|------------------------------|-------------------------|---------------|--------------------|-------------|--------------|
| `startup`                    | none                    |           1.2 |                1.2 |       0.96x |        3.02x |
| `identity`                   | users-1mb               |          16.5 |                8.0 |  **2.07x**† |        1.22x |
| `identity`                   | users-10mb              |         158.8 |               65.5 |  **2.42x**† |        0.40x |
| `identity`                   | users-100mb             |        1626.6 |              650.6 |   **2.50x** |        0.30x |
| `identity`                   | numbers-1mb             |          21.8 |                6.6 |  **3.29x**† |        1.06x |
| `identity`                   | numbers-10mb            |         211.4 |               52.5 |  **4.02x**† |        0.38x |
| `identity`                   | numbers-100mb           |        2189.7 |              521.9 |   **4.20x** |        0.29x |
| `identity`                   | strings-1mb             |          12.0 |                4.1 |   **2.93x** |        3.06x |
| `identity`                   | strings-10mb            |         111.7 |               27.0 |  **4.14x**† |        1.91x |
| `identity`                   | strings-100mb           |        1152.1 |              257.1 |   **4.48x** |        1.57x |
| `identity`                   | wide-1mb                |          29.9 |               12.8 |  **2.33x**† |        0.70x |
| `identity`                   | wide-10mb               |         311.4 |              109.4 |  **2.85x**† |        0.30x |
| `identity`                   | wide-100mb              |        3703.6 |             1037.5 |   **3.57x** |        0.19x |
| `identity`                   | nested-1mb              |          13.4 |                4.6 |   **2.93x** |        2.24x |
| `identity`                   | nested-10mb             |         124.7 |               35.1 |  **3.56x**† |        1.36x |
| `identity`                   | nested-100mb            |        1247.4 |              337.1 |   **3.70x** |        1.12x |
| `identity`                   | comprehensive-1mb       |          30.3 |               10.6 |  **2.86x**† |        0.71x |
| `identity`                   | comprehensive-10mb      |         295.9 |               88.3 |  **3.35x**† |        0.25x |
| `identity`                   | comprehensive-100mb     |        3065.9 |              898.5 |   **3.41x** |        0.20x |
| `identity`                   | corpus-charts           |           1.2 |                1.3 |       0.92x |        2.99x |
| `identity`                   | corpus-npm              |           1.2 |                1.3 |       0.94x |        3.17x |
| `identity`                   | corpus-openapi          |           1.8 |                1.6 |   **1.13x** |        2.99x |
| `identity_compact`           | users-1mb               |          14.7 |                3.0 |  **4.87x**† |        1.07x |
| `identity_compact`           | users-10mb              |         139.6 |               18.2 |  **7.68x**† |        0.25x |
| `identity_compact`           | users-100mb             |        1442.9 |              176.7 |   **8.17x** |        0.14x |
| `identity_compact`           | numbers-1mb             |          20.1 |                3.6 |  **5.60x**† |        0.91x |
| `identity_compact`           | numbers-10mb            |         194.1 |               23.7 |  **8.20x**† |        0.23x |
| `identity_compact`           | numbers-100mb           |        2012.4 |              228.2 |   **8.82x** |        0.14x |
| `identity_compact`           | strings-1mb             |          11.7 |                3.2 |   **3.70x** |        2.72x |
| `identity_compact`           | strings-10mb            |         108.2 |               18.0 |  **6.00x**† |        1.27x |
| `identity_compact`           | strings-100mb           |        1119.7 |              172.0 |   **6.51x** |        0.87x |
| `identity_compact`           | wide-1mb                |          29.1 |                5.9 |  **4.91x**† |        0.65x |
| `identity_compact`           | wide-10mb               |         306.5 |               42.7 |  **7.18x**† |        0.24x |
| `identity_compact`           | wide-100mb              |        3641.1 |             1033.9 |   **3.52x** |        0.18x |
| `identity_compact`           | nested-1mb              |          13.3 |                3.3 |   **4.08x** |        2.04x |
| `identity_compact`           | nested-10mb             |         121.5 |               19.3 |  **6.30x**† |        0.91x |
| `identity_compact`           | nested-100mb            |        1225.8 |              180.2 |   **6.80x** |        0.63x |
| `identity_compact`           | comprehensive-1mb       |          24.4 |               10.3 |  **2.38x**† |        0.65x |
| `identity_compact`           | comprehensive-10mb      |         242.5 |               83.3 |  **2.91x**† |        0.19x |
| `identity_compact`           | comprehensive-100mb     |        2507.7 |              846.5 |   **2.96x** |        0.14x |
| `identity_compact`           | corpus-charts           |           1.2 |                1.3 |       0.91x |        3.13x |
| `identity_compact`           | corpus-npm              |           1.2 |                1.3 |       0.93x |        3.20x |
| `identity_compact`           | corpus-openapi          |           1.7 |                1.6 |   **1.07x** |        3.15x |
| `users_field`                | users-1mb               |           9.0 |                3.0 |   **2.96x** |        1.29x |
| `users_field`                | users-10mb              |          87.9 |               16.4 |  **5.36x**† |        0.29x |
| `users_field`                | users-100mb             |         928.0 |              152.9 |   **6.07x** |        0.16x |
| `users_length`               | users-1mb               |           9.2 |                3.3 |   **2.75x** |        1.36x |
| `users_length`               | users-10mb              |          93.3 |               18.3 |  **5.10x**† |        0.32x |
| `users_length`               | users-100mb             |         990.3 |              170.5 |   **5.81x** |        0.19x |
| `users_names`                | users-1mb               |           9.5 |                5.3 |   **1.80x** |        1.28x |
| `users_names`                | users-10mb              |         100.1 |               38.5 |  **2.60x**† |        0.29x |
| `users_names`                | users-100mb             |        1073.5 |              373.1 |   **2.88x** |        0.16x |
| `users_map_select`           | users-1mb               |          10.9 |                6.9 |   **1.57x** |        1.50x |
| `users_map_select`           | users-10mb              |         131.6 |               51.8 |  **2.54x**† |        0.43x |
| `users_map_select`           | users-100mb             |        1434.0 |              519.8 |   **2.76x** |        0.28x |
| `users_sort_by`              | users-1mb               |          15.1 |                9.7 |  **1.56x**† |        1.47x |
| `users_sort_by`              | users-10mb              |         186.1 |               88.4 |  **2.11x**† |        0.55x |
| `users_group_by`             | users-1mb               |          16.1 |                9.5 |  **1.70x**† |        1.44x |
| `users_group_by`             | users-10mb              |         200.8 |               86.0 |  **2.33x**† |        0.54x |
| `users_unique_by`            | users-1mb               |          16.0 |                9.7 |  **1.64x**† |        1.45x |
| `users_unique_by`            | users-10mb              |         202.2 |               87.4 |  **2.31x**† |        0.56x |
| `users_reduce`               | users-1mb               |          10.3 |                8.2 |   **1.25x** |        1.56x |
| `users_reduce`               | users-10mb              |         105.5 |               66.6 |  **1.58x**† |        0.32x |
| `users_reduce`               | users-100mb             |        1094.5 |              654.4 |   **1.67x** |        0.16x |
| `users_to_entries`           | users-1mb               |           8.7 |                3.3 |   **2.68x** |        1.59x |
| `users_to_entries`           | users-10mb              |          88.7 |               16.7 |  **5.31x**† |        0.32x |
| `users_to_entries`           | users-100mb             |         930.0 |              154.2 |   **6.03x** |        0.16x |
| `users_csv`                  | users-1mb               |          15.3 |               11.7 |  **1.30x**† |        1.35x |
| `users_csv`                  | users-10mb              |         148.4 |              104.2 |  **1.42x**† |        0.29x |
| `users_csv`                  | users-100mb             |        1542.6 |             1034.7 |   **1.49x** |        0.15x |
| `users_regex`                | users-1mb               |          15.1 |                6.6 |  **2.28x**† |        1.69x |
| `users_regex`                | users-10mb              |         156.4 |               49.8 |  **3.14x**† |        0.45x |
| `users_ascii_downcase`       | users-1mb               |          26.0 |                7.3 |  **3.56x**† |        1.49x |
| `users_ascii_downcase`       | users-10mb              |         289.4 |               60.0 |  **4.83x**† |        0.43x |
| `users_ascii_downcase`       | users-100mb             |        3131.0 |              595.2 |   **5.26x** |        0.30x |
| `users_first_early_exit`     | users-1mb               |           9.0 |                3.0 |   **3.03x** |        1.36x |
| `users_first_early_exit`     | users-10mb              |          75.8 |               14.7 |  **5.15x**† |        0.30x |
| `users_first_early_exit`     | users-100mb             |         782.3 |              134.3 |   **5.83x** |        0.16x |
| `users_limit_early_exit`     | users-1mb               |           8.8 |                3.0 |   **2.94x** |        1.42x |
| `users_limit_early_exit`     | users-10mb              |          87.9 |               14.8 |  **5.93x**† |        0.30x |
| `users_limit_early_exit`     | users-100mb             |         932.2 |              135.0 |   **6.90x** |        0.16x |
| `users_assign_update`        | users-1mb               |           8.9 |                9.3 |       0.95x |        2.23x |
| `users_assign_update`        | users-10mb              |          89.7 |               71.3 |  **1.26x**† |        1.39x |
| `users_del`                  | users-1mb               |           9.3 |                9.3 |       1.00x |        2.20x |
| `users_del`                  | users-10mb              |          94.3 |               71.7 |  **1.32x**† |        1.30x |
| `users_paths`                | users-1mb               |          32.8 |               17.6 |  **1.86x**† |        1.71x |
| `users_paths`                | users-10mb              |         349.8 |              151.8 |  **2.30x**† |        1.15x |
| `users_tojson_roundtrip`     | users-1mb               |          21.8 |               13.5 |  **1.62x**† |        1.87x |
| `users_tojson_roundtrip`     | users-10mb              |         239.0 |              138.8 |  **1.72x**† |        1.20x |
| `users_walk`                 | users-1mb               |          76.5 |               18.1 |  **4.22x**† |        2.31x |
| `users_walk`                 | users-10mb              |         769.0 |              164.4 |  **4.68x**† |        1.76x |
| `users_slurp`                | users-1mb               |           9.1 |                3.6 |   **2.53x** |        1.38x |
| `users_slurp`                | users-10mb              |          90.0 |               21.3 |  **4.22x**† |        0.35x |
| `users_stream`               | users-1mb               |          26.4 |               22.5 |  **1.17x**† |        4.00x |
| `users_stream`               | users-10mb              |         253.2 |              212.9 |  **1.19x**† |        4.05x |
| `numbers_add`                | numbers-1mb             |          27.0 |               10.8 |  **2.51x**† |        1.67x |
| `numbers_add`                | numbers-10mb            |         266.7 |               94.5 |  **2.82x**† |        0.77x |
| `numbers_add`                | numbers-100mb           |        2762.4 |              943.6 |   **2.93x** |        0.63x |
| `numbers_sort`               | numbers-1mb             |          44.7 |               29.5 |  **1.52x**† |        1.86x |
| `numbers_sort`               | numbers-10mb            |         873.3 |              329.5 |  **2.65x**† |        1.13x |
| `numbers_max`                | numbers-1mb             |          12.6 |               13.5 |       0.94x |        2.46x |
| `numbers_max`                | numbers-10mb            |         117.3 |              118.0 |      0.99x† |        1.91x |
| `numbers_max`                | numbers-100mb           |        1259.9 |             1182.7 |   **1.07x** |        1.82x |
| `numbers_map_floor`          | numbers-1mb             |          32.8 |               15.4 |  **2.12x**† |        1.86x |
| `numbers_map_floor`          | numbers-10mb            |         319.4 |              142.4 |  **2.24x**† |        1.23x |
| `numbers_map_floor`          | numbers-100mb           |        3310.6 |             1401.4 |   **2.36x** |        1.25x |
| `strings_length`             | strings-1mb             |           7.8 |                4.5 |   **1.73x** |        3.43x |
| `strings_length`             | strings-10mb            |          68.7 |               30.9 |  **2.22x**† |        1.65x |
| `strings_length`             | strings-100mb           |         723.9 |              300.6 |   **2.41x** |        1.21x |
| `strings_upcase`             | strings-1mb             |         323.4 |                9.3 | **34.84x**† |        3.07x |
| `strings_upcase`             | strings-10mb            |        3324.2 |               76.6 |  **43.37x** |        1.43x |
| `strings_upcase`             | strings-100mb           |       33473.6 |              741.9 |  **45.12x** |        1.12x |
| `strings_split`              | strings-1mb             |          11.9 |                7.9 |   **1.51x** |        3.32x |
| `strings_split`              | strings-10mb            |         109.8 |               63.3 |  **1.73x**† |        1.64x |
| `strings_test`               | strings-1mb             |          12.1 |                5.1 |   **2.37x** |        3.49x |
| `strings_test`               | strings-10mb            |         111.4 |               35.9 |  **3.11x**† |        1.72x |
| `wide_keys`                  | wide-1mb                |          29.5 |                7.4 |  **3.99x**† |        0.66x |
| `wide_keys`                  | wide-10mb               |         316.2 |               56.7 |  **5.58x**† |        0.21x |
| `wide_keys`                  | wide-100mb              |        3977.9 |              570.5 |   **6.97x** |        0.14x |
| `wide_to_entries`            | wide-1mb                |          59.8 |               53.9 |  **1.11x**† |        1.28x |
| `wide_to_entries`            | wide-10mb               |         679.4 |              510.3 |  **1.33x**† |        1.18x |
| `wide_map_values`            | wide-1mb                |         124.3 |               50.8 |  **2.45x**† |        2.10x |
| `wide_map_values`            | wide-10mb               |        1262.5 |              541.4 |   **2.33x** |        1.93x |
| `wide_add_values`            | wide-1mb                |          37.2 |               32.6 |  **1.14x**† |        1.49x |
| `wide_add_values`            | wide-10mb               |         382.9 |              287.4 |  **1.33x**† |        1.13x |
| `wide_add_values`            | wide-100mb              |        4358.7 |             2589.1 |   **1.68x** |        1.16x |
| `wide_with_entries`          | wide-1mb                |         297.5 |              145.3 |  **2.05x**† |        1.42x |
| `wide_with_entries`          | wide-10mb               |        3036.4 |             1474.2 |   **2.06x** |        1.24x |
| `nested_paths`               | nested-1mb              |           6.0 |                3.8 |   **1.59x** |        2.33x |
| `nested_paths`               | nested-10mb             |          50.0 |               23.9 |  **2.09x**† |        0.99x |
| `nested_recurse`             | nested-1mb              |           6.0 |                4.4 |   **1.35x** |        2.37x |
| `nested_recurse`             | nested-10mb             |          50.3 |               29.7 |  **1.69x**† |        0.98x |
| `comprehensive_paths`        | comprehensive-1mb       |          77.3 |               37.8 |  **2.04x**† |        1.43x |
| `comprehensive_paths`        | comprehensive-10mb      |         799.5 |              386.0 |  **2.07x**† |        1.21x |
| `comprehensive_tostream`     | comprehensive-1mb       |         157.4 |               63.0 |  **2.50x**† |        1.88x |
| `comprehensive_tostream`     | comprehensive-10mb      |        1651.8 |              628.4 |   **2.63x** |        1.72x |
| `comprehensive_numbers`      | comprehensive-1mb       |          57.4 |               28.0 |  **2.05x**† |        1.83x |
| `comprehensive_numbers`      | comprehensive-10mb      |         532.6 |              267.7 |  **1.99x**† |        1.44x |
| `ndjson_identity`            | ndjson-1mb              |          19.8 |                6.6 |  **3.01x**† |        3.44x |
| `ndjson_identity`            | ndjson-10mb             |         184.9 |               53.9 |  **3.43x**† |        6.55x |
| `ndjson_identity`            | ndjson-100mb            |        1833.7 |              527.0 |   **3.48x** |       40.26x |
| `ndjson_field`               | ndjson-1mb              |          11.0 |                8.6 |   **1.29x** |        3.48x |
| `ndjson_field`               | ndjson-10mb             |          96.6 |               71.3 |  **1.35x**† |        7.04x |
| `ndjson_field`               | ndjson-100mb            |         962.3 |              694.9 |   **1.38x** |       40.42x |
| `ndjson_select`              | ndjson-1mb              |          13.1 |               13.3 |       0.98x |        4.19x |
| `ndjson_select`              | ndjson-10mb             |         117.4 |              115.8 |  **1.01x**† |        7.49x |
| `ndjson_select`              | ndjson-100mb            |        1154.7 |             1128.3 |   **1.02x** |       40.75x |
| `ndjson_tsv`                 | ndjson-1mb              |          20.5 |               20.0 |  **1.03x**† |        3.94x |
| `ndjson_tsv`                 | ndjson-10mb             |         192.5 |              185.0 |  **1.04x**† |        7.24x |
| `ndjson_tsv`                 | ndjson-100mb            |        1913.9 |             1805.1 |   **1.06x** |       42.86x |
| `ndjson_slurp`               | ndjson-1mb              |          16.8 |                9.7 |  **1.73x**† |        1.23x |
| `ndjson_slurp`               | ndjson-10mb             |         188.3 |               79.4 |  **2.37x**† |        0.40x |
| `ndjson_inputs_reduce`       | ndjson-1mb              |          12.4 |               11.2 |   **1.10x** |        4.50x |
| `ndjson_inputs_reduce`       | ndjson-10mb             |         109.4 |               93.4 |  **1.17x**† |        7.54x |
| `ndjson_inputs_reduce`       | ndjson-100mb            |        1081.7 |              908.7 |   **1.19x** |       37.62x |
| `ndjson_inputs_first`        | ndjson-1mb              |           1.2 |                1.6 |       0.76x |        3.86x |
| `ndjson_inputs_first`        | ndjson-10mb             |           1.2 |                1.6 |       0.75x |        4.10x |
| `ndjson_inputs_first`        | ndjson-100mb            |           1.2 |                1.6 |       0.74x |        3.88x |
| `ndjson_raw_lines`           | ndjson-1mb              |           6.4 |                3.5 |   **1.83x** |        3.23x |
| `ndjson_raw_lines`           | ndjson-10mb             |          55.5 |               22.3 |  **2.49x**† |        6.29x |
| `ndjson_raw_lines`           | ndjson-100mb            |         522.1 |              208.5 |  **2.50x**† |       34.51x |
| `ndjson_raw_fromjson`        | ndjson-1mb              |          13.8 |               11.4 |   **1.21x** |        3.36x |
| `ndjson_raw_fromjson`        | ndjson-10mb             |         123.7 |               97.8 |  **1.26x**† |        6.51x |
| `ndjson_raw_slurp`           | ndjson-1mb              |           5.0 |                2.2 |   **2.30x** |        2.54x |
| `ndjson_raw_slurp`           | ndjson-10mb             |          45.4 |                9.7 |  **4.69x**† |        1.41x |
| `ndjson_raw_slurp`           | ndjson-100mb            |         401.2 |               84.3 |  **4.76x**† |        1.51x |
| `ndjson_three_files`         | ndjson-1mb              |          31.6 |               22.0 |  **1.43x**† |        4.15x |
| `ndjson_three_files`         | ndjson-10mb             |         304.2 |              208.3 |  **1.46x**† |       13.13x |
| `users_three_files`          | users-1mb               |          19.6 |                5.9 |  **3.33x**† |        1.45x |
| `users_three_files`          | users-10mb              |         223.1 |               47.7 |  **4.67x**† |        0.50x |
| `seq_identity`               | jsonseq-1mb             |          19.8 |                8.2 |  **2.41x**† |        3.33x |
| `seq_identity`               | jsonseq-10mb            |         185.9 |               71.3 |  **2.61x**† |        6.67x |
| `seq_identity`               | jsonseq-100mb           |        1838.5 |              704.6 |   **2.61x** |       39.60x |
| `seq_field`                  | jsonseq-1mb             |          11.2 |               10.0 |   **1.13x** |        3.37x |
| `seq_field`                  | jsonseq-10mb            |          98.6 |               85.1 |  **1.16x**† |        6.67x |
| `seq_field`                  | jsonseq-100mb           |         974.5 |              834.6 |   **1.17x** |       42.41x |
| `seq_slurp`                  | jsonseq-1mb             |          17.2 |               11.9 |  **1.44x**† |        1.23x |
| `seq_slurp`                  | jsonseq-10mb            |         189.1 |              100.9 |  **1.87x**† |        0.40x |
| `users_seq_output`           | users-1mb               |           2.4 |                3.4 |       0.71x |        3.15x |
| `users_seq_output`           | users-10mb              |          12.5 |               21.5 |      0.58x† |        4.95x |
| `seq_field_bad_first`        | jsonseq-bad-first-1mb   |          11.3 |               12.0 |       0.94x |        3.43x |
| `seq_field_bad_first`        | jsonseq-bad-first-10mb  |          98.1 |              104.4 |      0.94x† |        7.22x |
| `seq_field_bad_first`        | jsonseq-bad-first-100mb |         976.9 |             1039.5 |       0.94x |       49.46x |
| `seq_field_bad_last`         | jsonseq-bad-last-1mb    |          11.2 |               12.5 |       0.90x |        3.59x |
| `seq_field_bad_last`         | jsonseq-bad-last-10mb   |          98.1 |              109.2 |      0.90x† |        7.27x |
| `seq_field_bad_last`         | jsonseq-bad-last-100mb  |         981.5 |             1082.1 |       0.91x |       49.39x |
| `users_slurpfile`            | users-1mb               |           9.0 |                6.6 |   **1.36x** |        1.87x |
| `users_slurpfile`            | users-10mb              |          87.9 |               50.6 |  **1.74x**† |        1.25x |
| `ndjson_slurpfile`           | ndjson-1mb              |          13.6 |               10.5 |   **1.30x** |        1.74x |
| `ndjson_slurpfile`           | ndjson-10mb             |         136.7 |               83.9 |  **1.63x**† |        1.30x |
| `users_rawfile`              | users-1mb               |           3.3 |                1.7 |   **1.95x** |        3.05x |
| `users_rawfile`              | users-10mb              |          22.5 |                4.3 |  **5.22x**† |        1.69x |
| `users_rawfile`              | users-100mb             |         244.4 |               33.3 |  **7.33x**† |        0.60x |
| `args_many`                  | none                    |           1.2 |                1.4 |       0.87x |        3.60x |
| `corpus_npm_deps`            | corpus-npm              |           1.2 |                1.4 |       0.85x |        4.07x |
| `corpus_npm_scripts`         | corpus-npm              |           1.2 |                1.4 |       0.84x |        3.87x |
| `corpus_charts_titles`       | corpus-charts           |           1.2 |                1.3 |       0.88x |        3.24x |
| `corpus_openapi_definitions` | corpus-openapi          |           1.5 |                1.4 |   **1.05x** |        3.37x |
| `corpus_openapi_strings`     | corpus-openapi          |           1.9 |                1.9 |   **1.04x** |        3.99x |
| `corpus_openapi_refs`        | corpus-openapi          |           2.2 |                2.2 |       0.99x |        4.40x |
| `corpus_openapi_paths`       | corpus-openapi          |           2.1 |                1.9 |   **1.08x** |        3.87x |
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
**Method**: `bench run jq_bench`, measured before #5409 made it interleave (all of jq's runs, then all of succinctly's, against the `jq` named above). Historical: no `dev compare jq` run exists for this platform, so it has no generated *Harness comparison* block.

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

**Run**: `./target/release/succinctly dev compare jq --full` on Apple M4 Pro (macOS 26.5.1), succinctly `968a5fa84`, run `20261110_134739_968a5fa84`. jq-1.7.1 is built from source by the harness; the tools are interleaved within each of 21 repetitions, and a row is timed only when the outputs are identical.

**Timed binary**: the harness itself, built with `cli,simd,regex,serde`.

Geomean speedup over jq-1.7.1: **2.51x** across 201 timed rows; jq-1.7.1 is faster on 25 of them. Speedup is jq-1.7.1's time over succinctly jq's, so below 1.00x jq is faster; memory is succinctly jq's peak RSS over jq-1.7.1's. Times are medians, except where marked †: both times and the ratio are then each tool's fastest run, which run-to-run noise moves least on a long row.

| Query                        | Input                   | jq-1.7.1 (ms) | succinctly jq (ms) |     Speedup | Memory ratio |
|------------------------------|-------------------------|---------------|--------------------|-------------|--------------|
| `startup`                    | none                    |           1.8 |                2.8 |       0.66x |        3.71x |
| `identity`                   | users-1mb               |          25.1 |                8.3 |  **3.02x**† |        1.22x |
| `identity`                   | users-10mb              |         238.7 |               56.1 |  **4.25x**† |        0.45x |
| `identity`                   | users-100mb             |        2551.0 |              535.1 |   **4.77x** |        0.30x |
| `identity`                   | numbers-1mb             |          22.8 |                7.6 |  **3.01x**† |        1.05x |
| `identity`                   | numbers-10mb            |         211.7 |               47.7 |  **4.44x**† |        0.37x |
| `identity`                   | numbers-100mb           |        2143.3 |              446.0 |   **4.81x** |        0.25x |
| `identity`                   | strings-1mb             |          21.6 |                3.8 |  **5.74x**† |        3.20x |
| `identity`                   | strings-10mb            |         201.8 |               13.8 | **14.62x**† |        1.88x |
| `identity`                   | strings-100mb           |        2087.9 |              118.2 |  **17.66x** |        1.30x |
| `identity`                   | wide-1mb                |          36.6 |               11.1 |  **3.31x**† |        0.52x |
| `identity`                   | wide-10mb               |         345.1 |               78.8 |  **4.38x**† |        0.24x |
| `identity`                   | wide-100mb              |        3904.7 |              779.1 |   **5.01x** |        0.21x |
| `identity`                   | nested-1mb              |          23.7 |                4.0 |  **5.91x**† |        2.49x |
| `identity`                   | nested-10mb             |         223.0 |               16.6 | **13.44x**† |        1.27x |
| `identity`                   | nested-100mb            |        2274.3 |              163.9 |  **13.88x** |        1.12x |
| `identity`                   | comprehensive-1mb       |          42.0 |               11.2 |  **3.75x**† |        0.78x |
| `identity`                   | comprehensive-10mb      |         406.0 |               78.1 |  **5.20x**† |        0.30x |
| `identity`                   | comprehensive-100mb     |        4050.8 |              763.1 |   **5.31x** |        0.23x |
| `identity`                   | corpus-charts           |           1.8 |                2.7 |       0.66x |        3.71x |
| `identity`                   | corpus-npm              |           1.8 |                2.6 |       0.69x |        3.70x |
| `identity`                   | corpus-openapi          |           3.0 |                3.0 |   **1.01x** |        3.33x |
| `identity_compact`           | users-1mb               |          19.1 |                3.9 |  **4.87x**† |        1.08x |
| `identity_compact`           | users-10mb              |         185.7 |               15.4 | **12.09x**† |        0.25x |
| `identity_compact`           | users-100mb             |        1890.0 |              127.6 |  **14.81x** |        0.14x |
| `identity_compact`           | numbers-1mb             |          17.5 |                4.8 |  **3.63x**† |        0.92x |
| `identity_compact`           | numbers-10mb            |         158.5 |               25.5 |  **6.23x**† |        0.22x |
| `identity_compact`           | numbers-100mb           |        1603.3 |              225.1 |   **7.12x** |        0.12x |
| `identity_compact`           | strings-1mb             |          21.2 |                3.0 |  **7.03x**† |        2.91x |
| `identity_compact`           | strings-10mb            |         199.9 |                7.5 | **26.67x**† |        1.24x |
| `identity_compact`           | strings-100mb           |        2051.6 |               47.6 |  **43.08x** |        0.72x |
| `identity_compact`           | wide-1mb                |          33.0 |                5.7 |  **5.75x**† |        0.52x |
| `identity_compact`           | wide-10mb               |         313.7 |               29.9 | **10.48x**† |        0.21x |
| `identity_compact`           | wide-100mb              |        3601.1 |              770.2 |   **4.68x** |        0.19x |
| `identity_compact`           | nested-1mb              |          23.2 |                3.0 |  **7.76x**† |        2.25x |
| `identity_compact`           | nested-10mb             |         221.6 |                7.4 | **30.15x**† |        0.86x |
| `identity_compact`           | nested-100mb            |        2241.4 |               46.0 |  **48.69x** |        0.63x |
| `identity_compact`           | comprehensive-1mb       |          26.1 |               10.5 |  **2.49x**† |        0.71x |
| `identity_compact`           | comprehensive-10mb      |         245.0 |               74.3 |  **3.30x**† |        0.23x |
| `identity_compact`           | comprehensive-100mb     |        2461.0 |              723.0 |   **3.40x** |        0.15x |
| `identity_compact`           | corpus-charts           |           1.8 |                2.7 |       0.66x |        3.72x |
| `identity_compact`           | corpus-npm              |           1.8 |                2.6 |       0.68x |        3.71x |
| `identity_compact`           | corpus-openapi          |           2.9 |                2.9 |       0.98x |        3.34x |
| `users_field`                | users-1mb               |           7.2 |                3.7 |   **1.93x** |        1.20x |
| `users_field`                | users-10mb              |          57.8 |               12.7 |  **4.54x**† |        0.28x |
| `users_field`                | users-100mb             |         580.2 |              100.3 |  **5.78x**† |        0.15x |
| `users_length`               | users-1mb               |           7.6 |                4.0 |   **1.93x** |        1.24x |
| `users_length`               | users-10mb              |          61.5 |               13.5 |  **4.57x**† |        0.36x |
| `users_length`               | users-100mb             |         648.4 |              104.5 |  **6.21x**† |        0.19x |
| `users_names`                | users-1mb               |           8.0 |                5.9 |   **1.35x** |        1.20x |
| `users_names`                | users-10mb              |          67.9 |               34.0 |  **1.99x**† |        0.28x |
| `users_names`                | users-100mb             |         659.2 |              321.9 |   **2.05x** |        0.15x |
| `users_map_select`           | users-1mb               |           8.6 |                7.1 |   **1.21x** |        1.36x |
| `users_map_select`           | users-10mb              |          72.2 |               43.3 |  **1.67x**† |        0.46x |
| `users_map_select`           | users-100mb             |         728.3 |              409.5 |   **1.78x** |        0.28x |
| `users_sort_by`              | users-1mb               |          11.9 |                9.3 |   **1.28x** |        1.40x |
| `users_sort_by`              | users-10mb              |         117.5 |               65.6 |  **1.79x**† |        0.60x |
| `users_group_by`             | users-1mb               |          12.3 |                9.2 |   **1.34x** |        1.35x |
| `users_group_by`             | users-10mb              |         131.6 |               63.6 |  **2.07x**† |        0.54x |
| `users_unique_by`            | users-1mb               |          12.4 |                9.2 |   **1.34x** |        1.37x |
| `users_unique_by`            | users-10mb              |         131.3 |               64.9 |  **2.02x**† |        0.59x |
| `users_reduce`               | users-1mb               |           8.3 |                8.6 |       0.96x |        1.30x |
| `users_reduce`               | users-10mb              |          69.5 |               55.4 |  **1.25x**† |        0.30x |
| `users_reduce`               | users-100mb             |         693.8 |              529.8 |   **1.31x** |        0.15x |
| `users_to_entries`           | users-1mb               |           7.3 |                3.8 |   **1.93x** |        1.30x |
| `users_to_entries`           | users-10mb              |          57.9 |               12.8 |  **4.54x**† |        0.30x |
| `users_to_entries`           | users-100mb             |         574.4 |               99.6 |  **5.77x**† |        0.15x |
| `users_csv`                  | users-1mb               |          13.4 |               13.4 |      1.00x† |        1.21x |
| `users_csv`                  | users-10mb              |         120.3 |              102.3 |  **1.17x**† |        0.28x |
| `users_csv`                  | users-100mb             |        1207.9 |              995.0 |   **1.21x** |        0.15x |
| `users_regex`                | users-1mb               |          13.3 |                6.9 |   **1.91x** |        1.39x |
| `users_regex`                | users-10mb              |         115.0 |               42.1 |  **2.73x**† |        0.47x |
| `users_ascii_downcase`       | users-1mb               |          21.3 |                7.1 |  **3.00x**† |        1.34x |
| `users_ascii_downcase`       | users-10mb              |         213.7 |               46.9 |  **4.56x**† |        0.45x |
| `users_ascii_downcase`       | users-100mb             |        2193.5 |              446.9 |   **4.91x** |        0.31x |
| `users_first_early_exit`     | users-1mb               |           7.3 |                3.6 |   **2.02x** |        1.22x |
| `users_first_early_exit`     | users-10mb              |          57.9 |               11.2 |  **5.18x**† |        0.29x |
| `users_first_early_exit`     | users-100mb             |         572.7 |               84.2 |  **6.80x**† |        0.15x |
| `users_limit_early_exit`     | users-1mb               |           7.2 |                3.6 |   **2.02x** |        1.25x |
| `users_limit_early_exit`     | users-10mb              |          57.0 |               11.1 |  **5.14x**† |        0.29x |
| `users_limit_early_exit`     | users-100mb             |         573.1 |               84.3 |  **6.80x**† |        0.15x |
| `users_assign_update`        | users-1mb               |           7.2 |                6.9 |   **1.05x** |        1.91x |
| `users_assign_update`        | users-10mb              |          57.4 |               42.7 |  **1.34x**† |        1.21x |
| `users_del`                  | users-1mb               |           7.4 |                6.8 |   **1.09x** |        1.86x |
| `users_del`                  | users-10mb              |          59.7 |               42.4 |  **1.41x**† |        1.15x |
| `users_paths`                | users-1mb               |          23.3 |               12.1 |  **1.92x**† |        1.42x |
| `users_paths`                | users-10mb              |         219.3 |               92.2 |  **2.38x**† |        0.93x |
| `users_tojson_roundtrip`     | users-1mb               |          17.3 |                9.8 |  **1.77x**† |        1.53x |
| `users_tojson_roundtrip`     | users-10mb              |         165.5 |               75.0 |  **2.21x**† |        0.96x |
| `users_walk`                 | users-1mb               |          62.1 |               12.2 |  **5.09x**† |        1.92x |
| `users_walk`                 | users-10mb              |         598.1 |               93.6 |  **6.39x**† |        1.41x |
| `users_slurp`                | users-1mb               |           7.3 |                3.9 |   **1.89x** |        1.30x |
| `users_slurp`                | users-10mb              |          57.2 |               13.3 |  **4.31x**† |        0.39x |
| `users_stream`               | users-1mb               |          23.1 |               20.7 |  **1.12x**† |        4.13x |
| `users_stream`               | users-10mb              |         212.5 |              177.0 |  **1.20x**† |        4.12x |
| `numbers_add`                | numbers-1mb             |          20.6 |               11.2 |  **1.84x**† |        1.66x |
| `numbers_add`                | numbers-10mb            |         188.1 |               86.4 |  **2.18x**† |        0.60x |
| `numbers_add`                | numbers-100mb           |        1892.7 |              840.4 |   **2.25x** |        0.44x |
| `numbers_sort`               | numbers-1mb             |          35.3 |               24.0 |  **1.47x**† |        2.07x |
| `numbers_sort`               | numbers-10mb            |         412.9 |              236.9 |  **1.74x**† |        0.97x |
| `numbers_max`                | numbers-1mb             |           9.9 |               11.5 |       0.86x |        2.22x |
| `numbers_max`                | numbers-10mb            |          80.6 |               86.7 |      0.93x† |        1.31x |
| `numbers_max`                | numbers-100mb           |         824.2 |              860.9 |       0.96x |        1.13x |
| `numbers_map_floor`          | numbers-1mb             |          23.5 |               12.2 |  **1.93x**† |        1.95x |
| `numbers_map_floor`          | numbers-10mb            |         218.9 |               96.2 |  **2.28x**† |        1.40x |
| `numbers_map_floor`          | numbers-100mb           |        2206.1 |              951.2 |   **2.32x** |        1.24x |
| `strings_length`             | strings-1mb             |           7.5 |                4.0 |   **1.86x** |        3.25x |
| `strings_length`             | strings-10mb            |          58.8 |               15.4 |  **3.83x**† |        1.76x |
| `strings_length`             | strings-100mb           |         595.2 |              124.6 |  **4.78x**† |        1.29x |
| `strings_upcase`             | strings-1mb             |         249.3 |                7.8 | **31.87x**† |        2.68x |
| `strings_upcase`             | strings-10mb            |        2487.9 |               52.7 |  **47.18x** |        1.33x |
| `strings_upcase`             | strings-100mb           |       24691.5 |              479.5 |  **51.49x** |        1.09x |
| `strings_split`              | strings-1mb             |          11.6 |                7.8 |   **1.48x** |        3.28x |
| `strings_split`              | strings-10mb            |          99.5 |               51.0 |  **1.95x**† |        1.77x |
| `strings_test`               | strings-1mb             |          11.8 |                4.5 |   **2.59x** |        3.20x |
| `strings_test`               | strings-10mb            |          98.0 |               19.7 |  **4.97x**† |        1.76x |
| `wide_keys`                  | wide-1mb                |          26.3 |                6.9 |  **3.79x**† |        0.57x |
| `wide_keys`                  | wide-10mb               |         268.4 |               41.6 |  **6.45x**† |        0.19x |
| `wide_keys`                  | wide-100mb              |        3167.3 |              414.4 |   **7.64x** |        0.13x |
| `wide_to_entries`            | wide-1mb                |          35.1 |               29.6 |  **1.19x**† |        1.02x |
| `wide_to_entries`            | wide-10mb               |         379.5 |              248.0 |  **1.53x**† |        0.88x |
| `wide_map_values`            | wide-1mb                |          94.6 |               32.1 |  **2.94x**† |        1.53x |
| `wide_map_values`            | wide-10mb               |         889.5 |              288.8 |  **3.08x**† |        1.56x |
| `wide_add_values`            | wide-1mb                |          25.6 |               20.7 |  **1.24x**† |        1.66x |
| `wide_add_values`            | wide-10mb               |         234.4 |              165.6 |  **1.42x**† |        1.22x |
| `wide_add_values`            | wide-100mb              |        2752.5 |             1597.2 |   **1.72x** |        1.28x |
| `wide_with_entries`          | wide-1mb                |         209.4 |               82.2 |  **2.55x**† |        1.04x |
| `wide_with_entries`          | wide-10mb               |        2036.7 |              775.5 |   **2.63x** |        0.95x |
| `nested_paths`               | nested-1mb              |           5.9 |                3.7 |   **1.61x** |        2.45x |
| `nested_paths`               | nested-10mb             |          43.5 |               11.3 |  **3.86x**† |        0.89x |
| `nested_recurse`             | nested-1mb              |           5.9 |                4.4 |   **1.34x** |        2.44x |
| `nested_recurse`             | nested-10mb             |          43.6 |               16.7 |  **2.61x**† |        0.89x |
| `comprehensive_paths`        | comprehensive-1mb       |          51.1 |               22.8 |  **2.24x**† |        1.34x |
| `comprehensive_paths`        | comprehensive-10mb      |         480.0 |              184.6 |  **2.60x**† |        0.97x |
| `comprehensive_tostream`     | comprehensive-1mb       |         111.6 |               29.8 |  **3.75x**† |        1.71x |
| `comprehensive_tostream`     | comprehensive-10mb      |        1086.9 |              262.4 |   **4.14x** |        1.43x |
| `comprehensive_numbers`      | comprehensive-1mb       |          41.9 |               21.4 |  **1.96x**† |        1.98x |
| `comprehensive_numbers`      | comprehensive-10mb      |         394.4 |              177.4 |  **2.22x**† |        1.57x |
| `ndjson_identity`            | ndjson-1mb              |          30.6 |                7.0 |  **4.35x**† |        4.04x |
| `ndjson_identity`            | ndjson-10mb             |         282.3 |               44.0 |  **6.41x**† |        7.92x |
| `ndjson_identity`            | ndjson-100mb            |        2895.2 |              405.5 |   **7.14x** |       47.94x |
| `ndjson_field`               | ndjson-1mb              |          12.8 |                9.2 |   **1.40x** |        4.11x |
| `ndjson_field`               | ndjson-10mb             |         106.8 |               61.8 |  **1.73x**† |        8.10x |
| `ndjson_field`               | ndjson-100mb            |        1032.7 |              581.6 |   **1.78x** |       48.30x |
| `ndjson_select`              | ndjson-1mb              |          13.9 |               11.7 |  **1.20x**† |        4.35x |
| `ndjson_select`              | ndjson-10mb             |         120.3 |               89.2 |  **1.35x**† |        8.22x |
| `ndjson_select`              | ndjson-100mb            |        1171.2 |              848.0 |   **1.38x** |       47.72x |
| `ndjson_tsv`                 | ndjson-1mb              |          20.4 |               18.6 |  **1.10x**† |        4.20x |
| `ndjson_tsv`                 | ndjson-10mb             |         183.8 |              151.1 |  **1.22x**† |        8.08x |
| `ndjson_tsv`                 | ndjson-100mb            |        1808.7 |             1465.1 |   **1.23x** |       47.13x |
| `ndjson_slurp`               | ndjson-1mb              |          14.1 |                9.2 |   **1.53x** |        1.11x |
| `ndjson_slurp`               | ndjson-10mb             |         123.8 |               62.8 |  **1.97x**† |        0.49x |
| `ndjson_inputs_reduce`       | ndjson-1mb              |          13.4 |               12.7 |   **1.05x** |        4.78x |
| `ndjson_inputs_reduce`       | ndjson-10mb             |         110.6 |               93.3 |  **1.19x**† |        8.23x |
| `ndjson_inputs_reduce`       | ndjson-100mb            |        1072.2 |              888.8 |   **1.21x** |       42.54x |
| `ndjson_inputs_first`        | ndjson-1mb              |           1.8 |                2.9 |       0.61x |        4.16x |
| `ndjson_inputs_first`        | ndjson-10mb             |           1.7 |                2.8 |       0.61x |        4.18x |
| `ndjson_inputs_first`        | ndjson-100mb            |           1.8 |                2.9 |       0.61x |        4.15x |
| `ndjson_raw_lines`           | ndjson-1mb              |           7.8 |                4.4 |   **1.80x** |        3.93x |
| `ndjson_raw_lines`           | ndjson-10mb             |          58.5 |               18.2 |  **3.22x**† |        7.10x |
| `ndjson_raw_lines`           | ndjson-100mb            |         559.4 |              152.9 |  **3.66x**† |       39.40x |
| `ndjson_raw_fromjson`        | ndjson-1mb              |          15.4 |               11.0 |  **1.40x**† |        3.98x |
| `ndjson_raw_fromjson`        | ndjson-10mb             |         131.9 |               80.7 |  **1.64x**† |        7.14x |
| `ndjson_raw_slurp`           | ndjson-1mb              |           5.7 |                2.8 |   **2.00x** |        2.42x |
| `ndjson_raw_slurp`           | ndjson-10mb             |          38.1 |                5.0 |  **7.56x**† |        1.00x |
| `ndjson_raw_slurp`           | ndjson-100mb            |         360.6 |               23.9 | **15.12x**† |        0.88x |
| `ndjson_three_files`         | ndjson-1mb              |          34.7 |               19.1 |  **1.81x**† |        4.76x |
| `ndjson_three_files`         | ndjson-10mb             |         322.9 |              164.4 |  **1.96x**† |       15.40x |
| `users_three_files`          | users-1mb               |          17.0 |                5.7 |  **2.97x**† |        1.34x |
| `users_three_files`          | users-10mb              |         161.9 |               33.3 |  **4.86x**† |        0.48x |
| `seq_identity`               | jsonseq-1mb             |          30.7 |                9.0 |  **3.41x**† |        4.06x |
| `seq_identity`               | jsonseq-10mb            |         283.9 |               61.7 |  **4.60x**† |        8.03x |
| `seq_identity`               | jsonseq-100mb           |        2930.3 |              592.2 |   **4.95x** |       47.94x |
| `seq_field`                  | jsonseq-1mb             |          13.2 |               10.7 |   **1.23x** |        4.12x |
| `seq_field`                  | jsonseq-10mb            |         107.4 |               76.6 |  **1.40x**† |        8.05x |
| `seq_field`                  | jsonseq-100mb           |        1039.0 |              727.3 |   **1.43x** |       47.96x |
| `seq_slurp`                  | jsonseq-1mb             |          14.1 |               11.5 |   **1.22x** |        1.11x |
| `seq_slurp`                  | jsonseq-10mb            |         125.2 |               83.8 |  **1.49x**† |        0.49x |
| `users_seq_output`           | users-1mb               |           3.0 |                4.8 |       0.64x |        3.95x |
| `users_seq_output`           | users-10mb              |          15.4 |               24.6 |      0.63x† |        6.31x |
| `seq_field_bad_first`        | jsonseq-bad-first-1mb   |          12.9 |               13.2 |       0.97x |        4.26x |
| `seq_field_bad_first`        | jsonseq-bad-first-10mb  |         107.1 |              100.9 |  **1.06x**† |        9.36x |
| `seq_field_bad_first`        | jsonseq-bad-first-100mb |        1051.1 |              982.4 |   **1.07x** |       59.49x |
| `seq_field_bad_last`         | jsonseq-bad-last-1mb    |          13.0 |               13.7 |       0.95x |        4.24x |
| `seq_field_bad_last`         | jsonseq-bad-last-10mb   |         107.7 |              104.1 |  **1.03x**† |        9.42x |
| `seq_field_bad_last`         | jsonseq-bad-last-100mb  |        1037.9 |             1007.8 |   **1.03x** |       59.14x |
| `users_slurpfile`            | users-1mb               |           7.2 |                5.1 |   **1.40x** |        1.84x |
| `users_slurpfile`            | users-10mb              |          56.8 |               24.0 |  **2.37x**† |        1.13x |
| `ndjson_slurpfile`           | ndjson-1mb              |          10.2 |                7.2 |   **1.42x** |        1.67x |
| `ndjson_slurpfile`           | ndjson-10mb             |          86.5 |               42.5 |  **2.03x**† |        1.15x |
| `users_rawfile`              | users-1mb               |           3.2 |                2.8 |   **1.14x** |        2.51x |
| `users_rawfile`              | users-10mb              |          16.3 |                3.8 |  **4.35x**† |        0.96x |
| `users_rawfile`              | users-100mb             |         158.3 |               12.2 | **12.96x**† |        0.39x |
| `args_many`                  | none                    |           1.7 |                2.7 |       0.64x |        3.96x |
| `corpus_npm_deps`            | corpus-npm              |           1.8 |                2.8 |       0.64x |        4.23x |
| `corpus_npm_scripts`         | corpus-npm              |           1.8 |                2.8 |       0.65x |        4.19x |
| `corpus_charts_titles`       | corpus-charts           |           1.7 |                2.6 |       0.66x |        3.87x |
| `corpus_openapi_definitions` | corpus-openapi          |           2.2 |                2.8 |       0.81x |        3.49x |
| `corpus_openapi_strings`     | corpus-openapi          |           2.5 |                3.1 |       0.83x |        3.65x |
| `corpus_openapi_refs`        | corpus-openapi          |           2.7 |                3.3 |       0.81x |        3.87x |
| `corpus_openapi_paths`       | corpus-openapi          |           2.6 |                3.1 |       0.85x |        3.42x |
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
