# Instructions Between AVX2 and AVX-512

## Your CPU Support (AMD Ryzen 9 7950X)

Your CPU has several important instruction sets between AVX2 (2013) and AVX-512 (2019+):

### Available Instruction Sets

| Instruction Set                 | Year | Your CPU     | Availability | Potential Use       |
|---------------------------------|------|--------------|--------------|---------------------|
| **BMI1** (Bit Manipulation 1)   | 2013 | ✓ bmi1       | ~95%         | TZCNT, BLSI, BLSMSK |
| **BMI2** (Bit Manipulation 2)   | 2013 | ✓ bmi2       | ~95%         | PDEP, PEXT, BZHI    |
| **FMA** (Fused Multiply-Add)    | 2013 | ✓ fma        | ~95%         | Math operations     |
| **F16C** (Half-precision float) | 2012 | ✓ f16c       | ~95%         | FP16 conversion     |
| **LZCNT** (Leading Zero Count)  | 2013 | ✓ (via bmi1) | ~95%         | Bit operations      |
| **POPCNT** (Population Count)   | 2008 | ✓ popcnt     | ~100%        | Already using       |
| **AES-NI** (AES instructions)   | 2010 | ✓ aes        | ~98%         | Encryption          |
| **PCLMULQDQ**                   | 2010 | ✓ pclmulqdq  | ~98%         | CRC, hashing        |
| **SHA** (SHA extensions)        | 2017 | ✓ sha_ni     | ~70%         | SHA hashing         |
| **VAES** (Vector AES)           | 2018 | ✓ vaes       | ~20%         | Wide AES            |
| **VPCLMULQDQ** (Vector PCLMUL)  | 2018 | ✓ vpclmulqdq | ~20%         | Wide CRC            |
| **GFNI** (Galois Field)         | 2018 | ✓ gfni       | ~20%         | Crypto, encoding    |

## High-Value Opportunities for This Codebase

### 1. BMI2 - PDEP/PEXT (Bit Deposit/Extract) ⭐⭐⭐

**Status**: Partially used in [src/json/simd/bmi2.rs](../src/json/simd/bmi2.rs)

**Current Usage**: BMI2 utilities exist but not actively used in hot paths

**Potential Applications**:

#### A. JSON Mask Processing (High Impact)

After SIMD character classification, we have bitmasks like:
```rust
let quotes = 0b0000_0010_0000_0010;  // Positions 1 and 9 have quotes
```

**Current approach**: Loop through bits
```rust
for i in 0..32 {
    if (mask & (1 << i)) != 0 {
        // Process structural character at position i
    }
}
```

**BMI2 approach**: Extract set bits directly
```rust
// PEXT extracts set bits into contiguous positions
// PDEP deposits bits to sparse positions
let positions = _tzcnt_u32(mask);  // Find next set bit (BMI1)
mask = _blsr_u32(mask);             // Clear lowest set bit (BMI1)
```

**Better approach**: Vectorized mask processing
```rust
// Use PEXT to extract structural character positions
// Build lookup tables for fast state transitions
// Reduce branch mispredictions
```

**Expected impact**: 10-15% JSON parsing speedup (Intel only - see caveat)

#### B. Select Operations (Medium Impact)

Select operations need to find k-th set bit:
```rust
pub fn select1(&self, k: usize) -> Option<usize> {
    // Current: Binary search + broadword tricks
}
```

**BMI2 enhancement**:
```rust
// Use PDEP to create mask with k-th bit set
// Combine with existing broadword algorithms
// Faster for small k values
```

**Expected impact**: 5-10% select speedup for small k

#### C. Balanced Parentheses Matching (Low Impact)

BP operations use excess masks - BMI2 could optimize mask manipulation.

**Expected impact**: <5% (not hot path)

### 2. LZCNT/TZCNT (Leading/Trailing Zero Count) ⭐⭐

**Status**: Available via BMI1

**Current Usage**: Using `count_ones()` and broadword tricks

**Potential Applications**:

#### A. Select Operations
```rust
// Current broadword select_in_word
pub fn select_in_word(word: u64, k: u32) -> u32 {
    // Complex broadword algorithm
}

// With TZCNT (simpler for k=0 case)
if k == 0 {
    return _tzcnt_u64(word);  // First set bit
}
```

