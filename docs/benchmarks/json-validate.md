# JSON Validation Benchmarks

[Home](../../) > [Docs](../) > [Benchmarks](./) > JSON Validation

Performance benchmarks for `succinctly json validate` - strict RFC 8259 JSON validation.

## Summary

| Metric         | Apple M4 Pro         | AMD Ryzen 9 7950X    |
|----------------|----------------------|----------------------|
| **Throughput** | 530-1800 MiB/s       | 466-1810 MiB/s       |
| **Peak**       | 1.8 GiB/s (nested)   | 1.81 GiB/s (nested)  |
| **Patterns**   | 10 (1KB-10MB each)   | 10 (1KB-10MB each)   |

**Note**: The validator is purely scalar (no SIMD). High throughput comes from efficient recursive descent parsing with good branch prediction.

## Platforms

### Apple M4 Pro (ARM)
- **CPU**: Apple M4 Pro (12 cores)
- **Build**: `cargo build --release --features cli`
- **Date**: 2026-02-04 (commit 16de8f9)

### AMD Ryzen 9 7950X (x86_64)
- **CPU**: AMD Ryzen 9 7950X 16-Core Processor
- **OS**: Ubuntu 22.04.5 LTS
- **Build**: `cargo build --release --features cli`
- **Date**: 2026-02-04 (commit a7f6f47)

---

## Apple M4 Pro (ARM) Results

### 1KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 124 ns   | 637 MiB/s    |
| users        | 517 ns   | 984 MiB/s    |
| strings      | 534 ns   | 1.52 GiB/s   |
| nested       | 568 ns   | 1.52 GiB/s   |
| numbers      | 683 ns   | 1.31 GiB/s   |
| literals     | 1.18 µs  | 824 MiB/s    |
| pathological | 1.32 µs  | 736 MiB/s    |
| comprehensive| 1.59 µs  | 994 MiB/s    |
| arrays       | 1.65 µs  | 597 MiB/s    |
| unicode      | 1.01 µs  | 991 MiB/s    |

### 10KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 1.11 µs  | 769 MiB/s    |
| nested       | 5.43 µs  | 1.74 GiB/s   |
| strings      | 5.18 µs  | 1.62 GiB/s   |
| users        | 5.77 µs  | 1006 MiB/s   |
| numbers      | 6.74 µs  | 1.31 GiB/s   |
| comprehensive| 11.31 µs | 831 MiB/s    |
| pathological | 13.05 µs | 749 MiB/s    |
| literals     | 15.65 µs | 623 MiB/s    |
| arrays       | 16.96 µs | 576 MiB/s    |
| unicode      | 9.75 µs  | 1004 MiB/s   |

### 100KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 13.4 µs  | 714 MiB/s    |
| nested       | 52.9 µs  | 1.80 GiB/s   |
| strings      | 52.3 µs  | 1.60 GiB/s   |
| users        | 59.2 µs  | 1016 MiB/s   |
| numbers      | 67.6 µs  | 1.31 GiB/s   |
| comprehensive| 109.5 µs | 780 MiB/s    |
| pathological | 134.2 µs | 728 MiB/s    |
| literals     | 179.7 µs | 543 MiB/s    |
| arrays       | 172.7 µs | 566 MiB/s    |
| unicode      | 98.1 µs  | 996 MiB/s    |

### 1MB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 164 µs   | 649 MiB/s    |
| nested       | 540 µs   | 1.81 GiB/s   |
| strings      | 528 µs   | 1.62 GiB/s   |
| users        | 618 µs   | 1.01 GiB/s   |
| numbers      | 685 µs   | 1.32 GiB/s   |
| comprehensive| 1.07 ms  | 757 MiB/s    |
| pathological | 1.35 ms  | 741 MiB/s    |
| literals     | 1.89 ms  | 528 MiB/s    |
| arrays       | 1.77 ms  | 565 MiB/s    |
| unicode      | 1.01 ms  | 989 MiB/s    |

### 10MB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 1.93 ms  | 606 MiB/s    |
| nested       | 5.44 ms  | 1.80 GiB/s   |
| strings      | 5.42 ms  | 1.58 GiB/s   |
| users        | 6.25 ms  | 1.03 GiB/s   |
| numbers      | 6.88 ms  | 1.31 GiB/s   |
| comprehensive| 10.45 ms | 766 MiB/s    |
| pathological | 13.56 ms | 738 MiB/s    |
| literals     | 18.80 ms | 532 MiB/s    |
| arrays       | 17.77 ms | 563 MiB/s    |
| unicode      | 10.12 ms | 989 MiB/s    |

## Performance by Pattern Type

