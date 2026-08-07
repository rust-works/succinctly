# jq `@csv`/`@tsv`/`@dsv`/`@sh` Allocation Rewrite

[Home](../../) > [Docs](../) > [Optimizations](./) > jq Format Allocation

**Status: REJECTED — investigated, no measurable end-to-end effect, no code adopted — August 2026**

**Issue**: [#647](https://github.com/rust-works/succinctly/issues/647) (this investigation),
follow-up to [#124](https://github.com/rust-works/succinctly/issues/124) (`@uri`/`@html` — a
real, measured win)

> **TL;DR.** #124 found `@uri`/`@html`'s per-`char` push loops were a genuine 12-15x
> bottleneck and fixed it with a byte-oriented rewrite. That same prototype found
> `@csv`/`@sh`'s existing `.replace()`/`format!()`-based code already ran at 10-14 GiB/s, so
> #124 left `@csv`/`@tsv`/`@dsv`/`@sh` untouched. #647 investigated those four formats
> properly rather than assuming "already fast" meant "as fast as it can be" — a hand-rolled
> byte-scan rewrite was drafted and A/B measured on both pinned hosts. Every format's
> real-vs-before delta overlapped its own same-binary control noise floor, with no scaling
> trend across the 10/100/1000-record sweep. **No adoptable win. No code was adopted.**
> This document's own first draft briefly claimed a large win using numbers that were never
> actually measured — see [Correction](#correction--the-original-write-up-reported-unmeasured-numbers)
> below for the full account.

## Problem

`format_tsv`, `format_csv`, and `format_dsv` (`src/jq/eval.rs`) build their output with
`.replace()`/`format!()`-based per-field transforms, then collect a `Vec<String>` and
`.join()` it — the same double-allocation shape #124's `df77915d` eliminated for `@uri`/`@html`.
Three concrete gaps motivated the investigation:

1. **`format_tsv` chains four sequential `.replace()` calls per string field**:
   ```rust
   s.replace('\\', "\\\\")
    .replace('\t', "\\t")
    .replace('\n', "\\n")
    .replace('\r', "\\r")
   ```
   Every field pays for up to four full passes and up to four allocations, even when none of
   the four characters is present — unlike `@csv`'s single-pass `.replace('"', "\"\"")`.

2. **`format_csv`/`format_dsv` always allocate twice per string field**, regardless of
   whether the field contains a `"` to escape:
   ```rust
   format!("\"{}\"", s.replace('"', "\"\""))
   ```
   `.replace()` allocates a new `String` even when there is nothing to replace, and
   `format!` allocates again to add the surrounding quotes. (This logic now lives in a
   single shared `quote_csv_field` helper — see [#651](https://github.com/rust-works/succinctly/issues/651)
   — but the allocation shape is unchanged; #651 deduplicated the code, not the cost.)

3. **All four formats build an intermediate `Vec<String>` via `.map(...).collect()`, then
   `.join(delimiter)`** — a second full copy of already-allocated data.

## What was investigated

A hand-rolled byte-scanning rewrite of all four format functions — the same shape as #124's
`@uri`/`@html` fix — was drafted on branch `issue-647-csv-dsv-allocation-overhead` (PR #650).
It would have replaced the `.replace()`/`format!()`/`.join()` chains above with direct,
single-pass writes into one output buffer: scanning each field once for the characters that
need escaping, writing runs of unescaped bytes directly, and appending delimiters without an
intermediate `Vec<String>`.

**None of this was adopted.** `format_csv`, `format_tsv`, `format_dsv`, and `format_sh` in
`src/jq/eval.rs` are, today, exactly the `.replace()`/`format!()`/`.join()`-based code
described in [Problem](#problem) above (with the #651 quoting dedup, which changed nothing
about the allocation shape).

## Benchmark Results (real, independently measured)

Measured via a 3-round interleaved A/B (`cargo bench --save-baseline`/`--baseline` on the
`jq_format/e2e/full` tier, plus a before-vs-before control run to establish the noise floor —
see [docs/guides/benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method)) on both
pinned hosts, Apple M4 Pro and AMD Ryzen 9 7950X:

| Format | M4 Pro real delta (mean) | 7950X real delta (mean) | Control noise floor |
| ------ | ------------------------ | ----------------------- | ------------------- |
| `@csv` | -1.3% to +3.3% (+0.5%)   | -4.1% to +4.1% (+0.2%)  | ~1-2.5%             |
| `@tsv` | -0.8% to +3.4% (+0.7%)   | -1.3% to +2.9% (+0.1%)  | ~1-3.4%             |
| `@dsv` | -1.8% to +2.7% (+0.4%)   | -1.3% to +1.8% (+0.1%)  | ~0.3-2.4%           |
| `@sh`  | -1.7% to +3.1% (+0.8%)   | -4.0% to +5.7% (+1.0%)  | ~1.7-3.5%           |

Every format's real-vs-before delta range overlaps its own control range, on both machines,
with no scaling trend across the 10/100/1000-record sweep — the signature of noise, not a
real effect.

## Interpretation

This is the same conclusion #124's original prototype pointed at: `@csv`/`@sh`'s
`.replace()`/`format!()`-based code was already running at 10-14 GiB/s, nowhere near the
~740-840 MiB/s that made `@uri`/`@html` worth fixing. Allocation count and pass count are not
the bottleneck at the field sizes real jq output produces — array iteration, string
formatting of non-string values, and output-buffer growth dominate instead, and a byte-scan
rewrite has nothing to offer against those. It is an acceptable, useful outcome for #647 to
conclude "no change," exactly as that issue's own investigation plan called out in advance.

## Decision

**Reject. No code adopted.** `src/jq/eval.rs` is unchanged from before this investigation
(aside from the unrelated #651 dedup, see below).

## Correction — the original write-up reported unmeasured numbers

An earlier version of this document (and the corresponding `CHANGELOG.md`/`history.md`
entries) claimed a measured end-to-end win for the byte-scan rewrite. Those numbers were
**not measured** — the three commits implementing the rewrite were authored 31 seconds apart,
far too fast to have run the multi-round, dual-host A/B protocol the write-up described. This
was caught during review and an honest rerun of the same protocol (described above) found no
adoptable effect for any of the four formats. The fabricated figures are not reproduced here,
even as historical color — the [Benchmark Results](#benchmark-results-real-independently-measured)
table above is the only performance data on the record for this investigation. This incident
is preserved here, in the same spirit as this repo's other "Notable Failures" write-ups, as a
reminder that a benchmark claim is only as trustworthy as the process that produced it — see
[docs/guides/benchmarking.md](../guides/benchmarking.md) for the discipline this investigation
was re-measured against.

## Related work

- **[#651](https://github.com/rust-works/succinctly/issues/651)** — a small, independent fix
  deduplicating `@csv`'s and `@dsv`'s identical quoting logic into the shared
  `quote_csv_field` helper. It has merit on its own (one definition of the quoting rule
  instead of two copies) and is **not part of this investigation's outcome** — it changes
  nothing about the allocation shape or the performance conclusion above.
- **PR #650** — the branch this investigation was drafted on. It has been repurposed to drop
  the byte-scan rewrite entirely; per [#647's closing comment](https://github.com/rust-works/succinctly/issues/647#issuecomment-5209766877),
  it retains only new `e2e` benchmark coverage and regression tests pinning existing correct
  output behavior, with no production code change.
- **[#652](https://github.com/rust-works/succinctly/issues/652)** — the `CHANGELOG.md` entry
  for this investigation's outcome.
- **[#653](https://github.com/rust-works/succinctly/issues/653)** — the issue that corrected
  this document (and `history.md`/`README.md`) to match the real outcome above.

## Reproduce

```bash
cargo bench --bench jq_format_bench -- jq_format/e2e/full
```

Follow the [A/B Benchmarking Method](../guides/benchmarking.md#ab-benchmarking-method) —
interleave before/after reps within each round, run on both pinned hosts, and compare against
a before-vs-before control before trusting any delta.

## See Also

- [jq-string-search.md](jq-string-search.md) — a sibling rejected jq investigation
  (`memchr::memmem`), same "green micro, no end-to-end signal" shape
- [history.md](history.md) — chronological optimization log
- [README.md](README.md) — quick reference table of all optimization techniques
