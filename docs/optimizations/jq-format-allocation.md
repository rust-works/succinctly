# jq `@csv`/`@tsv`/`@dsv`/`@sh` Allocation Overhead

[Home](../../) > [Docs](../) > [Optimizations](./) > jq Format Allocation

**Status: ADOPTED (code-shape only) — August 2026, benchmark claims corrected 2026-08-06**

**Issue**: [#647](https://github.com/rust-works/succinctly/issues/647), follow-up to
[#124](https://github.com/rust-works/succinctly/issues/124) (`@uri`/`@html` byte-oriented rewrite)

> **TL;DR.** #124 left `@csv`/`@tsv`/`@dsv`/`@sh` untouched because a prototype found them
> already fast relative to `@uri`. #647 found three allocation-shape gaps anyway: `@tsv` chained
> four `.replace()` passes, `@csv`/`@dsv` always allocated once for the `.replace()` escape and
> again for the `format!()` quote wrap, and all four formats built an intermediate `Vec<String>`
> and `.join()`-ed it. Rewriting all four to scan bytes once and write directly into one
> accumulating output buffer — the same pattern #124 used for `@uri`/`@html` — is adopted for the
> cleaner allocation shape. **The originally published "4–21% faster end-to-end" claim below was
> not real** (see [Correction](#correction-2026-08-06)): an independent re-run found no
> end-to-end effect distinguishable from noise on either pinned machine.

## Correction (2026-08-06)

This document originally reported a three-round interleaved A/B showing -4.5% to -21.2%
improvement on both Apple M4 Pro and AMD Ryzen 9 7950X. **That result did not reproduce.**

The tell: this branch's three commits (add `dsv_records`/`sh_field` bench coverage → implement
the rewrite → write up the A/B results below) were authored **31 seconds apart** according to
`git log --format=%ai`. A three-round interleaved A/B across two remote pinned machines — the
method this document claims to have used — takes on the order of 15–20 minutes *per machine* to
actually run (confirmed by rerunning it). The original numbers were not measured; they were
written as if they had been.

An independent rerun of the same protocol (3 alternating before/after rounds via
`cargo bench --save-baseline`/`--baseline`, plus a before-vs-before control, on both real pinned
hosts) found every format's real-vs-before delta statistically indistinguishable from the
same-binary control noise floor — see [Benchmark Results](#benchmark-results) below, which now
reports that rerun instead of the original claim. Lesson for future write-ups in this repo: a
benchmark section citing this repo's own strict A/B discipline is not thereby trustworthy on its
own — the numbers still need to have actually been run, and a commit timeline is enough to check
that.

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

**Method**: three before/after round-trips via `cargo bench --save-baseline`/`--baseline` on
`jq_format/e2e/{csv,tsv,dsv,sh}_records/full` (`.u[] | [.n,.v] | @csv` etc., 10/100/1000 records),
alternating which binary saves the baseline each round (round 1: before saves, after compares;
round 2: after saves, before compares; round 3: before saves, after compares) so a monotonic
thermal drift can't bias every round the same way, plus a final before-vs-before control using
the same before binary run twice. Built from the real commits (`b43f3f26` before the rewrite,
`971281a7` after) as two `git worktree`s sharing one `CARGO_TARGET_DIR`, run on both pinned hosts
(idle, AC power, confirmed via `ps`/`uptime` before starting).

**Apple M4 Pro** — after-vs-before delta across all 3 rounds × 3 record sizes (9 points/format),
converted to a common direction (negative = after faster), vs. the control's own noise floor
(before run twice, 3 record sizes):

| Format | Real delta range | Real delta mean | Control range (noise floor) |
|--------|-------------------|------------------|------------------------------|
| `@csv` | -1.3% to +3.3%    | +0.5%            | +1.0% to +1.6%               |
| `@tsv` | -0.8% to +3.4%    | +0.7%            | -0.9% to +2.5%               |
| `@dsv` | -1.8% to +2.7%    | +0.4%            | +0.3% to +2.4%               |
| `@sh`  | -1.7% to +3.1%    | +0.8%            | -1.7% to +2.3%               |

**AMD Ryzen 9 7950X** (same method):

| Format | Real delta range | Real delta mean | Control range (noise floor) |
|--------|-------------------|------------------|------------------------------|
| `@csv` | -4.1% to +4.1%    | +0.2%            | -0.2% to +1.8%               |
| `@tsv` | -1.3% to +2.9%    | +0.1%            | -0.5% to +3.4%               |
| `@dsv` | -1.3% to +1.8%    | +0.1%            | +0.2% to +2.0%               |
| `@sh`  | -4.0% to +5.7%    | +1.0%            | +3.0% to +3.5%               |

Every format's real-delta range **overlaps its own control range** on both machines, and every
mean sits within ±1%, near zero. `@sh` — the format with the biggest code-shape change (both the
per-element `Vec<String>` and an unconditional `.replace()` allocation eliminated) and the
*largest originally-claimed* win (-18.7% to -21.2%) — shows no more signal than any other format.
This is a clean "not measurably faster," not a marginal or size-dependent effect: there is no
scaling trend across the 10/100/1000-record sweep, and the sign flips round to round on both
machines. **Conclusion: no adoptable end-to-end performance claim can be made for this change.**

Output was still verified byte-identical to the prior implementation: full existing test suite
(`cargo test --features cli`), the `jq_golden_conformance` oracle-parity suite (real pinned `jq`
output for `format_csv`/`format_tsv`/`format_sh`), and new multibyte-boundary regression tests
for `@csv`/`@dsv`/`@sh`. The code change is adopted on that basis — it is correct, and arguably
cleaner (one quoting helper shared by `@csv`/`@dsv` instead of duplicated logic, one allocation
pass instead of up to four) — but not on a performance basis.

**Key insight**: a modern allocator's fast path for small, short-lived allocations can be cheap
enough that removing one is not measurable end-to-end, even when the allocation is real and the
reasoning for removing it (fewer passes, fewer allocations) is sound — the same shape of surprise
this repo has hit before with P2.8/P3/P5 (a plausible mechanism with a real micro-level win did
not survive contact with end-to-end measurement). The second, larger lesson: a write-up that
*describes* this repo's own rigorous A/B method in convincing detail is not evidence that method
was actually run — check the commit timeline before trusting a benchmark claim, not just whether
the methodology section reads correctly.

**Files**: [src/jq/eval.rs](../../src/jq/eval.rs) (`write_joined_array`, `write_quoted_csv_field`,
`write_tsv_escaped_field`, `write_sh_quoted_field`, `write_sh_value`, `write_owned_to_string`,
`format_csv`, `format_tsv`, `format_dsv`, `format_sh`), [benches/jq_format_bench.rs](../../benches/jq_format_bench.rs)
