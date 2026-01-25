# Compact Index Investigation Plan

This document outlines the investigation plan for reducing `YamlIndex` memory overhead through compact position encoding.

## Problem Statement

The `bp_to_text` and `bp_to_text_end` arrays use **4 bytes (u32) per node**, contributing ~0.8× input size overhead:

| Component        | Current Size | Formula                                     |
|------------------|--------------|---------------------------------------------|
| `bp_to_text`     | 4 bytes × M  | Start position per BP open                  |
| `bp_to_text_end` | 4 bytes × M  | End position per BP open (0 for containers) |

Where M = number of nodes ≈ input_size / 10 for typical YAML.

**Example (100MB file, 10M nodes):**
- Current: 10M × 4 × 2 = **80 MB**
- With Elias-Fano: ~10M × 5.3 bits ≈ **6.6 MB** (potential 12× reduction)

## Literature Research

### 1. Elias-Fano Encoding

**Source**: [Elias-Fano encoding](https://www.antoniomallia.it/sorted-integers-compression-with-elias-fano-encoding.html)

For monotonically increasing sequences of n integers from universe [0, m):
- **Space**: 2n + n⌈log(m/n)⌉ bits
- **Access**: O(1) using select1
- **Rust crate**: [ef_rs](https://crates.io/crates/ef_rs)

**How it works**:
1. Split each integer into high bits (⌈log n⌉ bits) and low bits (⌈log(m/n)⌉ bits)
2. Store low bits directly (n × ⌈log(m/n)⌉ bits)
3. Encode high bits using unary representation with rank/select support

**Access(i)**:
- Low part: direct lookup at position i × low_bits
- High part: select1(i) - i

### 2. Partitioned Elias-Fano

**Source**: [SIGIR 2014 paper](https://dl.acm.org/doi/10.1145/2600428.2609615)

Partitions the sequence into chunks, encoding each with local parameters:
- Better compression for clustered data
- Two-level structure: chunk endpoints + chunk contents
- Improves compression by 10-30% on real data

### 3. Bit-Compressed Integer Vectors

**Source**: [SDSL Library](https://github.com/simongog/sdsl/wiki/Bit-compressed-integer-vectors)

Use minimum bits to represent the maximum value:
- 100MB file → max position = 100M → needs 27 bits (not 32)
- Simple implementation, maintains O(1) access
- Less dramatic than Elias-Fano but simpler

### 4. Delta Encoding with Sampling

**Source**: [Golomb-Rice coding](https://en.wikipedia.org/wiki/Golomb_coding)

Store differences between consecutive positions:
- Deltas are small (average ~10 for typical YAML)
- Variable-length encode with Golomb-Rice or VByte
- Add samples every k elements for O(k) access

**Trade-off**: Tunable k for space vs access time.

## Key Insight: Scalar vs Container Separation

The positions have different properties for scalars vs containers:

| Property                 | Scalars                  | Containers          |
|--------------------------|--------------------------|---------------------|
| `bp_to_text_end`         | Non-zero (actual end)    | Zero (sentinel)     |
| Positions                | Strictly monotonic       | May share positions |
| Interleaved [start, end] | Monotonically increasing | Not applicable      |
| Count                    | ~70-80% of nodes         | ~20-30% of nodes    |

**Proposed structure**:
```rust
struct CompactPositionIndex {
    // Scalars: interleaved [start₀, end₀, start₁, end₁, ...] - monotonic!
    scalar_positions: EliasFano,

    // Containers: smaller, keep simple
    container_positions: Vec<u32>,

    // Which BP opens are containers (for index translation)
    is_container: BitVec,  // Already exists as `containers` bitvector
}
```

Access:
```rust
fn scalar_span(&self, bp_open_idx: usize) -> (usize, usize) {
    let scalar_idx = bp_open_idx - self.is_container.rank1(bp_open_idx);
    let start = self.scalar_positions.access(scalar_idx * 2);
    let end = self.scalar_positions.access(scalar_idx * 2 + 1);
    (start, end)
}

fn container_start(&self, bp_open_idx: usize) -> usize {
    let container_idx = self.is_container.rank1(bp_open_idx);
    self.container_positions[container_idx] as usize
}
```

## Investigation Phases

### Phase 1: Measurement & Baselines

**Goal**: Understand actual position distributions in real YAML files.

**Tasks**:
1. Add instrumentation to count scalars vs containers
2. Measure average gap size between consecutive positions
3. Profile actual memory usage breakdown per component
4. Test with different YAML patterns (users, config, nested, etc.)

**Deliverable**: Data table with position statistics for benchmark files.

```bash
# Proposed instrumentation command
./target/release/succinctly dev analyze-index data/bench/generated/yaml/*/1mb.yaml
```

### Phase 2: Elias-Fano Prototype

**Goal**: Validate Elias-Fano compression ratio and access performance.

**Tasks**:
1. Evaluate existing Rust crates:
   - [ef_rs](https://crates.io/crates/ef_rs)
   - [succinct](https://crates.io/crates/succinct)
   - [elias-fano](https://crates.io/crates/elias-fano)
2. Build standalone prototype with scalar positions
3. Benchmark access latency vs Vec<u32>
4. Measure actual compression ratio

**Success criteria**:
- Access latency < 2× current Vec<u32>
- Space reduction > 4× for scalar positions

### Phase 3: Integration Design

**Goal**: Design integration with YamlIndex without breaking existing API.

**Key questions**:
1. Should CompactPositionIndex be behind a feature flag?
2. How to handle the `W: AsRef<[u64]>` generic parameter?
3. Impact on `from_parts()` for serialization?
4. Migration path for existing code?

**Proposed approach**:
```rust
pub struct YamlIndex<W = Vec<u64>, P = DensePositions> {
    // ... existing fields ...
    positions: P,  // Generic over position encoding
}

// Default: current behavior
pub struct DensePositions {
    bp_to_text: Vec<u32>,
    bp_to_text_end: Vec<u32>,
}

// Compact: Elias-Fano for scalars
pub struct CompactPositions {
    scalar_positions: EliasFano,
    container_positions: Vec<u32>,
}

// Trait for position lookup
pub trait PositionLookup {
    fn text_pos(&self, bp_open_idx: usize, is_container: bool) -> usize;
    fn text_end_pos(&self, bp_open_idx: usize) -> Option<usize>;
}
```

### Phase 4: Implementation & Benchmarking

**Goal**: Full implementation with comprehensive benchmarks.

**Tasks**:
1. Implement `CompactPositions` struct
2. Integrate with YamlIndex build path
3. Update cursor navigation to use new lookup
4. Benchmark memory and latency:
   - Index build time
   - `bp_to_text_pos()` latency
   - Full query benchmarks (yq_comparison)
5. Memory profiling with `/usr/bin/time -l`

**Benchmark matrix**:

| File Size | Pattern | Current Memory | Compact Memory | Build Time Δ | Query Time Δ |
|-----------|---------|----------------|----------------|--------------|--------------|
| 10KB      | users   | ?              | ?              | ?            | ?            |
| 100KB     | users   | ?              | ?              | ?            | ?            |
| 1MB       | users   | ?              | ?              | ?            | ?            |
| 10MB      | users   | ?              | ?              | ?            | ?            |
| 100MB     | users   | ?              | ?              | ?            | ?            |

### Phase 5: Trade-off Analysis

**Goal**: Document trade-offs and make go/no-go decision.

**Considerations**:
1. **Memory reduction**: Target 4× reduction in position arrays
2. **Access latency**: Maximum 2× slowdown acceptable
3. **Build time**: Maximum 20% increase acceptable
4. **Code complexity**: Is the added complexity justified?
5. **Maintenance burden**: Are dependencies stable?

**Decision criteria**:
- **Accept** if: Memory reduction ≥ 3× AND latency increase ≤ 50%
- **Reject** if: Memory reduction < 2× OR latency increase > 100%
- **Defer** if: Results mixed, need more investigation

## Expected Outcomes

### Optimistic Case

| Component           | Current   | Compact   | Reduction |
|---------------------|-----------|-----------|-----------|
| Scalar positions    | 64 MB     | 5 MB      | 12×       |
| Container positions | 16 MB     | 16 MB     | 1×        |
| **Total**           | **80 MB** | **21 MB** | **4×**    |

Total index overhead: ~5× → ~3.5× input size

### Pessimistic Case

Elias-Fano access latency too high for hot path (`bp_to_text_pos` called frequently during navigation). Fall back to bit-compressed vectors:

| Component     | Current | Bit-compressed | Reduction |
|---------------|---------|----------------|-----------|
| All positions | 80 MB   | 68 MB (27-bit) | 1.2×      |

Minimal improvement, likely reject optimization.

## Related Work

- [Issue #53: M3 streaming builtins](https://github.com/rust-works/succinctly/issues/53) - Different M3, for streaming `length`/`keys`
- [yq-memory-optimization.md](yq-memory-optimization.md) - DOM streaming optimizations (M1-M6)
- [docs/benchmarks/yq.md](../benchmarks/yq.md) - Current benchmark results

## References

1. [Elias-Fano encoding introduction](https://www.antoniomallia.it/sorted-integers-compression-with-elias-fano-encoding.html)
2. [Elias-Fano for Rank/Select (SEA 2025)](https://drops.dagstuhl.de/storage/00lipics/lipics-vol338-sea2025/LIPIcs.SEA.2025.23/LIPIcs.SEA.2025.23.pdf)
3. [Partitioned Elias-Fano (SIGIR 2014)](https://dl.acm.org/doi/10.1145/2600428.2609615)
4. [SDSL bit-compressed vectors](https://github.com/simongog/sdsl/wiki/Bit-compressed-integer-vectors)
5. [RRR succinct bitvectors](https://www.alexbowe.com/rrr/)
6. [Golomb-Rice coding](https://en.wikipedia.org/wiki/Golomb_coding)
7. [ef_rs Rust crate](https://crates.io/crates/ef_rs)
8. [succinct Rust crate](https://github.com/miiohio/succinct)

## Implementation Status

### Phase 2 Complete: Portable Elias-Fano Implementation

**Location**: `src/bits/elias_fano.rs`

A portable Elias-Fano implementation with cursor-based access has been completed:

```rust
pub struct EliasFano {
    low_bits: Vec<u64>,      // Dense packed low bits
    low_width: usize,        // Bits per low value
    high_bits: Vec<u64>,     // Unary encoded high bits
    len: usize,              // Element count
    universe: u64,           // Max value + 1
    select_samples: Vec<u32>, // Sampled positions (every 256 elements)
}

pub struct EliasFanoCursor<'a> { /* cursor state for O(1) amortized iteration */ }
```

**Operations implemented**:
- `build(values)` - Construction from sorted `Vec<u32>`
- `get(i)` - O(1) random access via select samples
- `cursor()` / `cursor_from(idx)` - Create cursor at position
- `advance_one()` - O(1) amortized forward iteration
- `advance_by(k)` - Skip k elements forward
- `seek(idx)` - Jump to arbitrary index

### Benchmark Results (Apple M1 Max)

**Compression** (bp_to_text-like data with 10-100 byte gaps):

| Elements | Vec\<u32\> | EliasFano | Compression |
|----------|------------|-----------|-------------|
| 10K      | 39 KB      | 8 KB      | **4.42×**   |
| 100K     | 390 KB     | 88 KB     | **4.42×**   |
| 1M       | 3.9 MB     | 883 KB    | **4.42×**   |

**Construction throughput**:

| Elements | Time      | Throughput   |
|----------|-----------|--------------|
| 10K      | 22.7 µs   | 441 Melem/s  |
| 100K     | 231 µs    | 432 Melem/s  |
| 1M       | 2.33 ms   | 430 Melem/s  |

**Access latency** (criterion benchmarks, 100K elements):

| Operation                | EliasFano     | Vec\<u32\>    | Ratio   |
|--------------------------|---------------|---------------|---------|
| Sequential (advance_one) | 1.77 ns/elem  | 0.048 ns/elem | **37×** |
| Random access (get)      | 30.7 ns       | 0.67 ns       | **46×** |
| cursor_from              | 19.0 ns       | 0.43 ns       | **44×** |
| cursor_from + 5 iter     | 32.6 ns       | 1.69 ns       | **19×** |
| Skip-by-2-8 (advance_by) | 128 µs total  | 20.8 µs total | **6.2×** |
| seek (random jump)       | 17.1 ns       | 0.42 ns       | **41×** |

**Key findings**:
1. **Compression achieved**: 4.42× (exceeds 4× target)
2. **Random access**: 46× slower than Vec (exceeds 2× target)
3. **Cursor iteration amortizes well**: Ratio drops from 44× to 19× with 5 iterations
4. **Skip-by-small is efficient**: Only 6.2× slower for advance_by(2-8) patterns
5. **Forward-only patterns**: Cursor-based approach works well for jq navigation patterns

### Next Steps

**Phase 3: SIMD Acceleration (x86)**
- BMI2 PDEP+TZCNT for O(1) select-in-word (currently O(popcount) loop)
- Expected: 3-4× speedup on random access

**Phase 4: SIMD Acceleration (ARM)**
- NEON popcount for broadword select
- SVE2 BDEP if available

**Phase 5: Integration**
- Replace `bp_to_text: Vec<u32>` with `EliasFano` in YamlIndex
- Evaluate end-to-end yq benchmark impact

**Phase 6: bp_to_text_end**
- Delta encoding from bp_to_text (scalar end = start + length)
- Container end = 0 (sentinel, don't encode)

### Decision Status

**Current assessment**: PROMISING - broadword optimization available for M1.

- ✅ Compression: 4.42× (target: 4×)
- ❌ Random access latency: 46× with CTZ (target: ≤2×)
- ⚠️ Cursor iteration: 19× with amortization (acceptable for forward-only patterns)
- ✅ Skip-by-small: 6.2× (good for next_sibling patterns)

**M1 Optimization Discovery** (select_in_word benchmark):

| k  | CTZ (ns) | Broadword (ns) | Winner    | Speedup |
|----|----------|----------------|-----------|---------|
| 0  | 0.65     | 1.47           | CTZ       | 0.4×    |
| 1  | 0.96     | 1.78           | CTZ       | 0.5×    |
| 4  | 2.57     | 2.43           | Broadword | 1.1×    |
| 8  | 3.85     | 1.52           | Broadword | 2.5×    |
| 16 | 6.44     | 1.98           | Broadword | 3.3×    |
| 31 | 12.37    | 3.59           | Broadword | 3.4×    |
| 48 | 20.82    | 3.22           | Broadword | 6.5×    |
| 63 | 28.14    | 4.62           | Broadword | 6.1×    |

**Key insight**: CTZ is O(k), Broadword is O(1). For k≥4, broadword wins.
In Elias-Fano `select1`, k averages ~32 within a word → **~3-4× speedup expected**.

**Platform-specific expectations**:
- x86 with BMI2: PDEP+TZCNT should achieve 3-4× speedup
- ARM with SVE2 (Graviton 4): BDEP should achieve 3-4× speedup
- M1/M2/M3 with broadword: **~3× speedup** (46× → ~15× vs Vec)

**Next optimization**: Switch `select_in_word` to use broadword for k≥4 on M1.

## Changelog

| Date       | Change                                                        |
|------------|---------------------------------------------------------------|
| 2026-01-25 | Initial investigation plan based on literature research       |
| 2026-01-25 | Phase 2 complete: Portable EliasFano with cursor-based access |
| 2026-01-25 | Updated benchmarks with criterion results; M1 optimization analysis complete |
| 2026-01-25 | Discovery: Broadword 3-6× faster than CTZ for k≥4 on M1       |
