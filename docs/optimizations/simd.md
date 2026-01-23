# SIMD (Single Instruction, Multiple Data)

SIMD enables parallel processing of multiple data elements with a single instruction. Modern CPUs provide various SIMD extensions with different vector widths and capabilities.

## Overview

| Extension    | Width   | Platform | Key Instructions           | Best For              |
|--------------|---------|----------|----------------------------|-----------------------|
| SSE2         | 128-bit | x86_64   | Basic vector ops           | Baseline              |
| SSE4.2       | 128-bit | x86_64   | PCMPISTRI, POPCNT          | String matching       |
| AVX2         | 256-bit | x86_64   | Wide vectors, gather       | Memory-bound work     |
| AVX-512      | 512-bit | x86_64   | VPOPCNTDQ, masked ops      | Compute-bound only    |
| BMI2         | 64-bit  | x86_64   | PDEP, PEXT                 | Bit manipulation      |
| NEON         | 128-bit | ARM64    | TBL, CNT, parallel ops     | ARM baseline          |
| PMULL        | 64→128  | ARM64    | Carryless multiply         | Prefix XOR (DSV)      |
| SVE2-BITPERM | 64-bit  | ARM64    | BDEP, BEXT                 | Bit deposit/extract   |

---

## x86_64 SIMD Hierarchy

### SSE2 (Baseline)

Available on all x86_64 CPUs. 128-bit vectors (16 bytes).

```rust
use core::arch::x86_64::*;

unsafe fn classify_sse2(data: &[u8; 16]) -> u16 {
    let chunk = _mm_loadu_si128(data.as_ptr() as *const __m128i);

    // Compare for specific characters
    let quotes = _mm_cmpeq_epi8(chunk, _mm_set1_epi8(b'"' as i8));
    let backslash = _mm_cmpeq_epi8(chunk, _mm_set1_epi8(b'\\' as i8));

    // Extract to bitmask
    let mask = _mm_or_si128(quotes, backslash);
    _mm_movemask_epi8(mask) as u16
}
```

**Key instructions**:
- `_mm_cmpeq_epi8`: Compare 16 bytes in parallel
- `_mm_movemask_epi8`: Extract comparison results to 16-bit mask
- `_mm_loadu_si128`: Unaligned 128-bit load

### SSE4.2

Adds string processing instructions.

```rust
use core::arch::x86_64::*;

unsafe fn find_structural_sse42(data: &[u8; 16]) -> u16 {
    let chunk = _mm_loadu_si128(data.as_ptr() as *const __m128i);

    // Find any of these characters: {}[]:,"
    let chars = _mm_setr_epi8(
        b'{' as i8, b'}' as i8, b'[' as i8, b']' as i8,
        b':' as i8, b',' as i8, b'"' as i8, 0, 0, 0, 0, 0, 0, 0, 0, 0
    );

    _mm_cmpistrm(
        chars, chunk,
        _SIDD_UBYTE_OPS | _SIDD_CMP_EQUAL_ANY | _SIDD_BIT_MASK
    ) as u16
}
```

**Result**: 38% faster character classification vs SSE2.

### AVX2

256-bit vectors (32 bytes). Best for memory-bound workloads.

```rust
use core::arch::x86_64::*;

unsafe fn classify_avx2(data: &[u8; 32]) -> u32 {
    let chunk = _mm256_loadu_si256(data.as_ptr() as *const __m256i);

    // Character classification
    let quotes = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'"' as i8));
    let colon = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b':' as i8));
    let comma = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b',' as i8));
    // ... more characters ...

    let structural = _mm256_or_si256(quotes, _mm256_or_si256(colon, comma));
    _mm256_movemask_epi8(structural) as u32
}
```

**Result in succinctly**: 78% faster JSON parsing vs SSE2.

### AVX-512

512-bit vectors. **Use with caution** - not always faster.

```rust
use core::arch::x86_64::*;

#[target_feature(enable = "avx512vpopcntdq")]
unsafe fn popcount_avx512(words: &[u64; 8]) -> u64 {
    let data = _mm512_loadu_si512(words.as_ptr() as *const i32);
    let counts = _mm512_popcnt_epi64(data);
    _mm512_reduce_add_epi64(counts) as u64
}
```