**Expected impact**: 5% select speedup for k=0 queries

#### B. JSON Mask Processing
```rust
// Find next structural character
while mask != 0 {
    let pos = _tzcnt_u32(mask);      // Find position
    mask = _blsr_u32(mask);           // Clear bit (BMI1)
    process_structural_char(pos);
}
```

**Expected impact**: 5-10% JSON parsing speedup

### 3. PCLMULQDQ (Carry-less Multiply) ⭐

**Status**: Not currently used

**Potential Applications**:

#### A. CRC32 for Data Validation
```rust
// Fast CRC32 for integrity checking
// Useful for:
// - Validating loaded bitvectors
// - Checksumming serialized data
// - Hash functions for deduplication
```

**Expected impact**: 5-10x faster CRC32 (if needed)

**Relevance**: Medium - only if adding data validation features

### 4. FMA (Fused Multiply-Add) ⭐

**Status**: Not applicable to current codebase

**Why not useful here**:
- No floating-point math in hot paths
- Bitvector operations are integer-only
- JSON parsing is text processing

**When it would help**: If adding statistical analysis or numerical indexing

### 5. SHA Extensions ⭐

**Status**: Not currently used

**Potential Applications**:

#### A. Content Hashing
```rust
// Ultra-fast SHA-256 for:
// - Content-addressable storage
// - Deduplication
// - Cache keys
```

**Expected impact**: 5-10x faster SHA-256 (if needed)

**Relevance**: Low - not core use case

## Critical Caveat: AMD vs Intel BMI2 Performance

### ⚠️ AMD Zen 1/2 BMI2 Problem

**Intel** (Haswell+):
- PDEP/PEXT: 3 cycles (hardware implementation)
- Fast, worth using

**AMD Zen 1/2**:
- PDEP/PEXT: 18 cycles (microcode)
- Slower than scalar alternatives
- **Don't use on Zen 1/2**

**AMD Zen 3+** (including your Zen 4):
- PDEP/PEXT: 3 cycles (hardware implementation)
- Fast, like Intel
- **Safe to use**

### Detection Strategy

```rust
#[cfg(target_arch = "x86_64")]
fn should_use_bmi2() -> bool {
    if !is_x86_feature_detected!("bmi2") {
        return false;
    }

    // Check if AMD Zen 1/2 (slow BMI2)
    #[cfg(feature = "std")]
    {
        if is_amd_zen1_or_zen2() {
            return false;  // Microcode implementation
        }
    }

    true  // Intel or AMD Zen 3+
}

fn is_amd_zen1_or_zen2() -> bool {
    // CPUID checks for AMD family/model
    // Zen 1: Family 17h, Model 00h-0Fh
    // Zen 2: Family 17h, Model 30h-3Fh, 60h-6Fh, 70h-7Fh
    // Zen 3+: Family 19h+ (fast BMI2)
    false  // Implement CPUID checking
}
```

## Recommended Implementation Priority

### High Priority: BMI1 (TZCNT, BLSR)

**Why**:
- ✓ Fast on all CPUs (no microcode issues)
- ✓ Simple to use
- ✓ Clear benefit for mask processing
- ✓ Already available (bmi1 flag present)

**Where**: JSON mask iteration, select operations

**Expected impact**: 5-10% JSON parsing, 5% select operations

**Implementation effort**: Low (1-2 hours)

### Medium Priority: BMI2 with CPU Detection

**Why**:
- ✓ Major benefits on Intel and Zen 3+ (10-15% JSON speedup)
- ⚠️ Must detect and avoid on Zen 1/2
- ✓ Your CPU (Zen 4) has fast BMI2

**Where**: JSON mask processing, select operations

**Expected impact**: 10-15% JSON parsing on compatible CPUs

**Implementation effort**: Medium (4-6 hours including CPU detection)

### Low Priority: PCLMULQDQ, SHA

**Why**:
- Not core to current use cases
- Would need new features (validation, hashing)
- Significant implementation effort

**When**: If adding data validation or content-addressable features

## Comparison with AVX-512 Learnings

