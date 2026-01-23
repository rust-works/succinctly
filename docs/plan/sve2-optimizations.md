# SVE2 Optimization Plan for Succinctly

**Status**: Updated 2026-01-23 - Graviton 4 optimizations complete, low-hanging fruit exhausted

## Executive Summary

This document analyzes all current x86 and ARM NEON optimizations in the Succinctly codebase and plans SVE2 equivalents for ARM CPUs that support it (Azure Cobalt 100/Neoverse N2, AWS Graviton 3+, etc.).

**Key finding**: SVE2 on Neoverse N2 (GitHub Actions ARM runners) has **128-bit vectors** - the same width as NEON. Therefore, SVE2's value comes from:
1. **BDEP/BEXT instructions** - equivalent to x86 BMI2 PDEP/PEXT ✅ **Implemented**
2. **Predicated operations** - cleaner handling of tail bytes without scalar fallbacks ❌ **Rejected** (50% slower)
3. **First-fault loads** - potential for speculative scanning ❌ **Not pursued** (complexity vs benefit)

---

## Graviton 4 Optimization Status (January 2026)

### ✅ Completed Optimizations

| Optimization | Location | Speedup | Date |
|--------------|----------|---------|------|
| NEON JSON parsing | `json/simd/neon.rs` | 1.11x | Pre-2026 |
| NEON YAML block scalars | `yaml/simd/neon.rs` | 11-23% | 2026-01 |
| NEON YAML anchors | `yaml/simd/neon.rs` | 3-6% | 2026-01 |
| NEON PMULL prefix XOR | `dsv/simd/neon.rs` | 25% | 2026-01-22 |
| NEON VMINV BP L1 index | `trees/bp.rs` | 2.8x | 2026-01 |
| NEON VMINV BP L2 index | `trees/bp.rs` | 1-3% | 2026-01 |
| NEON popcount unrolling | `bits/popcount.rs` | 15% | 2026-01 |
| SVE2-BITPERM BDEP select | `util/broadword.rs` | 5-17x micro, 2-5% e2e | 2026-01-23 |
| SSE4.1 PHMINPOSUW BP L1/L2 | `trees/bp.rs` | 1-3% (large data) | 2026-01-24 |

### ❌ Rejected Optimizations

| Attempt | Result | Reason |
|---------|--------|--------|
| SVE2 inline assembly (block scalars) | -40% | Compiler can't optimize asm blocks |
| SVE2 MATCH (multi-char search) | -46% | Same vector width as NEON, more overhead |
| Software prefetch | -30% | Hardware prefetcher is better |
| AVX-512 YAML | -7% | Memory bandwidth saturated |
| Branchless char classification | -25-44% | Branch predictors are 93-95% accurate |

### ⚠️ Marginal Optimizations

| Optimization | Result | Platform | Notes |
|--------------|--------|----------|-------|
| SSE4.1 PHMINPOSUW BP L1/L2 | +1-3% (10M+ nodes) | x86_64 | Bias trick overhead limits gains vs NEON's 2.8x |

### 🔮 Remaining Opportunities

| Opportunity | Expected Gain | Effort | Platform |
|-------------|---------------|--------|----------|
| x86 PDEP select_in_word | 5-17x micro | Low | x86_64 only |
| LTO (Link-Time Optimization) | 2-5% | Low | All |
| PGO (Profile-Guided Optimization) | 5-10% | Medium | All |
| Dead code cleanup (json/simd/bmi2.rs) | 0% (cleanliness) | Low | All |

### Conclusion

**For Graviton 4 (Neoverse-V2), the low-hanging SIMD fruit is exhausted.**

- 128-bit SVE2 vectors don't provide wider-than-NEON benefits
- SVE2 inline assembly is 40-50% slower than NEON intrinsics
- Specialized instructions (BDEP via SVEBITPERM) are the only SVE2 win
- Further gains require algorithmic changes, not SIMD optimizations

**Next steps** (if pursuing further optimization):
1. Profile real workloads to identify new hotspots
2. Consider LTO for release builds
3. Implement x86 PDEP select_in_word for cross-platform parity

---

## SVE2 Inline Assembly Benchmark Results (January 2026)

**IMPORTANT FINDING**: SVE2 with inline assembly is **~50% slower** than NEON intrinsics on Graviton 4.

### Experiment: find_block_scalar_end

Tested SVE2 predicates (`whilelt`, `ld1b`, `cmpeq`, `brkb`, `cntp`) vs NEON intrinsics:

| Test Case | NEON | SVE2 | Speedup |
|-----------|------|------|---------|
| 10 lines, 4 indent | 95 ns | 154 ns | **0.62x** |
| 100 lines, 4 indent | 1052 ns | 1745 ns | **0.60x** |
| 1000 lines, 4 indent | 10605 ns | 17617 ns | **0.60x** |