**When AVX-512 wins**: Compute-bound operations like popcount (5.2x faster).

**When AVX-512 loses**: Memory-bound operations like JSON parsing (7-17% slower) and YAML parsing (7% slower at 64B).

**Why it can be slower**:
1. **Memory bandwidth bottleneck**: Sequential text parsing saturates RAM bandwidth at AVX2 width already
2. **Zen 4 splits AVX-512 ops**: Physical execution units are 256-bit wide, so 512-bit ops split into two 256-bit micro-ops (overhead without benefit)
3. **Higher power → frequency throttling**: CPU may reduce clock speed with AVX-512
4. **Benchmark design matters**: Measuring loop iterations vs actual work can show misleading "wins"

**Lessons from YAML P8 rejection (2026-01-18)**:
- AVX-512 showed 7% regression at realistic 64B chunk size
- Apparent "2x wins" at 256B+ were benchmark artifacts (half the loop iterations, not twice the efficiency)
- Real parsing scans linearly → AVX2 and AVX-512 do same total work, but AVX-512 adds overhead
- **Pattern**: When JSON AVX-512 failed (7-17% slower), YAML AVX-512 also failed (same memory-bound workload)
- See [docs/parsing/yaml.md](../parsing/yaml.md#p8-avx-512-variants---rejected-) for full analysis

---

## ARM NEON

128-bit vectors, always available on ARM64.

### Basic Operations

```rust
use core::arch::aarch64::*;

unsafe fn classify_neon(data: &[u8; 16]) -> u16 {
    let chunk = vld1q_u8(data.as_ptr());

    // Character comparisons
    let quotes = vceqq_u8(chunk, vdupq_n_u8(b'"'));
    let backslash = vceqq_u8(chunk, vdupq_n_u8(b'\\'));

    let mask = vorrq_u8(quotes, backslash);
    neon_movemask(mask)
}
```

### NEON Movemask (Custom)

NEON lacks `movemask`. Use multiplication trick:

```rust
unsafe fn neon_movemask(mask: uint8x16_t) -> u16 {
    // Extract high bit of each byte
    let high_bits = vshrq_n_u8(mask, 7);

    // Pack into u64 using magic multiplication
    let magic: u64 = 0x0102040810204080;

    let lo = vget_low_u8(high_bits);
    let hi = vget_high_u8(high_bits);

    let lo_sum = vaddv_u8(vmul_u8(lo, vcreate_u8(magic)));
    let hi_sum = vaddv_u8(vmul_u8(hi, vcreate_u8(magic)));

    ((hi_sum as u16) << 8) | (lo_sum as u16)
}
```

**Result**: 10-18% improvement over variable shifts.

### NEON Table Lookup (vqtbl1q_u8)

16 parallel lookups from a 16-entry table:

```rust
unsafe fn nibble_classify(chars: uint8x16_t) -> uint8x16_t {
    let lo_nibble = vandq_u8(chars, vdupq_n_u8(0x0F));
    let hi_nibble = vshrq_n_u8(chars, 4);

    let lo_table = vld1q_u8(LO_TABLE.as_ptr());
    let hi_table = vld1q_u8(HI_TABLE.as_ptr());

    let lo_result = vqtbl1q_u8(lo_table, lo_nibble);
    let hi_result = vqtbl1q_u8(hi_table, hi_nibble);

    vandq_u8(lo_result, hi_result)
}
```

**Result**: Replaces 13 comparisons with 6 operations.

### NEON PMULL for Prefix XOR (2026-01-22)

Carryless multiplication computes prefix XOR in O(1) instead of O(log n):

```rust
#[target_feature(enable = "neon")]
unsafe fn prefix_xor_neon(x: u64) -> u64 {
    use core::arch::aarch64::*;

    // Load x into polynomial register
    let a = vreinterpretq_p64_u64(vdupq_n_u64(x));
    let b = vreinterpretq_p64_u64(vdupq_n_u64(!0u64));

    // Carryless multiply: prefix_xor(x) = clmul(x, 0xFFFF...FFFF)
    let product = vmull_p64(vgetq_lane_p64::<0>(a), vgetq_lane_p64::<0>(b));

    vgetq_lane_u64::<0>(vreinterpretq_u64_p128(product))
}
```

**Why it works**: Carryless multiplication by all-1s propagates each bit to all higher positions via XOR,
which is exactly the prefix XOR operation (cumulative XOR from bit 0 to bit i).

**Result**: 25% faster DSV index building on ARM64 (3.18 → 3.84 GiB/s on Graviton 4).

**Platform support**: Works on ALL ARM64 (ARMv8 Cryptography Extension is mandatory since ARMv8.0).

### NEON Popcount

```rust
unsafe fn popcount_neon(data: &[u8; 64]) -> u32 {
    let mut sum = vdupq_n_u8(0);

    for chunk in data.chunks_exact(16) {
        let v = vld1q_u8(chunk.as_ptr());
        sum = vaddq_u8(sum, vcntq_u8(v));
    }

    // Horizontal sum with widening to avoid overflow
    let sum16 = vpaddlq_u8(sum);
    let sum32 = vpaddlq_u16(sum16);
    let sum64 = vpaddlq_u32(sum32);

    vgetq_lane_u64(sum64, 0) as u32 + vgetq_lane_u64(sum64, 1) as u32
}
```

### NEON Horizontal Reductions (VMINV, VMAXV, VADDV)

NEON provides horizontal reduction instructions that operate across all lanes:

```rust
use core::arch::aarch64::*;

unsafe fn find_min_across_vector(values: int16x8_t) -> i16 {
    vminvq_s16(values)  // Single instruction - finds minimum across 8 lanes
}

unsafe fn sum_across_vector(values: int16x8_t) -> i16 {
    vaddvq_s16(values)  // Single instruction - sums all 8 lanes
}
```

**Result in succinctly**: VMINV enables 2.8x faster BP index construction by finding
block minimums in a single instruction instead of a scalar loop.

**Key pattern**: Combine SIMD prefix sums with horizontal reduction:
1. Load 8 values
2. Compute prefix sums using parallel prefix (3 shuffle+add steps)
3. Add offset to all values
4. Use VMINV/VMAXV to find min/max in one instruction

---

## ARM SVE2-BITPERM (Graviton 4+)

SVE2-BITPERM provides BDEP (bit deposit) and BEXT (bit extract) instructions equivalent to x86 BMI2 PDEP/PEXT. Available on Neoverse-V2 (AWS Graviton 4) and Azure Cobalt 100.

### BDEP for select_in_word (2026-01-23)

The `select_in_word(x, k)` function finds the position of the k-th set bit in a 64-bit word. The BDEP implementation provides O(1) complexity vs O(k) for the CTZ loop:

```rust
#[target_feature(enable = "sve2-bitperm")]
pub unsafe fn select_in_word_bdep(x: u64, k: u32) -> u32 {
    if x == 0 { return 64; }
    let pop = x.count_ones();
    if k >= pop { return 64; }

    // Create mask with k+1 low bits set
    let mask = if k >= 63 { u64::MAX } else { (1u64 << (k + 1)) - 1 };

    // BDEP scatters the mask bits to positions where x has set bits
    let scattered = bdep_u64(mask, x);

    // Find highest set bit (the k-th bit position)
    63 - scattered.leading_zeros()
}
```

**Micro-benchmark results** (AWS Graviton 4):

| Pattern | CTZ Loop | BDEP | Speedup |
|---------|----------|------|---------|
| sparse (k=0) | 0.88 ns | 1.78 ns | 0.5x (CTZ wins) |
| dense (64 bits) | 20.4 ns | 1.7 ns | **12x** |
| high_k (k=32-63) | 30.8 ns | 1.8 ns | **17x** |
| mixed patterns | 9.6 ns | 1.8 ns | **5.4x** |

**End-to-end results** (full select1 operations):

| Benchmark | Change | Notes |
|-----------|--------|-------|
| select1/1M/90% | **-4.9%** | Dense bitvectors benefit most |
| select1/10M/50% | **-1.8%** | Modest improvement |

**Why modest end-to-end improvement**: SelectIndex provides O(1) jump to approximate position, so most selects only need 1-2 `select_in_word` calls. The optimization benefits dense patterns most.

**Platform support**: Requires SVE2-BITPERM (Graviton 4+). Falls back to CTZ loop on older CPUs.

---

## Runtime Feature Detection

### x86_64

```rust
fn select_simd_impl() -> impl Fn(&[u8]) -> Vec<u64> {
    if is_x86_feature_detected!("avx2") {
        avx2_impl
    } else if is_x86_feature_detected!("sse4.2") {
        sse42_impl
    } else {
        sse2_impl
    }
}
```

**Dispatch order**: AVX-512 VPOPCNTDQ → AVX2+BMI2 → AVX2 → SSE4.2 → SSE2

### ARM64

NEON is always available - no runtime detection needed:

```rust
#[cfg(target_arch = "aarch64")]
fn process(data: &[u8]) -> Vec<u64> {
    neon_impl(data)
}
```

---

## SIMD Patterns

### Character Classification

Process N bytes, produce N-bit mask:

```rust
// Input:  ['{', 'a', '"', 'b', '}', ...]
// Output: [1,   0,   1,   0,   1,   ...]  (structural chars)
```

**Pattern**:
1. Load vector
2. Compare against each target character
3. OR results together
4. Extract to bitmask

### Prefix Operations

Compute cumulative XOR/sum across vector:

```rust
fn prefix_xor(mut y: u64) -> u64 {
    y ^= y << 1;
    y ^= y << 2;
    y ^= y << 4;
    y ^= y << 8;
    y ^= y << 16;
    y ^= y << 32;
    y
}
```

**Application**: Quote state tracking in CSV/JSON parsing.

### Batch Processing

Process chunks larger than vector width:

```rust
// Process 64 bytes using 2x AVX2 (32-byte) loads
unsafe fn process_64_avx2(data: &[u8; 64]) -> u64 {
    let lo = _mm256_loadu_si256(data[0..32].as_ptr() as *const __m256i);
    let hi = _mm256_loadu_si256(data[32..64].as_ptr() as *const __m256i);

    let lo_mask = classify_avx2(lo);
    let hi_mask = classify_avx2(hi);

    (lo_mask as u64) | ((hi_mask as u64) << 32)
}
```

---

## Successful SIMD Optimizations

### YAML Unquoted Structural Skip (+3-8%)

**Pattern**: Use SIMD to find the next structural character (`\n`, `#`, `:`) in unquoted
values, then handle context-aware validation at scalar level.

```rust
// NEON implementation - find next \n, #, or :
let newline_vec = vdupq_n_u8(b'\n');
let hash_vec = vdupq_n_u8(b'#');
let colon_vec = vdupq_n_u8(b':');

while offset + 16 <= len {
    let chunk = vld1q_u8(data.as_ptr().add(offset));

    // Compare against all three targets
    let newlines = vceqq_u8(chunk, newline_vec);
    let hashes = vceqq_u8(chunk, hash_vec);
    let colons = vceqq_u8(chunk, colon_vec);

    // OR all results together
    let matches = vorrq_u8(vorrq_u8(newlines, hashes), colons);
    let mask = neon_movemask(matches);

    if mask != 0 {
        return Some(offset + mask.trailing_zeros() as usize);
    }
    offset += 16;
}
```

**Benchmark results** (Apple M1 Max):

| Size   | Before       | After        | Improvement     |
|--------|--------------|--------------|-----------------|
| 10 KB  | 28.9 µs      | 28.3 µs      | **+3.8%**       |
| 100 KB | 264 µs       | 257 µs       | **+5.2%**       |
| 1 MB   | 2.51 ms      | 2.33 ms      | **+8.3%**       |

### YAML Block Scalar SIMD (NEON) - 11-23% Improvement

**Pattern**: Use NEON to scan for newlines in block scalars, then verify indentation.

```rust
// NEON implementation - scan for newlines in 16-byte chunks
let newline_vec = vdupq_n_u8(b'\n');

while offset + 16 <= len {
    let chunk = vld1q_u8(data.as_ptr().add(offset));
    let newlines = vceqq_u8(chunk, newline_vec);
    let mask = neon_movemask(newlines);

    if mask != 0 {
        // Found newline - check indentation of next line
        let nl_pos = offset + mask.trailing_zeros() as usize;
        // ... indentation validation
    }
    offset += 16;
}
```

**Benchmark results** (Apple M1 Max - block scalar parsing):

| Benchmark                 | Before       | After        | Improvement     |
|---------------------------|--------------|--------------|-----------------|
| block_scalars/10x10       | 8.4 µs       | 7.5 µs       | **+10.9%**      |
| block_scalars/50x50       | 176 µs       | 152 µs       | **+13.8%**      |
| block_scalars/100x100     | 699 µs       | 606 µs       | **+13.3%**      |
| block_scalars/10x1000     | 693 µs       | 591 µs       | **+14.1%**      |
| block_scalars/long_10x100 | 172 µs       | 132 µs       | **+23.2%**      |
| block_scalars/long_50x100 | 786 µs       | 671 µs       | **+14.7%**      |

**Why it works**: Block scalars contain long runs of content where the only structural
character is the newline at line boundaries. SIMD efficiently skips content bytes.

### YAML Anchor/Alias SIMD (NEON) - 3-6% Improvement

**Pattern**: Use NEON to scan for anchor name terminators (space, newline, colon, etc.).

```rust
// NEON implementation - find anchor name end
let space_vec = vdupq_n_u8(b' ');
let newline_vec = vdupq_n_u8(b'\n');
let colon_vec = vdupq_n_u8(b':');

while offset + 16 <= len {
    let chunk = vld1q_u8(data.as_ptr().add(offset));

    let spaces = vceqq_u8(chunk, space_vec);
    let newlines = vceqq_u8(chunk, newline_vec);
    let colons = vceqq_u8(chunk, colon_vec);

    let matches = vorrq_u8(vorrq_u8(spaces, newlines), colons);
    let mask = neon_movemask(matches);

    if mask != 0 {
        return offset + mask.trailing_zeros() as usize;
    }
    offset += 16;
}
```

**Benchmark results** (Apple M1 Max - anchor-heavy YAML):

| Benchmark       | Before       | After        | Improvement     |
|-----------------|--------------|--------------|-----------------|
| anchors/10      | 3.76 µs      | 3.64 µs      | **+3.1%**       |
| anchors/100     | 28.8 µs      | 27.2 µs      | **+5.8%**       |
| anchors/1000    | 314 µs       | 299 µs       | **+4.7%**       |
| anchors/5000    | 1.67 ms      | 1.61 ms      | **+3.8%**       |
| anchors/k8s_10  | 8.25 µs      | 7.85 µs      | **+4.9%**       |
| anchors/k8s_50  | 32.5 µs      | 30.6 µs      | **+5.1%**       |
| anchors/k8s_100 | 64.1 µs      | 60.9 µs      | **+5.0%**       |

**Why it works**: Anchor names in Kubernetes-style YAML (like `&default-config`)
are typically 15-30 characters, making SIMD worthwhile for scanning to the terminator.

**Why it works** (unlike previous YAML SIMD attempts):

| Factor | Failed Attempts | This Approach |
|--------|-----------------|---------------|
| Target | `: ` (2-char pattern) | `\n`, `#`, `:` (1-char each) |
| Context handling | Tried SIMD | Scalar at found position |
| Typical scan distance | 5-15 bytes (keys) | 30-100+ bytes (values) |
| SIMD setup amortization | Below breakeven | Well above breakeven |

**Key insight**: SIMD works for finding the *next interesting byte* in potentially long
runs of uninteresting bytes. Context-sensitive checks (e.g., `#` needs preceding space,
`:` needs following whitespace) are handled efficiently at scalar level after the jump.

This avoids the trap that killed the colon-space detection: trying to validate context
in SIMD when the context check is simple and cheap at scalar level.

---

## Failed SIMD Optimizations

### AVX-512 JSON Parser (-10% penalty)

**Attempted**: Process 64 bytes per iteration with AVX-512.
**Result**: 7-17% slower than AVX2.
**Reason**: Memory-bound workload doesn't benefit from wider vectors.

### BMI1 Mask Iteration (-26% penalty)

**Attempted**: Use TZCNT/BLSR to skip non-structural bytes.
**Result**: 9-31% slower.
**Reason**: Optimized <1% of execution time; state machine needs all bytes.

### NEON Batched Popcount (-25% penalty)

**Attempted**: Batch popcount 8 words for cumulative index.
**Result**: 1.33x slower.
**Reason**: Prefix sum is inherently sequential; extraction overhead dominates.

### NEON Nibble Lookup for YAML (-12-15% penalty)

**Attempted**: Use nibble-based lookup tables for YAML string scanning.

The simdjson technique splits each byte into high/low nibbles (4 bits each), looks up both
in precomputed 16-entry tables, and ANDs the results to classify characters:

```rust
unsafe fn classify_yaml_chars(chunk: uint8x16_t) -> uint8x16_t {
    let lo_table = vld1q_u8(YAML_LO_NIBBLE.as_ptr());
    let hi_table = vld1q_u8(YAML_HI_NIBBLE.as_ptr());

    let lo_nibble = vandq_u8(chunk, vdupq_n_u8(0x0F));
    let hi_nibble = vshrq_n_u8::<4>(chunk);

    let lo_result = vqtbl1q_u8(lo_table, lo_nibble);
    let hi_result = vqtbl1q_u8(hi_table, hi_nibble);

    vandq_u8(lo_result, hi_result)  // Each byte has classification flags
}
```

**Result**: 12-15% slower than direct comparison for YAML.

**Benchmark data** (Apple M1, finding quote at end of string):

| Size | Nibble Lookup | Direct Comparison | Delta |
|------|---------------|-------------------|-------|
| 16B  | 2.77 ns       | 2.43 ns           | -12%  |
| 64B  | 5.58 ns       | 4.75 ns           | -15%  |
| 256B | 16.83 ns      | 14.88 ns          | -12%  |

**Why it failed for YAML**:

YAML string scanning only needs to find 2-3 specific characters (`"`, `\`, `'`).

| Approach | Operations per 16-byte chunk |
|----------|------------------------------|
| Direct comparison | 2 comparisons + 1 OR |
| Nibble lookup | 2 table loads + 2 lookups + 1 AND + 1 test |

The table lookup overhead doesn't amortize when searching for few characters.

**When nibble lookup DOES work**:

It's effective for JSON (used in `json/simd/neon.rs`) where you classify 6+ character
types simultaneously: `{`, `}`, `[`, `]`, `:`, `,`, `"`, `\`, plus value chars.
The single classification pass replaces ~13 comparisons with 6 operations.

### NEON/SSE2 Colon-Space Detection for YAML (-15-20% end-to-end penalty)

**Attempted**: Use SIMD to find the `: ` pattern for YAML key-value detection.

```rust
// NEON approach - find colons, then check if next byte is space
let colon_vec = vdupq_n_u8(b':');
let chunk = vld1q_u8(data.as_ptr().add(offset));
let colons = vceqq_u8(chunk, colon_vec);
let colon_mask = neon_movemask(colons);

if colon_mask != 0 {
    let bit_pos = colon_mask.trailing_zeros() as usize;
    if data[offset + bit_pos + 1] == b' ' {
        return Some(offset + bit_pos);
    }
}
```

**Isolation benchmark** (Apple M1 Max) - looked promising:

| Scenario             | SIMD    | Scalar   | Speedup           |
|----------------------|---------|----------|-------------------|
| Short key (11 bytes) | 4.13 ns | 2.31 ns  | **-44% (slower)** |
| Medium key (46 bytes)| 3.35 ns | 10.67 ns | **3.2x**          |
| Long key (92 bytes)  | 4.97 ns | 27.79 ns | **5.6x**          |

**End-to-end result**: 15-20% slower when integrated into parser.

| YAML benchmark   | With SIMD | Without SIMD | Change             |
|------------------|-----------|--------------|-------------------|
| simple_kv/10     | 1.38 µs   | 1.33 µs      | **-4% (slower)**  |
| simple_kv/100    | 6.05 µs   | 5.26 µs      | **-15% (slower)** |
| simple_kv/1000   | 53.7 µs   | 44.7 µs      | **-20% (slower)** |

**Why it failed end-to-end**:

| Factor             | Isolation Benchmark | Real YAML Parsing          |
|--------------------|---------------------|----------------------------|
| Search distance    | 46-92+ bytes        | 5-15 bytes (typical keys)  |
| Breakeven point    | ~16 bytes           | Keys shorter than breakeven|
| Additional overhead| None                | Finding line_end, extra call|

**Key insight**: SIMD setup cost (~4 ns) is only amortized when scanning 40+ bytes.
Typical YAML keys like `name:`, `host:`, `timeout:` are 5-15 chars, where scalar wins.

**Lesson learned**: Isolation benchmarks can mislead - a function can be 5x faster in
isolation but cause regression when integrated due to real-world data characteristics.

---

## Usage in Succinctly

| SIMD Level | Location                | Purpose                  | Speedup |
|------------|-------------------------|--------------------------|---------|
| AVX-512    | `bits/popcount.rs`      | Parallel popcount        | 5.2x    |
| AVX2       | `json/simd/avx2.rs`     | JSON char classification | 1.78x   |
| AVX2+BMI2  | `dsv/simd/bmi2.rs`      | DSV quote masking        | 10x     |
| SSE4.2     | `json/simd/sse42.rs`    | String matching          | 1.38x   |
| NEON       | `json/simd/neon.rs`     | ARM JSON parsing         | 1.11x   |
| NEON       | `dsv/simd/neon.rs`      | ARM DSV parsing          | 1.8x    |
| NEON       | `trees/bp.rs`           | BP L1/L2 index (VMINV)   | 2.8x (L1), 1-3% (L2) |
| NEON       | `bits/popcount.rs`      | 256-byte unrolling       | 1.15x   |
| NEON/AVX2  | `yaml/simd/`            | YAML unquoted structural | 3-8%    |
| NEON       | `yaml/simd/neon.rs`     | Block scalar scanning    | 11-23%  |
| NEON       | `yaml/simd/neon.rs`     | Anchor name scanning     | 3-6%    |
| SVE2-BDEP  | `util/broadword.rs`     | select_in_word           | 5-17x (micro), 2-5% (e2e) |

---

## Key Lessons

1. **Wider isn't always faster**: AVX-512 loses on memory-bound tasks
2. **Profile the bottleneck**: Don't optimize 1% of runtime
3. **Memory bandwidth limits**: 32 bytes/cycle is often the ceiling
4. **Runtime dispatch is cheap**: One branch vs. many iterations
5. **ARM NEON is different**: No movemask; use multiplication trick
6. **SIMD setup cost matters**: For short operations (<16 bytes), scalar wins
7. **Isolation benchmarks can mislead**: A function can be 5x faster in isolation but cause regression when integrated (colon-space detection won for 46+ byte scans, but real YAML keys are 5-15 bytes)
8. **Know your data characteristics**: Benchmark with realistic data sizes, not arbitrary test cases
9. **Specialized instructions win**: SVE2 inline assembly for general ops is 50% slower than NEON, but BDEP (SVE2-BITPERM) provides 5-17x speedup for select_in_word

---

## References

- Intel "Intrinsics Guide": https://www.intel.com/content/www/us/en/docs/intrinsics-guide
- ARM "NEON Intrinsics Reference": https://developer.arm.com/architectures/instruction-sets/intrinsics
- Lemire, D. "Parsing Gigabytes of JSON per Second" (2019)
- Langdale, G. & Lemire, D. "simdjson: Parsing Gigabytes of JSON per Second" (2019)
- Fog, A. "Instruction Tables" - Latency/throughput data
- Mytkowicz, T. et al. "Data-Parallel Finite-State Machines" (2014)
