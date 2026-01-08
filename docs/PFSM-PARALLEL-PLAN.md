# PFSM Parallel Implementation Plan (x86_64 AVX2 + BMI2)

**Date**: 2026-01-08
**Target Platform**: x86_64 with AVX2 and BMI2 (PEXT)
**Goal**: Fully parallel PFSM using fixed-length BP output + PEXT compaction

## Background

The current `pfsm_optimized.rs` achieves ~550 MiB/s on ARM with a simple scalar loop. Previous SIMD attempts failed because:

1. State transitions are sequential (state[i] depends on state[i-1])
2. BP output is variable-length (0-2 bits per byte), preventing parallel writes

This plan addresses both issues using the Mytkowicz PFSM technique combined with fixed-length output + PEXT compaction.

## Key Insight

Instead of computing one state path, compute **all 4 state paths in parallel**. Then select the correct one at the end based on the actual initial state. This breaks the sequential dependency.

For variable-length BP output, use **fixed-length padded output** (always 2 bits per byte) with a **validity mask**, then use **PEXT** to compact to actual bits.

## Algorithm Overview

For a 32-byte chunk (fits in AVX2 registers):

### Step 1: Parallel Table Lookups (AVX2 gather)

```rust
// Load 32 bytes
let bytes: [u8; 32] = chunk;

// For each byte, load PHI_TABLE[byte] (contains phi for all 4 states)
// AVX2 VPGATHERDD can load 8 dwords at once
let phi_all: [[u8; 4]; 32] = gather(PHI_TABLE, bytes);  // 4 gathers for 32 bytes

// Similarly for transitions
let trans_all: [[u8; 4]; 32] = gather(TRANSITION_TABLE, bytes);
```

### Step 2: Parallel Transition Composition (AVX2 shuffle)

Compose transitions using VPSHUFB (pshufb). For 4-state FSM, each transition is a 4-byte permutation.

```rust
// T[i] maps state -> next_state for byte i
// Compose: T_01 = T_1[T_0[s]] for all s
// Using shuffle: T_01 = vpshufb(T_1, T_0)

// Hierarchical composition (log2(32) = 5 levels):
// Level 0: T_01, T_23, T_45, ... (16 compositions)
// Level 1: T_0123, T_4567, ... (8 compositions)
// Level 2: T_01234567, ... (4 compositions)
// Level 3: T_0..15, T_16..31 (2 compositions)
// Level 4: T_0..31 (1 composition)

let T_composed: [u8; 4] = compose_all(trans_all);  // Final composed transition
```

### Step 3: Compute All 4 Phi Paths

For each of the 4 possible initial states, compute the sequence of states and select corresponding phi values:

```rust
// For initial_state in 0..4:
//   states[0] = initial_state
//   states[i] = T[i-1][states[i-1]]
//   phi_selected[i] = phi_all[i][states[i]]

// This can be done with shuffle:
// Given composed transitions at each prefix, use them to compute intermediate states
// Then use vpshufb to select phi values based on state indices
```

### Step 4: Fixed-Length Padded BP Output

For each byte, always output 2 bits (bp_open, bp_close), padded with zeros if not present:

```rust
// phi bits: bit 0 = bp_close, bit 1 = bp_open, bit 2 = ib
// Padded output for byte i: (bp_open << 1) | bp_close  (always 2 bits)
// Mask for byte i: indicates which bits are valid

// For 32 bytes: 64 bits of padded BP, 64 bits of mask
let padded_bp: u64 = pack_padded_bp(phi_selected);
let bp_mask: u64 = pack_bp_mask(phi_selected);
```

### Step 5: Select Correct Path

Based on actual initial state, select one of 4 computed paths:

```rust
let actual_initial_state = carry_in_state;
let (padded_bp, bp_mask, ib_bits) = paths[actual_initial_state];
```

### Step 6: PEXT Compaction

Use BMI2 PEXT to extract valid BP bits:

```rust
// PEXT extracts bits where mask is 1, compacting them
let actual_bp: u64 = _pext_u64(padded_bp, bp_mask);
let bp_bit_count: u32 = bp_mask.count_ones();

// Write to BitWriter
bp_writer.write_bits(actual_bp, bp_bit_count);
ib_writer.write_bits(ib_bits, 32);  // IB is always 1 bit per byte
```