### AVX-512: Wider SIMD Didn't Help

JSON parsing is **memory-bound** with **sequential state machine**:
- Doubling vector width (32 → 64 bytes) = 3% slower
- Memory access and state transitions dominate
- Wide SIMD adds overhead without benefit

### BMI1/BMI2: Different Type of Optimization

These are **scalar bit manipulation** instructions:
- Not about parallelism, about **better algorithms**
- PDEP/PEXT enable new approaches (parallel bit deposit/extract)
- TZCNT/BLSR make mask iteration faster
- Work on **the actual bottleneck** (mask processing, branches)

**Key difference**:
- AVX-512: Tried to parallelize already-parallel work (classification)
- BMI: Optimizes sequential work (mask iteration, branching)

### Expected Results

Based on AVX-512 learnings:

**BMI1 (TZCNT/BLSR)**: ✓ Likely 5-10% speedup
- Reduces branch mispredictions
- Faster mask iteration
- Simpler than current approaches

**BMI2 (PDEP/PEXT)**: ✓ Likely 10-15% speedup (on Zen 3+/Intel)
- Enables better algorithms
- Reduces sequential overhead
- But only on CPUs with fast implementation

**Why more optimistic than AVX-512**:
- Targeting actual bottleneck (mask processing, branches)
- Not adding width overhead
- Scalar optimizations for scalar work

## Proof of Concept: BMI1 JSON Mask Processing

Here's what the optimization would look like:

```rust
// Current approach (benches/json_avx512_comparison.rs shows)
fn process_chunk_avx2(class: CharClass, state: State, ...) {
    for i in 0..32 {
        let bit = 1u32 << i;
        let is_quote = (class.quotes & bit) != 0;
        let is_open = (class.opens & bit) != 0;
        // ... 5 more mask checks per byte
        // ... state machine logic
    }
}

// BMI1-optimized approach
#[target_feature(enable = "bmi1")]
unsafe fn process_chunk_bmi1(class: CharClass, state: State, ...) {
    let mut quotes = class.quotes;
    let mut opens = class.opens;
    let mut closes = class.closes;
    // ... other masks

    // Process each structural character directly
    while quotes != 0 {
        let pos = _tzcnt_u32(quotes);           // Find next quote
        quotes = _blsr_u32(quotes);              // Clear this bit
        state = handle_quote(state, pos);
    }

    while opens != 0 {
        let pos = _tzcnt_u32(opens);
        opens = _blsr_u32(opens);
        state = handle_open(state, pos);
    }

    // ... process other structural characters
}
```

**Why faster**:
1. No loop overhead (direct bit iteration)
2. No redundant mask checks (only process set bits)
3. Better branch prediction (fewer branches)
4. Simpler control flow

**Expected**: 5-10% speedup

## Recommendation

### Implement Now: BMI1 Optimization

**Target**: JSON mask processing in [src/json/simd/](../src/json/simd/)

**Steps**:
1. Add BMI1-optimized `process_chunk` variant
2. Runtime dispatch based on `is_x86_feature_detected!("bmi1")`
3. Benchmark against current AVX2 implementation
4. Update documentation if successful

**Risk**: Low (BMI1 is fast on all CPUs)
**Effort**: 2-3 hours
**Expected gain**: 5-10% JSON parsing speedup

### Consider Later: BMI2 with CPU Detection

**Benefit**: Potentially 10-15% more speedup
**Risk**: Medium (must handle Zen 1/2 correctly)
**Effort**: 4-6 hours (including CPU detection logic)

**Decision**: Implement BMI1 first, measure results, then decide if BMI2 is worth the added complexity.

## References

- [Intel Intrinsics Guide - BMI1](https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#techs=BMI1)
- [Intel Intrinsics Guide - BMI2](https://www.intel.com/content/www/us/en/docs/intrinsics-guide/index.html#techs=BMI2)
- [AMD Zen 3 BMI2 Improvements](https://www.anandtech.com/show/16214/amd-zen-3-ryzen-deep-dive-review-5950x-5900x-5800x-and-5700x-tested/7)
- [Travis Downs: BMI2 on AMD](https://travisdowns.github.io/blog/2020/05/18/icelake-bmi2.html)
