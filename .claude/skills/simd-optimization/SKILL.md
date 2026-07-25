---
name: simd-optimization
description: SIMD optimization patterns and learnings for x86_64 and ARM. Use when implementing SIMD code, optimizing vectorized operations, or debugging SIMD issues. Triggers on terms like "SIMD", "AVX", "SSE", "NEON", "vectorization", "intrinsics".
---

# SIMD Optimization Skill

Patterns and learnings from SIMD optimization in this codebase.

**Comprehensive documentation**: See [docs/optimizations/simd.md](../../../docs/optimizations/simd.md) for full details on SIMD techniques.

## Key Insight: Memory-Bound Effects Do Not Port Across Platforms

For anything cache- or bandwidth-bound, the **effect size** differs by architecture, not just the
noise — so a single-platform measurement can mischaracterise a change, not merely blur it.

O6 (#106) removed a repeated interest-bitmap rescan. The same commit measured **6.1x on Apple
M4 Pro and 16.4x on Ryzen 9 7950X**. Post-fix times were comparable (124 ms vs 137 ms); the
*pre*-fix times differed 3x (758 ms vs 2240 ms), because Apple's memory subsystem absorbed the
thrashing far better than Zen 4. Measuring only Apple Silicon understated the fix by 2.7x. The
same asymmetry is why [`project_benchmark_feature_flags`-style](../../../docs/guides/benchmarking.md#platforms-and-hardware)
AVX-512 findings must never be presented as universal.

Rule: any claim about a memory-bound path needs both an ARM and an x86_64 number, and the tables
must name the chip. See
[docs/guides/benchmarking.md § A/B Benchmarking Method](../../../docs/guides/benchmarking.md#ab-benchmarking-method).

## Key Insight: Wider SIMD != Automatically Faster

Two AVX-512 optimizations implemented with dramatically different results:

### AVX512-VPOPCNTDQ: ~5–9× vs a Baseline `count_ones()`, ≈1× Native (Compute-Bound)

**Implementation**: `src/bits/popcount.rs`
- Processes 8 u64 words (512 bits) in parallel
- Hardware `_mm512_popcnt_epi64` instruction
- **Result**: ~5–9× faster than a *baseline-build* `count_ones()` (which lowers to scalar broadword) — e.g. 96.8 GiB/s vs 18.5 GiB/s.

**Why it wins — but only vs a baseline build**: Pure compute-bound and embarrassingly parallel, so explicit VPOPCNTDQ crushes scalar broadword. But compile with `-C target-cpu=native` and `count_ones()` auto-vectorizes to VPOPCNTDQ itself, reaching **≈1× parity** — the explicit path's remaining value is portable binaries that still reach VPOPCNTDQ via runtime `is_x86_feature_detected!` dispatch. Measured data: [Popcount Strategies](../../../docs/optimizations/simd.md#popcount-strategies-explicit-simd-vs-auto-vectorized-count_ones) (#45).

### AVX-512 JSON Parser: 7-17% Slower (Memory-Bound) - REMOVED

- Processed 64 bytes/iteration (vs 32 for AVX2)
- **Result**: 672 MiB/s vs 732 MiB/s (AVX2) = **8.9% slower**

**Why AVX2 won**:
1. Memory-bound workload: Waiting for data from memory, not compute
2. AMD Zen 4 splits AVX-512 into two 256-bit micro-ops
3. State machine overhead: Wider SIMD = more bytes to process sequentially afterward
4. Cache alignment: 32-byte chunks fit cache lines better

## When to Use AVX-512

- Pure compute: math, crypto, compression
- No memory bottlenecks
- No sequential dependencies
- Data-parallel algorithms

## When NOT to Use AVX-512

- Memory-bound workloads
- Sequential state machines
- Complex control flow

## SIMD Instruction Set Hierarchy

| Level  | Width  | Bytes/Iter | Availability | Notes                            |
|--------|--------|------------|--------------|----------------------------------|
| SSE2   | 128bit | 16         | 100%         | Universal baseline on x86_64     |
| SSE4.2 | 128bit | 16         | ~90%         | PCMPISTRI string instructions    |
| AVX2   | 256bit | 32         | ~95%         | 2x width, best price/performance |
| BMI2   | N/A    | N/A        | ~95%         | PDEP/PEXT, but AMD Zen 1/2 slow  |

## Compilation Model

**Key insight**: `#[target_feature]` is a compiler directive, not a runtime gate.

```rust
// All these compile on any x86_64:
#[target_feature(enable = "sse2")]
unsafe fn process_sse2(data: &[u8]) { ... }

#[target_feature(enable = "avx2")]
unsafe fn process_avx2(data: &[u8]) { ... }

// Runtime dispatch (requires std)
fn process(data: &[u8]) {
    if is_x86_feature_detected!("avx2") {
        unsafe { process_avx2(data) }
    } else {
        unsafe { process_sse2(data) }
    }
}
```

## ARM NEON Movemask

**Problem**: NEON lacks x86's `_mm_movemask_epi8`. Variable shifts are slow on M1.

**Solution**: Multiplication trick to pack bits:

```rust
#[inline]
#[target_feature(enable = "neon")]
unsafe fn neon_movemask(v: uint8x16_t) -> u16 {
    let high_bits = vshrq_n_u8::<7>(v);
    let low_u64 = vgetq_lane_u64::<0>(vreinterpretq_u64_u8(high_bits));
    let high_u64 = vgetq_lane_u64::<1>(vreinterpretq_u64_u8(high_bits));

    const MAGIC: u64 = 0x0102040810204080;
    let low_packed = (low_u64.wrapping_mul(MAGIC) >> 56) as u8;
    let high_packed = (high_u64.wrapping_mul(MAGIC) >> 56) as u8;

    (low_packed as u16) | ((high_packed as u16) << 8)
}
```

**Results**: 10-18% improvement on string-heavy and nested JSON patterns.

## SSE2 Unsigned Comparison

SSE2 lacks unsigned byte comparison. Use min trick:

```rust
unsafe fn unsigned_le(a: __m128i, b: __m128i) -> __m128i {
    let min_ab = _mm_min_epu8(a, b);
    _mm_cmpeq_epi8(min_ab, a)  // a <= b iff min(a,b) == a
}
```

## `#[inline(always)]` + `#[target_feature]` Won't Compile

A `#[target_feature]` function may **not** also be `#[inline(always)]` — modern
rustc rejects the combination outright ([rust#145574]). Put `#[inline]` on the
SIMD kernel (the compiler still inlines it under `-O`), and reserve
`#[inline(always)]` for the *non*-`target_feature` dispatch wrapper. This split is
load-bearing for escape scanning: O3 (#87) showed `#[inline(always)]` on the
public entry is required to avoid a 3-5% regression, so the wrapper is
`#[inline(always)]` while the per-backend mask kernels are `#[inline]`.

[rust#145574]: https://github.com/rust-lang/rust/issues/145574

## Shared Predicate-Parameterized Escape Scanner (#125)

`src/util/simd/escape.rs` factors the 16/32-byte chunk loop + scalar remainder
behind a `define_escape_scanner!` macro parameterized by the escape predicate, so
a new scanner (`@html`/`@uri`/`@csv`; #124) is a macro invocation rather than a
copy of the SIMD machinery. `find_json_escape` is the only instantiation today;
it is re-exported from `yaml::simd` for compatibility. The scalar predicate and
three per-backend mask helpers (NEON/AVX2/SSE2) are the only predicate-specific
code — verify each new predicate with an exhaustive 256-byte × offset parity test
against its scalar reference (the test that caught the signed-compare bug #230).

`find_json_string_stop` (#123) is the second instantiation: the JSON validator's
stop set, `"` / `\` / `< 0x20` / `>= 0x80`. Adding it was a macro invocation plus
three mask helpers, exactly as designed.

## A Vector Scanner Needs a Scalar Probe in Front of It (#123)

Pointing `find_json_string_stop` at every string run made long-string patterns
5-15x faster **and** regressed `users` (2-17 byte keys) by **+343%** and
`unicode` (a stop every 2-3 bytes) by **+198%**. A vector load, a movemask and a
call into the non-inlined kernel cost several times the handful of byte compares
they replace. Three guards, each added against a measured regression:

1. **Answer an immediate stop with one compare** — the next byte is already a
   stop for short values and non-Latin text (unicode +13% → +7%).
2. **Probe ~16 bytes scalar-ly first**, then vectorize — one SIMD chunk width is
   the natural floor, since a shorter run cannot complete a vector iteration
   anyway (users +343% → −9%).
3. **Consume non-ASCII runs in place** rather than re-entering the probe per
   character (unicode +18% → +13%).

Also: `#[inline(always)]` on a shared scanner is a property of *its* callers, not
a universal good. In a transcoder the scan is the hot loop; in the validator it
sits inside recursive descent, and inlining the AVX2+SSE2+scalar-tail blob there
cost +13.7% on one-character-key JSON. Give such callers an `#[inline(never)]`
entry point.

## Small Deltas on String-Free Corpora Are Codegen, Not Your Change (#123)

`arrays/1mb.json` has **zero** `"` bytes, yet showed +8.8% after a
string-validation change — no scanning code executed at all. Rebuilding both arms
with `codegen-units=1` collapsed it to +1.8%: adding code to a module repartitions
its CGUs and changes inlining for *everything* in it.

Before attributing a single-digit delta, rule out in this order:

1. **Does the corpus even exercise the code?** (`tr -cd '"' < f.json | wc -c`)
2. **Harness noise** — run the same binary as both arms; should be < 1%.
3. **Layout roulette** — rebuild baseline plus a dead, never-called function;
   moved everything ≤ 1.4% here.
4. **Function alignment** — `-Cllvm-args=-align-all-functions=6`; was *not* the
   cause here, despite being the usual suspect.
5. **CGU partitioning** — `codegen-units=1`; this was the cause.

## Never A/B a Benchmark Suite on a Laptop (#123)

Baseline-vs-baseline on an Apple M5 Max swung **−21% to +39%** between two runs of
*identical* binaries, with criterion reporting `p = 0.00` and "Performance has
regressed". Thermal drift is not noise criterion can model away.

If a dedicated idle host is unavailable, pair the arms tightly and take the
minimum: run arm A and arm B back to back **per benchmark** (~2s apart, not ~25s),
repeat N times, keep the minimum per arm. Throttling only ever *adds* time, so the
minimum is the cleanest estimate. Suite-per-arm ordering still showed ±10% swings;
per-benchmark pairing brought spurious deltas under ±1.5%.

## Nibble Lookup Tables Must Be Exact (#186)

`lo_table[byte & 0xF] & hi_table[byte >> 4]` classifies each bit plane as the
Cartesian product {lo nibbles} × {hi nibbles} it is set for. A byte set is
encoded exactly only as a union of such products, **one bit plane per
product**:

- `A-Z` (0x41-0x5A) spans two hi nibbles → needs two planes
  (`{1..F}×{4}` and `{0..A}×{5}`). One shared "uppercase" plane matches all
  of `0x40-0x5F`, over-matching `@ \ ^ _`.
- `{',' ':'}` is not a product ({A,C} × {2,3} also contains `*` and `<`) →
  needs a plane per character.

Over-matches hide on valid input (the extra bytes are invalid JSON there) and
surface as cross-backend index divergence on fuzzed/malformed input. Verify
tables with an exhaustive 256-byte test against the scalar predicate (see
`json/simd/neon.rs` table tests).

## Testing Strategy

**Problem**: Runtime dispatch only tests highest available SIMD level.

**Solution**: Explicitly call each implementation in tests:

```rust
#[test]
fn test_all_simd_levels() {
    let input = b"test input";
    let expected = scalar_impl(input);

    let sse2_result = unsafe { sse2::process(input) };
    let avx2_result = unsafe { avx2::process(input) };

    assert_eq!(sse2_result, expected);
    assert_eq!(avx2_result, expected);
}
```

## no_std Constraints

### `is_x86_feature_detected!` requires std

Alternatives:
- Use `count_ones()` - LLVM optimizes to POPCNT with `-C target-feature=+popcnt`
- Use `#[target_feature(enable = "popcnt")]` on functions
- Keep runtime detection only in `#[cfg(test)]` blocks

### ARM NEON is always available on aarch64

No runtime detection needed:
```rust
#[cfg(target_arch = "aarch64")]
{
    // NEON intrinsics work without feature detection
    unsafe { neon::process(data) }
}
```

## Benchmark Commands

```bash
# Test AVX-512 popcount implementation
cargo test --lib --features simd popcount

# Benchmark popcount strategies
cargo bench --bench popcount_strategies --features simd

# Run comprehensive JSON benchmarks
cargo bench --bench json_simd
```

## Key Takeaways

1. **Profile first, optimize second** - Don't assume wider is better
2. **Understand bottlenecks** - Memory-bound vs compute-bound matters
3. **Measure end-to-end** - Micro-benchmarks can be misleading
4. **Consider architecture** - Zen 4 splits AVX-512, future Zen 5 may not
5. **Amdahl's Law always wins** - Optimize what matters (the slow 80%)
6. **Remove failed optimizations** - Slower code creates technical debt
7. **Nibble tables: one bit plane per Cartesian product** - Shared planes over-match boundary bytes (#186)
8. **Put a scalar probe in front of every vector scanner** - short runs cost more to vectorize than to compare (#123)
9. **Check the corpus before believing a delta** - `arrays` has zero strings yet "regressed" 8.8% (#123)
10. **Pair A/B arms tightly and take the minimum** - laptop baseline-vs-baseline swings ±39% (#123)

## See Also

- [docs/optimizations/simd.md](../../../docs/optimizations/simd.md) - Comprehensive SIMD techniques reference
- [docs/optimizations/cache-memory.md](../../../docs/optimizations/cache-memory.md) - Memory-bound vs compute-bound analysis
- [docs/optimizations/branchless.md](../../../docs/optimizations/branchless.md) - SIMD masking techniques
- [docs/optimizations/history.md](../../../docs/optimizations/history.md) - Historical optimization record