## Implementation Steps

### Phase 1: Scaffolding

1. Create `src/json/pfsm_avx2.rs`
2. Add feature flag `pfsm-avx2` in Cargo.toml
3. Add benchmark in `benches/pfsm_avx2.rs` comparing against production

### Phase 2: Core Implementation

1. **Transition composition** (`compose_transitions_avx2`):
   - Pack 4-byte transitions into __m256i
   - Use vpshufb for composition
   - Hierarchical reduction

2. **Phi path computation** (`compute_phi_paths_avx2`):
   - For each initial state, compute state sequence
   - Use shuffle to select phi values

3. **Padded output generation** (`generate_padded_output`):
   - Extract bp_open, bp_close bits
   - Pack into 64-bit words with masks

4. **PEXT compaction** (`compact_bp_pext`):
   - Apply PEXT with mask
   - Track bit count for writer

### Phase 3: Integration

1. Add to `standard.rs` as optional fast path
2. Runtime CPU detection for AVX2 + BMI2
3. Fallback to `pfsm_optimized` if not available

## Expected Performance

**Theoretical analysis** (32-byte chunk):

| Operation     | Scalar (current)      | Parallel (proposed) |
|---------------|-----------------------|---------------------|
| Table lookups | 32 × 2 = 64 loads     | 4 × 8 = 32 gathers  |
| State chain   | 32 sequential deps    | 5 shuffle levels    |
| BP writes     | 32 conditional writes | 1 PEXT + 1 write    |
| Total         | ~100 cycles?          | ~40 cycles?         |

**Potential speedup**: 2-3x over scalar (if memory-bound, less improvement)

## Risks and Mitigations

1. **Gather latency**: AVX2 gather is slower than expected on some CPUs
   - Mitigation: Benchmark on Zen 4 vs Intel, may need different strategies

2. **PEXT slowness on AMD pre-Zen3**: PEXT is microcoded and very slow
   - Mitigation: Only enable on Zen3+ or Intel

3. **Register pressure**: Tracking 4 paths uses many registers
   - Mitigation: Process 16 bytes instead of 32 if needed

4. **Complexity overhead**: Setup/teardown may dominate for small inputs
   - Mitigation: Only use for chunks > 64 bytes, fallback otherwise

## Benchmark Plan

```rust
// In benches/pfsm_avx2.rs
group.bench_function("pfsm_scalar", |b| { /* pfsm_optimized */ });
group.bench_function("pfsm_avx2_parallel", |b| { /* new implementation */ });
group.bench_function("standard_avx2", |b| { /* existing SIMD path */ });
```

Test with:
- Various sizes: 1KB, 10KB, 100KB, 1MB
- Various patterns: users, nested, arrays, comprehensive
- Compare throughput in MiB/s

## Success Criteria

- [ ] Faster than `pfsm_optimized` (~550 MiB/s baseline)
- [ ] Faster than existing `standard_avx2` (~550 MiB/s on x86)
- [ ] Correct output (matches scalar implementation)
- [ ] No regression on small inputs

## Files to Create/Modify

```
src/json/pfsm_avx2.rs         # New implementation
benches/pfsm_avx2.rs          # Benchmark
tests/pfsm_avx2_correctness.rs # Correctness tests
Cargo.toml                     # Feature flag
src/json/mod.rs                # Module export
src/json/standard.rs           # Integration point
```

## References

- [Data-Parallel Finite-State Machines (Mytkowicz et al., ASPLOS 2014)](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/asplos302-mytkowicz.pdf)
- [Intel Intrinsics Guide - VPGATHERDD](https://www.intel.com/content/www/us/en/docs/intrinsics-guide/)
- [Intel Intrinsics Guide - PEXT](https://www.intel.com/content/www/us/en/docs/intrinsics-guide/)
- [AMD Zen 4 Optimization Guide](https://www.amd.com/en/support/tech-docs)

---

**Status**: PLANNED
**Assigned**: To be implemented on x86_64 machine
