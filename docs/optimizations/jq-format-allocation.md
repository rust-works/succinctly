# jq `@csv`/`@tsv`/`@dsv`/`@sh` Allocation Overhead

[Home](../../) > [Docs](../) > [Optimizations](./) > jq Format Allocation

**Status: ACCEPTED — August 2026**

**Issue**: [#647](https://github.com/rust-works/succinctly/issues/647), follow-up to
[#124](https://github.com/rust-works/succinctly/issues/124) (`@uri`/`@html` byte-oriented rewrite)

> **TL;DR.** #124 left `@csv`/`@tsv`/`@dsv`/`@sh` untouched because a prototype found them
> already fast relative to `@uri`. #647 measured them anyway and found three allocation-shape
> gaps: `@tsv` chained four `.replace()` passes, `@csv`/`@dsv` always allocated once for the
> `.replace()` escape and again for the `format!()` quote wrap, and all four formats built an
> intermediate `Vec<String>` and `.join()`-ed it. Rewriting all four to scan bytes once and
> write directly into one accumulating output buffer — the same pattern #124 used for
> `@uri`/`@html` — measured **4–21% faster end-to-end**, reproducibly, on both pinned
> architectures. All four rewrites were adopted; none needed reverting.

## Problem

`format_csv`/`format_tsv`/`format_dsv` (`src/jq/eval.rs`) each built a `Vec<String>` via
`.iter().map(...).collect()` then `.join(delimiter)` — a second full copy of already-allocated
field data. On top of that:

```rust
// format_csv / format_dsv (identical, duplicated)
OwnedValue::String(s) => format!("\"{}\"", s.replace('"', "\"\"")),
```

allocates once for `.replace()` (even when there is no `"` to escape) and again for `format!()`'s
surrounding quotes. `format_tsv` chained four sequential `.replace()` calls:

```rust
OwnedValue::String(s) => s
    .replace('\\', "\\\\")
    .replace('\t', "\\t")
    .replace('\n', "\\n")
    .replace('\r', "\\r"),
```

— up to four full passes and four allocations per field, regardless of how many (if any) of the
four characters are actually present.

## Technique

Apply the same byte-oriented, single-pass, write-directly-into-a-buffer pattern #124 used for
`format_uri`/`format_html` to all four functions:

- `write_joined_array()` — walks the array once, writing the delimiter and each field directly
  into one accumulating `String` instead of collecting a `Vec<String>` and joining it.
- `write_quoted_csv_field()` — single-pass byte scan for `"`, shared by `@csv` **and** `@dsv`
  (which had duplicated the identical quoting logic — the exact "one definition" fix this repo's
  own #106 retrospective calls out).
- `write_tsv_escaped_field()` — single-pass scan matching all four TSV escape bytes, replacing
  the four chained `.replace()` calls.
- `write_sh_quoted_field()` / `write_sh_value()` — same pattern for `@sh`'s single-quote escaping,
  replacing `shell_quote_value`'s owned-`String`-per-element shape.
- `write_owned_to_string()` — a write-into-buffer sibling of `owned_to_string`, used at every
  `other => ...` non-string arm; removes a needless allocation for bare `true`/`false`/`null`
  array elements as a side effect of touching those call sites anyway.

No SIMD: per #124's own finding, the format functions' bottleneck is allocation/pass count, not a
byte-scan bottleneck at these string sizes. `src/util/simd/escape.rs`'s `define_escape_scanner!`
macro was deliberately not used here for the same reason.

Every escape byte involved (`"`, `\`, `\t`, `\n`, `\r`, `'`) is ASCII (`< 0x80`), so UTF-8
continuation bytes (`>= 0x80`) never collide with them — the same safety argument `format_uri`/
`format_html` already established, re-verified here with dedicated multibyte-boundary regression
tests (`test_format_csv_multibyte_boundary_647`, `test_format_sh_multibyte_boundary_647`).

## Benchmark Results

Gated on `benches/jq_format_bench.rs`'s `e2e/full` tier (the file's own convention: `scan`/
`density` are diagnostic only, adoption decisions are made on `e2e`). `dsv_records`/`sh_field`
entries were added to `bench_e2e`'s `QUERIES` first — the harness had no e2e coverage for those
two formats at all.

**Method**: three independent before/after round-trips (fresh `cargo bench --save-baseline`
capture immediately followed by the corresponding `--baseline` comparison), per the "Building
both halves on a remote box" A/B recipe in
[docs/guides/benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method), plus one final
before-vs-before control comparison to measure the noise floor. An earlier single-pass
before→before→after sequence showed 5–15% swings on the control alone — a thermal/scheduling
drift artifact from running all three phases back-to-back rather than alternating them; the
three-round structure below resolves that (control settles to within ≈1–3%).

**Apple M4 Pro** (median `%` change across 3 rounds × 3 record sizes, `e2e/*/full`):

| Format | Range        | Control (noise floor) |
|--------|--------------|------------------------|
| `@csv` | **-6.6% to -14.1%** faster | within ±1% (one -7.1% outlier on the smallest/fastest 10-record case) |
| `@tsv` | **-3.6% to -6.1%** faster  | within ±1% |
| `@dsv` | **-6.5% to -8.7%** faster  | within ±1% |
| `@sh`  | **-18.7% to -21.2%** faster | within ±1% |

**AMD Ryzen 9 7950X** (same method):

| Format | Range        | Control (noise floor) |
|--------|--------------|------------------------|
| `@csv` | **-4.5% to -9.7%** faster | within ±3% |
| `@tsv` | **-2.3% to -9.7%** faster | within ±3% |
| `@dsv` | **-3.4% to -8.9%** faster | within ±3% |
| `@sh`  | **-6.3% to -14.2%** faster | within ±3% |

All four formats improved, consistently, across all 3 independent rounds, all 3 record sizes
(10/100/1000), on both architectures — well outside each machine's own control noise floor.
`@sh` shows the largest win on both platforms (its rewrite eliminates both the per-element
`Vec<String>` and an unconditional `.replace()` allocation). The improvement is roughly constant
across record-count scale rather than growing with size, consistent with a constant-factor
per-field allocation-count reduction (not an algorithmic/asymptotic fix).

Output verified byte-identical to the prior implementation: full existing test suite
(`cargo test --features cli`), the `jq_golden_conformance` oracle-parity suite (real pinned `jq`
output for `format_csv`/`format_tsv`/`format_sh`), and new multibyte-boundary regression tests
for `@csv`/`@dsv`/`@sh`.

**Key insight**: a `.replace()`/`format!()`-based implementation that already benchmarks fast in
isolation (per #124's own prototype) can still hide a real, adoptable end-to-end win once the
allocation *shape* — not raw throughput — is measured; "already fast" is not the same claim as
"as fast as it can be." A three-round alternating A/B (not a single before-then-after pair) was
necessary here: the naive sequential method's control step alone showed 5–15% drift, which would
have been indistinguishable from a real regression.

**Files**: [src/jq/eval.rs](../../src/jq/eval.rs) (`write_joined_array`, `write_quoted_csv_field`,
`write_tsv_escaped_field`, `write_sh_quoted_field`, `write_sh_value`, `write_owned_to_string`,
`format_csv`, `format_tsv`, `format_dsv`, `format_sh`), [benches/jq_format_bench.rs](../../benches/jq_format_bench.rs)