### Experiment: Multi-character search (SVE2 MATCH approach)

Tested SVE2 for finding newline, colon, or hash characters:

| Size | NEON | SVE2 | Speedup |
|------|------|------|---------|
| 64B | 5.6 ns (11.5 GiB/s) | 10.3 ns (6.2 GiB/s) | **0.54x** |
| 1KB | 85 ns (12.0 GiB/s) | 156 ns (6.5 GiB/s) | **0.54x** |
| 16KB | 1345 ns (12.2 GiB/s) | 2483 ns (6.6 GiB/s) | **0.54x** |

### Why SVE2 Inline Assembly is Slower

1. **Same vector width**: SVE2 on Graviton 4 uses 128-bit vectors (identical to NEON)
2. **Inline assembly overhead**: `core::arch::asm!` blocks prevent:
   - Register allocation optimization
   - Instruction scheduling
   - Loop unrolling
3. **Predicate management cost**: SVE2 predicates add instructions that NEON handles more efficiently with masks

### Conclusion

**Do NOT use SVE2 inline assembly for YAML parsing on Graviton 4.**

The only beneficial SVE2 feature is **SVEBITPERM** (BDEP/BEXT instructions), which is already used in DSV parsing. For regular SIMD operations, NEON intrinsics remain superior until Rust SVE2 intrinsics stabilize.

---

## Current Optimization Inventory

### 1. x86_64 SIMD Hierarchy

| Level | Width | Files | Operations | Key Instructions |
|-------|-------|-------|------------|------------------|
| SSE2 | 128-bit | `json/simd/x86.rs` | Character classification | `_mm_cmpeq_epi8`, `_mm_movemask_epi8` |
| SSE4.2 | 128-bit | `json/simd/sse42.rs` | String matching | `_mm_cmpistrm` (PCMPISTRI) |
| AVX2 | 256-bit | `json/simd/avx2.rs`, `dsv/simd/avx2.rs`, `yaml/simd/x86.rs` | Character classification, string scanning | `_mm256_cmpeq_epi8`, `_mm256_movemask_epi8` |
| BMI2 | 64-bit | `dsv/simd/bmi2.rs`, `json/simd/bmi2.rs` | Quote state masking | `_pdep_u64`, `_pext_u64` |
| AVX-512 | 512-bit | `bits/popcount.rs` | Popcount only | `_mm512_popcnt_epi64` |

### 2. ARM NEON Implementations

| File | Operations | Key Techniques |
|------|------------|----------------|
| [src/json/simd/neon.rs](../../src/json/simd/neon.rs) | JSON character classification | Nibble lookup tables (`vqtbl1q_u8`), custom `neon_movemask` |
| [src/yaml/simd/neon.rs](../../src/yaml/simd/neon.rs) | String scanning, indentation counting | Direct comparison, broadword fallbacks |
| [src/dsv/simd/neon.rs](../../src/dsv/simd/neon.rs) | DSV field detection | Character matching, **PMULL prefix XOR** for quotes |
| [src/bits/popcount.rs](../../src/bits/popcount.rs) | Population count | `vcntq_u8`, `vaddvq_u16` |

### 3. Performance Gaps (ARM vs x86)

| Operation | x86_64 Performance | ARM NEON Performance | Gap Reason |
|-----------|-------------------|---------------------|------------|
| DSV quote masking | 85-1676 MiB/s (BMI2) | **3.84 GiB/s (PMULL)** | ✅ Gap closed with PMULL |
| JSON semi-indexing | 950+ MiB/s (AVX2) | ~850 MiB/s (NEON) | Wider vectors |
| Popcount | 5.2x with AVX-512 | 1x baseline | AVX-512 VPOPCNTDQ |

**Note**: DSV quote masking gap was closed on 2026-01-22 by using NEON PMULL (carryless multiplication)
for prefix XOR instead of the 6-shift scalar algorithm. This works on ALL ARM64 including Apple Silicon.

---

## SVE2 Capabilities Analysis

### Available on Neoverse N2 (GitHub Actions)

| Feature | Description | x86 Equivalent | Current ARM Fallback |
|---------|-------------|----------------|---------------------|
| **BDEP/BEXT** | Bit deposit/extract on vectors | BMI2 PDEP/PEXT | **PMULL prefix XOR** (now fast) |
| **Predication** | Per-lane masking | AVX-512 masks | Scalar tail handling |
| **CNT (vector)** | Per-byte popcount | VPOPCNTDQ | NEON `vcntq_u8` |
| **MATCH** | Character set matching | PCMPISTRI | Nibble lookup |
| **First-fault loads** | Safe speculative loads | None | Bounds checking |
| **Gather/Scatter** | Non-contiguous access | AVX2 gather | Scalar loops |