| Pattern           | Characteristics                      | Typical Throughput |
|-------------------|--------------------------------------|--------------------|
| **nested**        | Deeply nested objects/arrays         | 1.7-1.8 GiB/s      |
| **strings**       | String-heavy content                 | 1.5-1.6 GiB/s      |
| **numbers**       | Numeric arrays                       | 1.3 GiB/s          |
| **users**         | Realistic user data                  | 1.0 GiB/s          |
| **unicode**       | Unicode string content               | 990 MiB/s          |
| **comprehensive** | Mixed realistic content              | 760-830 MiB/s      |
| **pathological**  | Edge cases (escapes, deep nesting)   | 730-750 MiB/s      |
| **mixed**         | Small mixed records                  | 600-770 MiB/s      |
| **arrays**        | Large flat arrays                    | 560-600 MiB/s      |
| **literals**      | true/false/null heavy                | 530-820 MiB/s      |

---

## AMD Ryzen 9 7950X (x86_64) Results

### 1KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 156 ns   | 507 MiB/s    |
| strings      | 570 ns   | 1.43 GiB/s   |
| nested       | 576 ns   | 1.50 GiB/s   |
| users        | 602 ns   | 844 MiB/s    |
| numbers      | 762 ns   | 1.17 GiB/s   |
| unicode      | 1.13 µs  | 885 MiB/s    |
| literals     | 1.38 µs  | 704 MiB/s    |
| arrays       | 1.86 µs  | 530 MiB/s    |
| pathological | 1.87 µs  | 521 MiB/s    |
| comprehensive| 2.00 µs  | 788 MiB/s    |

### 10KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 1.31 µs  | 652 MiB/s    |
| nested       | 5.33 µs  | 1.77 GiB/s   |
| strings      | 5.50 µs  | 1.52 GiB/s   |
| users        | 6.67 µs  | 871 MiB/s    |
| numbers      | 7.48 µs  | 1.18 GiB/s   |
| unicode      | 10.86 µs | 902 MiB/s    |
| comprehensive| 12.94 µs | 726 MiB/s    |
| literals     | 13.82 µs | 706 MiB/s    |
| pathological | 18.67 µs | 523 MiB/s    |
| arrays       | 18.72 µs | 522 MiB/s    |

### 100KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 15.5 µs  | 617 MiB/s    |
| nested       | 52.8 µs  | 1.81 GiB/s   |
| strings      | 53.8 µs  | 1.56 GiB/s   |
| users        | 66.4 µs  | 905 MiB/s    |
| numbers      | 75.4 µs  | 1.17 GiB/s   |
| unicode      | 108 µs   | 904 MiB/s    |
| comprehensive| 125 µs   | 684 MiB/s    |
| pathological | 187 µs   | 522 MiB/s    |
| arrays       | 189 µs   | 517 MiB/s    |
| literals     | 201 µs   | 485 MiB/s    |

### 1MB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 167 µs   | 637 MiB/s    |
| nested       | 541 µs   | 1.81 GiB/s   |
| strings      | 550 µs   | 1.56 GiB/s   |
| users        | 693 µs   | 917 MiB/s    |
| numbers      | 776 µs   | 1.17 GiB/s   |
| unicode      | 1.10 ms  | 906 MiB/s    |
| comprehensive| 1.25 ms  | 649 MiB/s    |
| pathological | 1.91 ms  | 523 MiB/s    |
| arrays       | 1.97 ms  | 508 MiB/s    |
| literals     | 2.15 ms  | 466 MiB/s    |

### 10MB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 2.08 ms  | 562 MiB/s    |
| nested       | 5.43 ms  | 1.80 GiB/s   |
| strings      | 5.51 ms  | 1.56 GiB/s   |
| users        | 7.06 ms  | 929 MiB/s    |
| numbers      | 7.75 ms  | 1.17 GiB/s   |
| unicode      | 11.05 ms | 905 MiB/s    |
| comprehensive| 12.04 ms | 665 MiB/s    |
| pathological | 18.56 ms | 539 MiB/s    |
| arrays       | 19.91 ms | 502 MiB/s    |
| literals     | 21.45 ms | 466 MiB/s    |

### Performance by Pattern Type (x86_64)

| Pattern           | Characteristics                      | Typical Throughput |
|-------------------|--------------------------------------|--------------------|
| **nested**        | Deeply nested objects/arrays         | 1.5-1.8 GiB/s      |
| **strings**       | String-heavy content                 | 1.4-1.6 GiB/s      |
| **numbers**       | Numeric arrays                       | 1.17 GiB/s         |
| **users**         | Realistic user data                  | 840-930 MiB/s      |
| **unicode**       | Unicode string content               | 885-905 MiB/s      |
| **comprehensive** | Mixed realistic content              | 650-790 MiB/s      |
| **literals**      | true/false/null heavy                | 466-706 MiB/s      |
| **mixed**         | Small mixed records                  | 510-650 MiB/s      |
| **pathological**  | Edge cases (escapes, deep nesting)   | 520-540 MiB/s      |
| **arrays**        | Large flat arrays                    | 500-530 MiB/s      |

