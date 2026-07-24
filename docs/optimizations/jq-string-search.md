# jq Substring Search: `memchr::memmem` vs std Two-Way

[Home](../../) > [Docs](../) > [Optimizations](./) > jq String Search

**Status: REJECTED (deferred to a gated Phase 2) — July 2026**

**Issue**: [#303](https://github.com/rust-works/succinctly/issues/303) (this measurement) ·
follow-up to [#126](https://github.com/rust-works/succinctly/issues/126) (O6: profile jq
string operations) · depends on [#301](https://github.com/rust-works/succinctly/issues/301)
(realistic large-string corpus — the end-to-end validation this decision is gated on)

> **TL;DR.** The A/B micro-benchmark shows `memchr::memmem` (SIMD) beating Rust's std
> substring search (scalar Two-Way) exactly where theory predicts — long haystacks with a
> rare needle (**up to ~5.9× at 64 KB**, **~52× for a 1-byte needle**). That green result
> is **not** sufficient to ship. Every candidate jq op either allocates its output or
> materializes its input, so the byte-scan is a minority of wall-time; in the same
> benchmark the allocation-bound ops (`split`, `indices`) collapse to **1.1–1.5×**. There
> is **no** jq benchmark or realistic large-single-string workload exercising these ops, so
> the #126 decision gate is not met. Per that gate, and the seven prior micro-bench-win /
> end-to-end-reject precedents, the disciplined result is **reject / defer**. The bench is
> kept as the reusable harness for when [#301](https://github.com/rust-works/succinctly/issues/301)
> lands.

## Problem

Rust's std substring search — `str::find` / `rfind` / `contains` / `split(&str)` — is the
**scalar** Two-Way algorithm in `core::str::pattern`. `memchr::memmem` is **genuinely
SIMD**. The #126 recon concluded a blanket SIMD rewrite of jq string ops is unwarranted,
but flagged **one narrow cell** with real headroom: substring *search* on long haystacks
with a rare needle, in these builtins:

| Op        | `eval.rs` site      | Scan primitive today                        | What actually dominates the call |
|-----------|---------------------|---------------------------------------------|----------------------------------|
| `index`   | `builtin_index`     | `cow.find(pattern_str.as_str())`            | arg eval + owned needle; scan is the only SIMD target |
| `rindex`  | `builtin_rindex`    | `cow.rfind(pattern_str.as_str())`           | same |
| `indices` | `builtin_indices`   | overlapping `find` loop (`start += pos + 1`)| `+ Vec<OwnedValue::Int>` allocation |
| `contains`| `owned_contains`    | `a_str.contains(b_str.as_str())`            | `to_owned(&value)` materializes the full DOM first |
| `split`   | `builtin_split`     | `cow.split(&sep).map(to_string).collect()`  | `.to_string()` per part + `Vec` collect (allocation-bound) |

Note `eval.rs` always searches with a **`&str`** needle (never a `char`), so even a 1-byte
needle takes std's scalar Two-Way path — *not* the memchr fast-path `str::find(char)` gets.
That makes the 1-byte column a real comparison, and the one where memmem wins hardest.

## Method

Bench-only, **near-zero risk** (issue #303 Phase 1). Both implementations under test live
inside [`benches/jq_string_ops_bench.rs`](../../benches/jq_string_ops_bench.rs); the std
variants mirror the exact `eval.rs` call shapes above. **`eval.rs` was not touched.**

- **Input matrix**: haystack length 8 B → 64 KB; needle position start / middle / end /
  absent; needle length 1 / 4 / 16 / 64 B; separator frequency rare vs dense (`split`).
- **Parity guard** (`check_parity`, run before any timing): the memmem variants must match
  std byte-for-byte. It encodes the two semantics traps #303 calls out —
  - `indices` is **overlapping** (`indices("aa")` in `"aaaa"` → `[0,1,2]`); `memmem::find_iter`
    is non-overlapping (`[0,2]`) and is deliberately **not** used — the manual `+1` loop is kept.
  - `split` reproduces `str::split(&str)` empty-part edge cases exactly (trailing sep →
    trailing `""`; adjacent seps → interior `""`; empty sep stays on the per-char path).
- **Platform**: Apple **M4 Pro** (ARM64, NEON), `cargo 1.96`, `memchr 2.7`, criterion
  medians. The x86_64 / AVX-512 machine (7950X) was **not** measured this session; it does
  not change the decision, which is gated on workload existence, not on the micro numbers.

## Results (M4 Pro, criterion median; ratio = std ÷ memmem, >1 ⇒ memmem faster)

**`index`, needle absent (full scan — the regime memmem is built for):**

| Haystack | std      | memmem   | memmem vs std |
|----------|----------|----------|---------------|
| 8 B      | 10.5 ns  | 4.3 ns   | 2.4×          |
| 32 B     | 13.7 ns  | 12.6 ns  | 1.1×          |
| 256 B    | 41.7 ns  | 12.8 ns  | 3.3×          |
| 1 KB     | 129.6 ns | 25.8 ns  | 5.0×          |
| 16 KB    | 1.83 µs  | 322 ns   | 5.7×          |
| 64 KB    | 7.29 µs  | 1.23 µs  | **5.9×** (8.4 → 49.6 GiB/s) |

**`index`, needle length @ 16 KB haystack, absent:**

| Needle | std     | memmem  | memmem vs std |
|--------|---------|---------|---------------|
| 1 B    | 7.29 µs | 140 ns  | **52×** (2.1 → 108.9 GiB/s) |
| 4 B    | 1.83 µs | 328 ns  | 5.6×          |
| 16 B   | 513 ns  | 345 ns  | 1.5×          |
| 64 B   | 486 ns  | 639 ns  | **0.76× (memmem LOSES)** |

**Other ops:**

| Op / shape                       | std       | memmem    | memmem vs std |
|----------------------------------|-----------|-----------|---------------|
| `rindex` absent, 64 KB           | 7.29 µs   | 4.24 µs   | 1.7×          |
| `contains` miss, 16 B            | 22.6 ns   | 6.7 ns    | 3.4×          |
| `contains` miss, **64 B**        | 3.3 ns    | 10.5 ns   | **0.31× (memmem LOSES)** |
| `contains` miss, 16 KB           | 434 ns    | 330 ns    | 1.3×          |
| `indices` overlapping, 1 KB      | 9.78 µs   | 8.57 µs   | 1.1×          |
| `indices` overlapping, 16 KB     | 152 µs    | 138 µs    | 1.1×          |
| `split` **rare** sep, 16 KB      | 1.73 µs   | 616 ns    | 2.8×          |
| `split` **dense** sep, 16 KB     | 45.8 µs   | 30.3 µs   | 1.5×          |

## Interpretation — why a green micro-bench is not a ship signal

1. **The scan is a minority of every candidate op's wall-time.** The moment an op allocates
   the output it actually returns, memmem's advantage collapses: dense `split` 1.5×,
   overlapping `indices` 1.1×. A real jq call pays *more* than the micro-bench does — arg
   evaluation, owning the needle, and (for `contains`) `to_owned` materializing the whole
   DOM before any string is compared. So the scan's share in production is well under the
   micro-bench's, and far under the **40%** the #126 gate requires.
2. **memmem is not free at the short-haystack sizes that dominate jq.** Typical jq needles
   are short scalar fields (names, tags, ids). Here the picture is noisy single-digit
   nanoseconds, and memmem does lose in places (`contains` miss @ 64 B → **0.31×**). A
   blanket swap would risk regressing the common case to speed up a rare one.
3. **Long needles favour std** (`index` 64 B needle → **0.76×**): memmem's byte-prefilter
   loses its edge when no byte is rare, while Two-Way is built for long patterns.
4. **This is the exact trap this codebase has hit seven times.** P2.6, P2.8, P3, P5, P8 all
   looked good in a micro-bench and were rejected once measured end-to-end. The single most
   likely outcome of "the micro-bench is green" is a false positive, and on this hardware
   the result looks *more* uniformly green than #126 predicted — which is precisely when to
   be most suspicious, not least.

## Decision (gate from #126)

- stdlib scan **< 20 %** of end-to-end time → **skip** (expected outcome).
- stdlib scan **> 40 %** of end-to-end time **AND** a realistic large-single-string
  workload exists → consider adoption, respecting every caveat below.

No jq benchmark exercises these ops (confirmed by grep of `benches/`), and the realistic
large-string corpus ([#301](https://github.com/rust-works/succinctly/issues/301)) does not
yet exist. The allocation-bound results put the scan share below 20 % for `split` /
`indices`, and `contains` / `index` / `rindex` cannot be validated end-to-end without #301.
**→ Reject / defer.** Matches the #126 author's own prediction ("most likely no
optimization needed").

## If a Phase 2 is ever opened (only after #301 validates end-to-end)

Restricted to the ops with a genuine substring-search target — `index`, `rindex`,
`indices`, `contains` (string leaf only), `split` — and every caveat below MUST hold:

- **Keep `indices` overlapping** — the manual `start += pos + 1` loop, never `find_iter`.
- **`split` must reproduce `str::split(&str)` exactly**, empty parts included; empty
  separator stays on the per-char path. (Both guarded by `check_parity` in the bench.)
- **No length threshold without end-to-end justification** — that is exactly what P2.8
  rejected (micro said tune, end-to-end regressed 8–15 %).
- **Exclude entirely** `startswith` / `endswith` (O(needle) memcmp), `join` (pure concat),
  and the recursive `owned_contains` traversal — none have a substring-search target.
- **Byte vs codepoint**: `index` / `indices` return **byte** offsets today (both memmem and
  std agree on this); flag only if the semantics are ever revisited (jq proper counts
  codepoints).

## Reproduce

```bash
cargo bench   --bench jq_string_ops_bench   # A/B numbers (runs check_parity first)
cargo test    --bench jq_string_ops_bench   # parity guard only
succinctly bench run jq_string_ops_bench    # via the bench runner (feature: bench-runner)
```

## See Also

- [access-patterns.md](access-patterns.md) — search algorithms (Two-Way, prefilters)
- [simd.md](simd.md) — why wider/SIMD ≠ faster on memory-bound work (AVX-512 precedents)
- [history.md](history.md) — chronological optimization log
- [../parsing/yaml.md](../parsing/yaml.md) — the P/O-series micro-bench-win / reject precedents