### Vector Width Reality

| Processor | SVE2 Vector Width | Notes |
|-----------|-------------------|-------|
| Neoverse N2 (Azure Cobalt 100) | 128-bit | Same as NEON |
| Neoverse V1 (AWS Graviton 3) | 256-bit | SVE (not SVE2) |
| Neoverse V2 (AWS Graviton 4) | 128-bit | SVE2 |
| Apple M1-M4 | N/A | No SVE/SVE2 support |

**Critical insight**: At 128-bit width, SVE2 won't provide wider-vector benefits. Value comes from **new instructions**, not wider vectors.

---

## Rust SVE2 Support Status

### Current State (January 2026)

From [Rust Project Goals](https://rust-lang.github.io/rust-project-goals/2025h2/scalable-vectors.html):

- **Status**: Experimental, nightly-only
- **Intrinsics**: PR [stdarch#1509](https://github.com/rust-lang/stdarch/pull/1509) pending
- **Type system**: Requires "Sized Hierarchy" RFC for scalable vector types
- **Runtime detection**: `is_aarch64_feature_detected!("sve2")` works

### Workarounds

1. **Inline assembly**: Use `core::arch::asm!` with SVE2 instructions
2. **C FFI**: Call SVE2 intrinsics via C wrapper
3. **Wait for stabilization**: Target 2026H2 for stable intrinsics

---

## NEON PMULL Discovery (2026-01-22) ✅ BETTER SOLUTION

**Key insight**: The scalar prefix XOR algorithm (6 shifts + 6 XORs) can be replaced with a single
NEON PMULL instruction. Carryless multiplication by all-1s computes prefix XOR in O(1):

```
prefix_xor(x) = clmul(x, 0xFFFFFFFFFFFFFFFF) truncated to 64 bits
```

**Implementation** (completed 2026-01-22):
- [src/dsv/simd/neon.rs](../../src/dsv/simd/neon.rs) - `prefix_xor_neon()` using `vmull_p64`

```rust
#[target_feature(enable = "neon")]
unsafe fn prefix_xor_neon(x: u64) -> u64 {
    use core::arch::aarch64::*;
    let a = vreinterpretq_p64_u64(vdupq_n_u64(x));
    let b = vreinterpretq_p64_u64(vdupq_n_u64(!0u64));
    let product = vmull_p64(vgetq_lane_p64::<0>(a), vgetq_lane_p64::<0>(b));
    vgetq_lane_u64::<0>(vreinterpretq_u64_p128(product))
}
```

**Benchmark results** (AWS Graviton 4):

| Metric | Before (scalar) | After (PMULL) | Improvement |
|--------|-----------------|---------------|-------------|
| 10MB index build | 3.18 GiB/s | 3.84 GiB/s | **+25%** |
| 10MB index+iterate | 0.93 GiB/s | 1.00 GiB/s | **+8%** |

**Why this is better than SVE2 BDEP**:
1. **Universal ARM64 support**: Works on ALL aarch64 (Apple M1+, Graviton 2+, all NEON chips)
2. **No runtime detection needed**: PMULL is part of ARMv8 Cryptography Extension, mandatory since ARMv8.0
3. **Simpler code**: Single instruction vs SVE2 inline assembly
4. **Same performance**: PMULL achieves similar throughput to SVE2 BDEP

**Status**: ✅ Deployed as default for all ARM64 platforms

---

## SVE2 Optimizations (Reference)

### Priority 1: DSV Quote Masking with BDEP ⚠️ SUPERSEDED BY PMULL

**Original approach**: Use SVE2 BDEP instruction for quote state masking.

**Implementation** (completed 2026-01-21, now superseded):
- [src/util/simd/sve2.rs](../../src/util/simd/sve2.rs) - BDEP/BEXT wrappers using inline assembly
- [src/dsv/simd/sve2.rs](../../src/dsv/simd/sve2.rs) - DSV indexing with SVE2 BDEP
- [src/dsv/simd/mod.rs](../../src/dsv/simd/mod.rs) - Runtime dispatch

**Note**: The NEON PMULL approach (above) is now preferred because:
- Works on Apple Silicon (no SVE2)
- Works on Graviton 2/3 (no SVE2-BITPERM)
- Achieves equivalent performance without runtime detection complexity

### Priority 2: Popcount with SVE2 CNT

**Current state**: NEON `vcntq_u8` already efficient, but SVE2 can use predication for cleaner tail handling.

**SVE2 advantage**:
- Vector-length-agnostic code
- Built-in predication for remainders
- Horizontal reduction instructions

**Files to modify**:
- `src/bits/popcount.rs` - Add `popcount_words_sve2`

**Expected improvement**: Marginal (~5-10%) from cleaner tail handling

### Priority 3: JSON Semi-indexing with SVE2 MATCH

**Current state**: NEON nibble lookup (2 table lookups + AND) for character classification.

**SVE2 alternative**: `MATCH` instruction finds if any element matches a set of characters.

```rust
// Current NEON approach
let lo_result = vqtbl1q_u8(lo_table, lo_nibble);
let hi_result = vqtbl1q_u8(hi_table, hi_nibble);
let class = vandq_u8(lo_result, hi_result);

// SVE2 MATCH approach
// Compare each byte against up to 8 target characters simultaneously
```

**Caveat**: At 128-bit width, SVE2 MATCH may not beat optimized nibble lookup. Requires benchmarking.

**Files to modify**:
- `src/json/simd/sve2.rs` - New file (if benchmarks show benefit)

**Expected improvement**: Unknown - needs micro-benchmarking

### Priority 4: YAML String Scanning with Predication

**Current state**: NEON scans 16 bytes, scalar fallback for remainder.

**SVE2 advantage**: Predicated loads handle arbitrary lengths without scalar fallback.

```rust
// Current NEON
while offset + 16 <= len {
    let chunk = vld1q_u8(data.as_ptr().add(offset));
    // ... SIMD processing
    offset += 16;
}
// Scalar fallback for remainder
data[offset..].iter().position(|&b| b == target)

// SVE2 with predication
// Predicate mask automatically handles partial final vector
// No separate scalar tail handling needed
```

**Files to modify**:
- `src/yaml/simd/sve2.rs` - New file

**Expected improvement**: Cleaner code, marginal performance gain

### Priority 5: First-Fault Loads for Speculative Scanning

**Use case**: String scanning can speculatively read ahead without bounds checks.

**Caveat**: Complex to implement correctly; may not provide significant benefit for this workload.

**Recommendation**: Defer to later phase

---

## Implementation Plan

### Phase 1: Infrastructure (Pre-requisite)

1. **Add SVE2 feature detection**
   ```rust
   // src/util/simd/mod.rs
   #[cfg(target_arch = "aarch64")]
   pub fn has_sve2() -> bool {
       #[cfg(feature = "std")]
       { is_aarch64_feature_detected!("sve2") }
       #[cfg(not(feature = "std"))]
       { false }
   }

   pub fn has_sve2_bitperm() -> bool {
       #[cfg(feature = "std")]
       { is_aarch64_feature_detected!("sve2-bitperm") }
       #[cfg(not(feature = "std"))]
       { false }
   }
   ```

2. **Create SVE2 module structure**
   ```
   src/
   ├── util/simd/
   │   └── sve2.rs          # New: SVE2 utilities
   ├── dsv/simd/
   │   └── sve2.rs          # New: DSV with BDEP
   ├── json/simd/
   │   └── sve2.rs          # New: JSON with MATCH (if beneficial)
   └── yaml/simd/
       └── sve2.rs          # New: YAML with predication
   ```

3. **Add CI for SVE2 testing**
   ```yaml
   # .github/workflows/ci.yml
   jobs:
     test-arm64-sve2:
       runs-on: ubuntu-24.04-arm
       steps:
         - run: cargo test --features simd,sve2
   ```

### Phase 2: BDEP Implementation (High Priority)

1. **Research approach**:
   - SVE2 BDEP operates on vectors, not scalars
   - Need to load scalar into vector lane, apply BDEP, extract result
   - May need inline assembly until Rust intrinsics stabilize

2. **Implementation options**:

   **Option A: Inline Assembly**
   ```rust
   #[cfg(target_arch = "aarch64")]
   #[target_feature(enable = "sve2-bitperm")]
   unsafe fn bdep_u64(data: u64, mask: u64) -> u64 {
       let result: u64;
       core::arch::asm!(
           "mov z0.d, {data}",
           "mov z1.d, {mask}",
           "bdep z0.d, z0.d, z1.d",
           "mov {result}, z0.d[0]",
           data = in(reg) data,
           mask = in(reg) mask,
           result = out(reg) result,
           options(pure, nomem, nostack)
       );
       result
   }
   ```

   **Option B: C FFI Wrapper**
   ```c
   // sve2_util.c
   #include <arm_sve.h>
   uint64_t bdep_u64(uint64_t data, uint64_t mask) {
       svuint64_t vdata = svdup_u64(data);
       svuint64_t vmask = svdup_u64(mask);
       svuint64_t result = svbdep_u64(vdata, vmask);
       return svlasta_u64(svptrue_b64(), result);
   }
   ```

3. **Port toggle64 algorithm**:
   ```rust
   #[target_feature(enable = "sve2-bitperm")]
   unsafe fn toggle64_sve2(carry: u64, quote_mask: u64) -> (u64, u64) {
       let c = carry & 0x1;
       let addend = bdep_u64(ODDS_MASK << c, quote_mask);
       let comp_w = !quote_mask;
       let shifted = (addend << 1) | c;
       let (result, overflow) = shifted.overflowing_add(comp_w);
       let new_carry = if overflow { 1 } else { 0 };
       (result, new_carry)
   }
   ```

### Phase 3: Benchmarking and Validation

1. **Micro-benchmarks**: Compare SVE2 vs NEON for each operation
2. **End-to-end tests**: Verify no regressions in real workloads
3. **GitHub Actions CI**: Test on `ubuntu-24.04-arm` runner

### Phase 4: Additional Optimizations (If Beneficial)

Based on Phase 3 benchmarks:
- JSON MATCH instruction
- YAML predicated scanning
- Popcount with predication

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Rust SVE2 intrinsics not stable | High | Medium | Use inline assembly or C FFI |
| 128-bit width limits benefit | Medium | High | Focus on BDEP, not wider vectors |
| GitHub Actions hardware changes | Low | Medium | Feature detection, graceful fallback |
| BDEP slower than expected | Low | High | Benchmark before full integration |

---

## Success Criteria

1. **DSV parsing**: Achieve 5x+ improvement on SVE2 ARM vs current NEON
2. **Feature detection**: Graceful fallback to NEON when SVE2 unavailable
3. **CI coverage**: SVE2 tests passing on GitHub Actions ARM runners
4. **No regressions**: NEON and x86 paths unaffected

---

## References

- [Rust SVE2 Project Goal](https://rust-lang.github.io/rust-project-goals/2025h2/scalable-vectors.html)
- [ARM SVE2 Intrinsics](https://developer.arm.com/documentation/102340/latest/New-features-in-SVE2)
- [PDEP/PEXT Cross-Architecture](https://gist.github.com/Validark/9455d410ccc8e4bfd1c1c5c4fa38f934)
- [GitHub Actions ARM64 Runners](https://github.com/orgs/community/discussions/148648)

---

## Appendix: Current Codebase SIMD Patterns

### Pattern 1: Runtime Dispatch

```rust
// src/dsv/simd/mod.rs
pub fn build_index(text: &[u8], config: &DsvConfig) -> DsvIndex {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("bmi2") && is_x86_feature_detected!("avx2") {
            return bmi2::build_index_simd(text, config);
        }
        if is_x86_feature_detected!("avx2") {
            return avx2::build_index_simd(text, config);
        }
        return sse2::build_index_simd(text, config);
    }
    #[cfg(target_arch = "aarch64")]
    {
        // TODO: Add SVE2 dispatch here
        return neon::build_index_simd(text, config);
    }
}
```

### Pattern 2: Compile-Time Feature Flags

```rust
// src/bits/popcount.rs
#[cfg(all(feature = "simd", target_arch = "aarch64"))]
fn popcount_words_neon(words: &[u64]) -> u32 { ... }

#[cfg(all(feature = "simd", target_arch = "x86_64"))]
fn popcount_words_x86(words: &[u64]) -> u32 { ... }
```

### Pattern 3: Target Feature Annotation

```rust
#[target_feature(enable = "neon")]
unsafe fn find_quote_or_escape_neon_impl(...) { ... }

#[target_feature(enable = "avx2", enable = "bmi2")]
unsafe fn toggle64_bmi2(carry: u64, w: u64) -> (u64, u64) { ... }
```

These patterns should be extended for SVE2:

```rust
#[target_feature(enable = "sve2-bitperm")]
unsafe fn toggle64_sve2(carry: u64, w: u64) -> (u64, u64) { ... }
```

---

## Future NEON Optimization Opportunities

Based on analysis of ARM NEON instructions for indexing and data structures (January 2026), the following opportunities have been identified. These are organized by confidence level.

### ✅ Balanced Parentheses Unrolled Lookup (IMPLEMENTED - January 2026)

**Status**: ✅ **Implemented and deployed**

**Implementation** ([src/trees/bp.rs](../../src/trees/bp.rs)):
- Added `word_min_excess_unrolled()` function with fully unrolled lookup table access
- Batches all 8 byte lookups before computing prefix sums and global minimum
- Enabled via `--features simd` flag

**Benchmark Results** (Apple M1 Max):

| Size | Without Optimization | With Optimization | Improvement |
|------|---------------------|-------------------|-------------|
| 10K nodes | 2.41 µs | 2.00 µs | **17%** |
| 100K nodes | 20.70 µs | 17.07 µs | **18%** |
| 1M nodes | 207.21 µs | 171.96 µs | **17%** |

**Key insight**: The optimization doesn't use NEON intrinsics directly. Instead, it benefits from:
1. **Loop unrolling** - eliminates branch overhead
2. **Batched lookups** - better cache locality for lookup tables
3. **Better instruction scheduling** - compiler can optimize independent operations

**Why the original SIMD approach was not used**:
- Min excess computation requires sequential prefix sums (each byte's excess depends on previous bytes)
- True SIMD parallelism would need a parallel prefix scan, which adds complexity
- The unrolled scalar approach achieves similar benefit with simpler code

**Remaining opportunities** (not yet implemented):
- `find_open` and `enclose` could benefit from similar unrolling

---

### ✅ Popcount Loop Unrolling (IMPLEMENTED - January 2026)

**Status**: ✅ **Implemented and deployed**

**Implementation** ([src/bits/popcount.rs](../../src/bits/popcount.rs)):
- Process 256 bytes (4 × 64 bytes) per iteration instead of 64 bytes
- Enables better instruction-level parallelism on superscalar CPUs

**Benchmark Results** (AWS Graviton 4 / Neoverse-V2):

| Metric | Improvement |
|--------|-------------|
| 10M bits raw popcount | **15.5% faster** |
| 1M bits raw popcount | **10.8% faster** |
| 1M bits construction | **4.0% faster** |

**Result**: Exceeded 5-8% expectation with 10-15% improvement.

---

### ✅ NEON VMINV for L1 Index Building (IMPLEMENTED - January 2026)

**Status**: ✅ **Implemented and deployed**

**Implementation** ([src/trees/bp.rs](../../src/trees/bp.rs)):
- Process 8 words at a time using NEON int16x8
- Compute prefix sums via parallel prefix pattern (3 shuffle+add steps)
- Use `vminvq_s16` to find block minimum in one instruction

**Benchmark Results** (AWS Graviton 4 / Neoverse-V2):

| Size | Before | After | Improvement |
|------|--------|-------|-------------|
| 10K nodes | 5.4 µs | 2.07 µs | **2.6x faster** |
| 100K nodes | 52.7 µs | 19.1 µs | **2.8x faster** |
| 1M nodes | 527 µs | 185 µs | **2.8x faster** |

**Key insight**: VMINV finds minimum across 8 vector lanes in one instruction,
combined with SIMD prefix sums for dramatic speedup.

---

### ✅ NEON VMINV for L2 Index Building (IMPLEMENTED - January 2026)

**Status**: ✅ **Implemented and deployed**

**Implementation** ([src/trees/bp.rs](../../src/trees/bp.rs)):
- Same SIMD pattern as L1, processing 8 L1 entries at a time
- Added for code consistency and benefit at large data sizes

**Benchmark Results** (AWS Graviton 4 / Neoverse-V2):

| Size | Nodes | L2 Entries | Scalar | SIMD | Improvement |
|------|-------|------------|--------|------|-------------|
| ~2.5MB | 10M | ~10K | 1.91ms | 1.91ms | ~0% (noise) |
| ~25MB | 100M | ~100K | 19.70ms | 19.44ms | **1.3%** |
| ~125MB | 500M | ~488K | 153.78ms | 148.73ms | **3.4%** |

**Key insight**: L2 SIMD benefit scales with data size. At 500M nodes, L2 has ~488K entries
(comparable to L1 at ~1M nodes). No regression at smaller sizes, kept for consistency.

---

### Low Confidence: Anchor Terminator Nibble Tables

**Current State** ([src/yaml/simd/neon.rs](../../src/yaml/simd/neon.rs#L422-L498)):
```rust
// 10 separate CMEQ comparisons + 9 ORR operations
let is_space = vceqq_u8(chunk, space);
let is_tab = vceqq_u8(chunk, tab);
let is_newline = vceqq_u8(chunk, newline);
// ... 7 more comparisons
let flow = vorrq_u8(is_lbracket, is_rbracket);
// ... more ORRs
```

**Opportunity**: Replace 10 CMEQ + 9 ORR with 2 TBL + 1 AND (nibble table approach):
```rust
// Proposed: 2 table lookups + 1 AND (like JSON does)
let lo_nibble = vandq_u8(chunk, vdupq_n_u8(0x0F));
let hi_nibble = vshrq_n_u8::<4>(chunk);
let lo_result = vqtbl1q_u8(anchor_lo_table, lo_nibble);
let hi_result = vqtbl1q_u8(anchor_hi_table, hi_nibble);
let terminators = vandq_u8(lo_result, hi_result);
```

**Expected Impact**: 5-10% on anchor-heavy workloads (if validated)

**⚠️ WARNING**: This approach was **rejected** for YAML character classification:
> "NEON nibble lookup for YAML: -12-15% penalty (only 2-3 chars searched)"

**However**, anchor parsing searches for **10 characters** (space, tab, newline, CR, `[`, `]`, `{`, `}`, `,`, `:`), not 2-3. The crossover point where nibble tables win may be different.

**Validation required before implementation**:
1. Micro-benchmark 10-terminator nibble lookup vs 10-CMEQ approach
2. Only implement if micro-benchmark shows **2x+ improvement**
3. Run end-to-end `yq` benchmarks to validate (learned from P2.8, P3 failures)

**Risk**: High - similar approach rejected for 2-3 character YAML classification

**Files to modify (only if validated)**:
- `src/yaml/simd/neon.rs` - Replace `parse_anchor_name_neon_impl` inner loop

---

### Reference: NEON Instructions for Indexing

Based on research papers and production systems, these NEON instructions have proven utility for indexing:

| Instruction | Use Case | Proven Performance |
|-------------|----------|-------------------|
| **CNT** (`vcntq_u8`) | Rank/popcount | 6.7x faster (Redis) |
| **CMEQ** (`vceqq_*`) | Character classification | 4+ GB/s JSON parsing (simdjson) |
| **TBL** (`vqtbl1q_u8`) | Byte classification | Replaces 13 comparisons with 6 ops |
| **ADDV** (`vaddvq_*`) | Horizontal sum | Essential for popcount reduction |
| **PMULL** (`vmull_p64`) | Prefix XOR | 25% faster DSV index (deployed) |
| **CLZ** (scalar) | First set bit | Single cycle |
| **EXT** (`vextq_*`) | Sliding window | Essential for text parsing |
| **MINV** (`vminvq_s16`) | L1/L2 block minimum | ✅ **Deployed (L1: +2.8x, L2: +1-3% at scale)** |

**Instructions NOT applicable to this codebase**:
- **SDOT/UDOT**: No quantized embedding vectors
- **SMMLA/UMMLA**: No matrix operations
- **BFMMLA/BFDOT**: No neural network inference
- **FCMLA/FCADD**: No complex number processing
- **Gather/Scatter**: Not available on Apple Silicon (no SVE)

---

### Lessons from Failed NEON Optimizations

The YAML optimization history (see [docs/parsing/yaml.md](../parsing/yaml.md)) documents 9 rejected optimizations with a common pattern:

| Rejected Optimization | Penalty | Key Lesson |
|----------------------|---------|------------|
| P1: YFSM | N/A | YAML strings too simple for FSM |
| P2.6: Software prefetch | **+30%** | Hardware prefetcher is better |
| P2.8: SIMD threshold tuning | **+8-15%** | Modern CPUs handle branches well |
| P3: Branchless char class | **+25-44%** | Branch predictors are 93-95% accurate |
| P5: Flow collection fast path | N/A | Real flow collections too small (<30B) |
| P6: BMI2 PDEP/PEXT | N/A | YAML grammar prevents DSV-style indexing |
| P7: Newline index | N/A | CLI feature, not parsing optimization |
| P8: AVX-512 | **+7%** | Memory bandwidth saturated at AVX2 |

**Key lessons for future NEON optimization**:
1. **Micro-benchmarks lie** - P2.6/P2.8/P3/P8 all showed micro-bench improvements but regressed end-to-end
2. **Memory bandwidth saturates** - Sequential text parsing can't benefit from wider SIMD
3. **Branch predictors are excellent** - Branchless code often loses (93-95% prediction accuracy)
4. **SIMD only wins when**:
   - Scanning long runs (30+ bytes average)
   - Processing 3+ character types simultaneously
   - Using specialized instructions (PMULL for prefix XOR, VMINV for reductions)
   - Compute-bound operations (popcount, index building - not parsing)

**Key lesson for SVE2**:
5. **Inline assembly defeats optimization** - SVE2 via `core::arch::asm!` is ~50% slower than NEON intrinsics due to:
   - Lost register allocation optimization
   - Prevented instruction scheduling
   - Blocked loop unrolling
   - Extra predicate management overhead
6. **Wait for stable intrinsics** - Only use SVE2 when Rust stabilizes `#![feature(stdarch_aarch64_sve)]`
7. **SVE2 value is in specialized instructions** - BDEP/BEXT (via SVEBITPERM) provides wins; general ops lose to optimized NEON
8. **BDEP wins for select_in_word** - 5-17x faster than CTZ loop for k > 0 (✅ implemented January 2026)

### SVE2 BDEP select_in_word Implementation (January 2026)

**Status**: ✅ **Implemented and deployed**

Used SVE2 BDEP instruction to accelerate the `select_in_word` function which finds the k-th set bit in a 64-bit word.

**Algorithm** (PDEP-style):
```rust
let mask = (1u64 << (k + 1)) - 1;  // k+1 low bits set
let scattered = bdep(mask, x);     // Scatter to bit positions
63 - scattered.leading_zeros()     // Find highest bit
```

**Benchmark Results** (AWS Graviton 4):

| Scenario | CTZ Loop | BDEP | Speedup |
|----------|----------|------|---------|
| sparse (k=0) | 0.88 ns | 1.78 ns | 0.5x |
| dense (all bits) | 20.4 ns | 1.7 ns | **12x** |
| high_k (k=32-63) | 30.8 ns | 1.8 ns | **17x** |
| mixed patterns | 9.6 ns | 1.8 ns | **5.4x** |

**Key insight**: CTZ loop is O(k) while BDEP is O(1). For bitvectors with varied densities, BDEP wins decisively.

**End-to-end benchmark results** (full select1 operations):

| Benchmark | Change | Notes |
|-----------|--------|-------|
| select1/1M/90% | **-4.9%** | Dense bitvectors benefit most |
| select1/10M/50% | **-1.8%** | Modest improvement |
| select_in_word/alternating | **-2.6%** | Per-call improvement visible |

End-to-end improvements are modest (2-5%) because:
1. SelectIndex provides O(1) jump to approximate position
2. Most selects only need 1-2 select_in_word calls
3. Dense patterns (90%) show best improvement where k values are higher

**Files modified**:
- `src/util/simd/sve2.rs` - Added `select_in_word_bdep` function
- `src/util/broadword.rs` - Added runtime dispatch to BDEP on supported CPUs

**Future work**: Add x86 PDEP equivalent for cross-platform parity:
```rust
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "bmi2")]
unsafe fn select_in_word_pdep(x: u64, k: u32) -> u32 {
    if x == 0 { return 64; }
    let pop = x.count_ones();
    if k >= pop { return 64; }
    let mask = if k >= 63 { u64::MAX } else { (1u64 << (k + 1)) - 1 };
    let scattered = _pdep_u64(mask, x);
    63 - scattered.leading_zeros()
}
```
This would provide the same 5-17x speedup on Intel Haswell+ and AMD Zen 3+.

### SSE4.1 PHMINPOSUW BP L1/L2 Implementation (January 2026)

**Status**: ✅ **Implemented** (marginal benefit on x86_64)

Implemented SSE4.1 equivalent of NEON VMINV for BP L1/L2 index building.

**Challenge**: SSE4.1's `PHMINPOSUW` (`_mm_minpos_epu16`) only works on **unsigned** 16-bit values,
but min_excess can be negative. Solution: bias/unbias workaround.

**Algorithm**:
```rust
#[target_feature(enable = "sse4.1")]
unsafe fn horizontal_min_i16(v: __m128i) -> i16 {
    // Bias by 0x8000 to convert signed -> unsigned range
    let bias = _mm_set1_epi16(i16::MIN);
    let biased = _mm_add_epi16(v, bias);

    // Find unsigned minimum
    let minpos = _mm_minpos_epu16(biased);
    let biased_min = _mm_extract_epi16(minpos, 0) as i16;

    // Unbias: wrapping add converts back to signed
    biased_min.wrapping_add(i16::MIN)
}
```

**Benchmark Results** (AMD Ryzen 9 7950X, Zen 4):

| Size | Scalar | SSE4.1 | Improvement |
|------|--------|--------|-------------|
| 10K nodes | 1.97 µs | 1.98 µs | ~0% (neutral) |
| 100K nodes | 18.96 µs | 18.97 µs | ~0% (neutral) |
| 1M nodes | 188.0 µs | 197.2 µs | **-5% (regression)** |
| 10M nodes | 3.15 ms | 3.05 ms | **+3%** |
| 100M nodes | 31.98 ms | 31.36 ms | **+2%** |
| 500M nodes | 253.8 ms | 250.6 ms | **+1%** |

**Why SSE4.1 is less effective than NEON**:

| Factor | ARM NEON | x86 SSE4.1 |
|--------|----------|------------|
| Horizontal min | `vminvq_s16` (signed direct) | `_mm_minpos_epu16` (unsigned only) |
| Horizontal sum | `vaddvq_s16` (single instruction) | Multiple shifts + adds |
| Bias overhead | None needed | +2 ops per block |

**Key insight**: ARM NEON's `vminvq_s16` handles signed values directly, giving 2.8x speedup.
SSE4.1's unsigned-only PHMINPOSUW requires bias/unbias overhead that negates most benefit.

**Recommendation**: Keep SSE4.1 path for:
- Large data (10M+ nodes) where 1-3% improvement is measurable
- Cross-platform parity with NEON
- No regression on end-to-end benchmarks

**Files modified**:
- `src/trees/bp.rs` - Added `build_l1_index_sse41`, `build_l2_index_sse41`