---

## Key Findings

1. **Nested structures are fastest**: The validator excels at deeply nested JSON, achieving 1.8 GiB/s throughput. Simple structural characters (`{`, `}`, `[`, `]`) are validated quickly.

2. **String-heavy content is fast**: At 1.4-1.6 GiB/s, string validation is efficient despite checking escape sequences, surrogate pairs, and control characters.

3. **Consistent scaling**: Throughput remains stable from 1KB to 10MB files on both platforms, indicating good cache behavior and minimal overhead.

4. **Literals are slowest**: JSON with many `true`/`false`/`null` values requires keyword matching, reducing throughput to ~466-820 MiB/s.

5. **Cross-platform consistency**: ARM and x86_64 achieve comparable peak throughput (~1.8 GiB/s). The validator is purely scalar (no SIMD) - performance comes from efficient branch prediction and cache-friendly sequential access.

## Shape analysis: what a chunk scanner could win (#130)

Issue #130 proposes replacing the scalar recursive-descent validator with a
64-byte SIMD chunk scanner feeding an explicit state machine. Per
[docs/guides/benchmarking.md](../guides/benchmarking.md), #130 is gated on the
corpus-shape check. This section is that check.

### The metric that decides it

Run length is the wrong metric. A chunk scanner does not need *long* runs of
skippable bytes — it needs **few positions its state machine must visit per
chunk**. Call that the *interesting density*: structural characters outside
strings, plus one visit per string (the body and closing quote are skipped),
plus one per atom start, plus any garbage byte.

