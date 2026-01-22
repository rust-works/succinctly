# SVE2 Optimization Plan for Succinctly

**Status**: Updated 2026-01-22 - NEON PMULL approach outperforms SVE2 for most ARM platforms

## Executive Summary

This document analyzes all current x86 and ARM NEON optimizations in the Succinctly codebase and plans SVE2 equivalents for ARM CPUs that support it (Azure Cobalt 100/Neoverse N2, AWS Graviton 3+, etc.).

**Key finding**: SVE2 on Neoverse N2 (GitHub Actions ARM runners) has **128-bit vectors** - the same width as NEON. Therefore, SVE2's value comes from:
1. **BDEP/BEXT instructions** - equivalent to x86 BMI2 PDEP/PEXT (currently software-emulated on ARM)
2. **Predicated operations** - cleaner handling of tail bytes without scalar fallbacks
3. **First-fault loads** - potential for speculative scanning

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