Measured over the real-workload corpus as it stood at the time of the decision
(5 JSON files, 971 KB), against current M4 Pro throughput. The corpus has since
grown to 7 files / 1014 KB — see [below](#the-corpus-has-since-grown) for what
that changed:

| corpus file            | visits / 64 B | bytes / visit | current throughput |
| ---------------------- | ------------- | ------------- | ------------------ |
| pretty/3d-ribbon.json  |           4.0 |          16.0 | **1.79 GiB/s**     |
| tabular/carshare-data  |           8.0 |           8.0 | 1.21 GiB/s         |
| geojson/us-election    |          10.9 |           5.9 | 1.19 GiB/s         |
| charts/bullet-data     |          20.0 |           3.2 | 856 MiB/s          |
| graph/miserables.json  |          24.0 |           2.7 | 789 MiB/s          |

Throughput is almost perfectly inverse to density — unsurprising, since scalar
cost tracks token count.

### Where break-even sits

#130's own prototype measured **1.16–1.32×** on string-heavy input (density ≈ 1)
and **0.86–0.91×** on ASCII-heavy structured objects (density ≈ 18–22).
Interpolating those two measured points puts break-even at roughly **8–12
visits per 64-byte chunk**.

### The answer

**Every real corpus file falls on the wrong side of that line, in one of two
ways:**

* **Below break-even, but no headroom.** `3d-ribbon.json` is the one file with a
  favourable density (4.0). It already validates at **1.79 GiB/s** — statistically
  indistinguishable from the `nested` pattern (1.81 GiB/s) that this document
  already describes as "near memory bandwidth limit". There is nothing left to
  capture.
* **Headroom, but above break-even.** The two slowest files, `miserables.json`
  (789 MiB/s) and `bullet-data.json` (856 MiB/s), sit at densities of 24.0 and
  20.0 — squarely in the band where #130's prototype measured a **regression**.

No corpus file is both slow enough to be worth optimising and sparse enough for
a chunk scanner to help.

### Two supporting findings

* **String skipping is dead on real input.** String bodies are 5.7% of corpus
  bytes and the longest string is **111 bytes**, against the ~300 B regime where
  #130 measured its win — **not one string in the corpus reaches it**. Only 2.2%
  reach even 32 bytes.
* **The escape machinery never pays.** The entire 1014 KB corpus contains **six
  backslashes**, all in one file. The odd-backslash-run mask (`find_escaped`) is
  the most intricate and highest-risk primitive in the design, has no equivalent
  anywhere in this crate, and is nonetheless *structurally required*: a single
  `\"` desynchronises the in-string mask for the rest of the document. On real
  input it is pure cost that can never be recovered.

Note also that whitespace fraction is a misleading proxy. `3d-ribbon.json` is
85% whitespace, but in runs of p50=19 / max=23 bytes — **0.00% of whitespace
runs in the entire corpus reach 32 bytes**, so no single SIMD lane is ever
filled by whitespace alone.

### The corpus has since grown

The decision above was taken against a 5-file / 971 KB JSON corpus. `f9792019`
then added two string-heavy files (`npm/express-package.json`,
`openapi/swagger-v2-schema.json`), taking it to 7 files / 1014 KB. Both are
token-dense object documents, so they land *above* break-even and reinforce the
rejection rather than challenging it. The two supporting findings above are
quoted at post-growth values; against the original 5 files they read 4.6% of
bytes / 51 B longest string / 0.03% ≥ 32 B, and zero backslashes.

The one number that moved in the scanner's favour is the longest string, 51 B →
111 B. It does not approach the threshold that matters: **no string in the
corpus reaches the ~300 B regime where #130 measured its 1.16–1.32× win**, and
the p50 is still 9 bytes.

### Consequence

The optimisation with real headroom is the opposite of #130: the slow files are
slow because they are *token-dense*, so the win is in making per-token work
cheaper (keyword and number scanning in the scalar path), not in skipping bytes
between tokens. See #122/#123 for the adjacent SIMD proposals, which the same
analysis constrains.

### Reproducing

The throughput column needs the **full** corpus: only three of the seven JSON
files are vendored in the committed seed, and `json_validate_real_corpus_bench`
announces on stderr when it is running seed-only.

```bash
./scripts/sync-bench-corpus.sh                            # fetch the full JSON corpus

# shape statistics (the visits/64B column)
succinctly dev bench corpus-stats --data-dir data/bench/corpus

# throughput column — a standalone bench target, no synthetic ladder needed
cargo bench --bench json_validate_real_corpus_bench
```

## Per-token scalar optimisations

ADR-0013's finding — the slow patterns are slow because they are *token-dense* —
points the optimisation effort at per-token cost rather than at skipping bytes
between tokens. Two changes were written and measured against that.

**Method.** Three interleaved A/B/A/B rounds on an Apple M4 Pro, Criterion
baselines, 100 KB tier. `nested` and `strings` contain neither numbers nor
keywords, so any movement there is pure noise; they calibrate the floor at
**±3.4%**. An earlier all-A-then-all-B run was discarded: it showed ±10% swings
on those same control patterns, which the changes cannot affect, so thermal
drift — not the code — dominated it.

| change | numbers | literals | arrays | users | verdict |
|---|---|---|---|---|---|
| digit-run scanning | **−9.0%** | −0.9% | +0.3% | +1.9% | ✅ accepted |
| keyword literal compare | +2.6% | **+6.0%** | −0.9% | +0.8% | ❌ rejected |

**Accepted — digit-run scanning.** `validate_number`'s three digit loops called
`advance()` per digit, each re-checking bounds and constructing an `Option`.
Scanning the run once and adjusting `offset`/`column` in one step is exactly
equivalent (digits contain no line terminator) and worth 9% on number-heavy
input, with nothing else moving beyond noise.

**Rejected — keyword literal compare.** Replacing `validate_keyword`'s
byte-at-a-time loop with a first-byte dispatch plus a whole-literal compare made
`literals` **6% slower** — the one pattern it was written for. The original loop
runs four or five perfectly-predicted iterations; the dispatch, `starts_with`,
and the mandatory one-byte lookahead together cost more than they save. This is
[ADR-0006](../adrs/adr-0006.md)'s finding again, in a new place: on short,
highly-predictable runs the branch predictor beats bulk comparison. Reverted
rather than threshold-tuned, per [ADR-0005](../adrs/adr-0005.md).

The lookahead that made it correct is worth recording even though the code is
gone: without it, `truex` matches the `true` prefix and is reported as an
unexpected `x`, where the greedy path reports `InvalidKeyword{found:"truex"}` at
the `t` — different kind *and* offset, both rendered by the CLI. Two named tests
in `src/json/validate.rs` now pin that contract so a future fast path cannot
quietly lose it.

## Running Benchmarks

```bash
# Build with benchmark runner
cargo build --release --features bench-runner

# Run JSON validation benchmarks
./target/release/succinctly bench run json_validate_bench

# Run with Criterion directly
cargo bench --bench json_validate_bench

# Real-workload corpus (separate target; see "Reproducing" above)
./scripts/sync-bench-corpus.sh
cargo bench --bench json_validate_real_corpus_bench
```

## Benchmark Data Location

Raw benchmark results:
- Apple M4 Pro: `data/bench/results/20260204_194447_16de8f9/`
- AMD Ryzen 9 7950X: `data/bench/results/20260204_211039_a7f6f47/`

## See Also

- [jq Benchmarks](jq.md) - JSON query performance
- [Rust JSON Parsers](rust-parsers.md) - Parser comparison
- [JSON Validation Command](../guides/cli.md#json-validation) - CLI documentation
