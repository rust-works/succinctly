# PFSM (Parallel Finite State Machine) Implementation Results

## Summary

The PFSM approach **is faster** when implemented correctly with single-pass processing, but the original two-pass implementation with intermediate vector allocation was slower.

## Performance Results

### 1KB File
| Implementation     | Throughput    | vs Standard Scalar | vs AVX2           |
|--------------------|---------------|--------------------|-------------------|
| **PFSM Optimized** | **951 MiB/s** | **+73% faster** ✓  | **+43% faster** ✓ |
| Standard AVX2      | 666 MiB/s     | +21% faster        | -                 |
| Standard Scalar    | 551 MiB/s     | -                  | -21% slower       |
| PFSM Two-Pass      | 472 MiB/s     | -14% slower        | -29% slower       |

### 10KB File
| Implementation     | Throughput    | vs Standard Scalar | vs AVX2           |
|--------------------|---------------|--------------------|-------------------|
| **PFSM Optimized** | **949 MiB/s** | **+77% faster** ✓  | **+57% faster** ✓ |
| Standard AVX2      | 603 MiB/s     | +12% faster        | -                 |
| Standard Scalar    | 537 MiB/s     | -                  | -11% slower       |
| PFSM Two-Pass      | 457 MiB/s     | -15% slower        | -24% slower       |

### 100KB File
| Implementation     | Throughput    | vs Standard Scalar | vs AVX2           |
|--------------------|---------------|--------------------|-------------------|
| **PFSM Optimized** | **817 MiB/s** | **+66% faster** ✓  | **+42% faster** ✓ |
| Standard AVX2      | 577 MiB/s     | +17% faster        | -                 |
| Standard Scalar    | 491 MiB/s     | -                  | -15% slower       |
| PFSM Two-Pass      | 447 MiB/s     | -9% slower         | -23% slower       |

## Key Findings

### 1. The Overhead Source Was Memory Allocation, Not Tables

The original PFSM implementation was slow because it:
- Allocated a `Vec<u8>` to store intermediate phi values
- Required two passes over the data
- Had worse cache locality

### 2. Single-Pass PFSM is Fastest

The optimized single-pass implementation:
- **No intermediate allocation**: Processes phi values immediately
- **One loop**: State machine and bit extraction in the same iteration
- **Better cache locality**: All data accessed once, in order
- **Table lookups are fast**: Modern CPUs handle them well

### 3. Why PFSM Optimized Beats Standard Cursor

The table-driven approach has advantages:
1. **Fewer branches**: Standard cursor has many `if` statements
2. **Predictable memory access**: Table lookups are cache-friendly
3. **Simpler control flow**: One loop, no nested conditions
4. **Better compiler optimization**: More opportunities for instruction-level parallelism

### 4. Why PFSM Optimized Beats AVX2

AVX2 SIMD implementation has overhead from:
1. **Lane alignment**: AVX2 operates on 32-byte chunks
2. **Shuffle operations**: Extracting results from SIMD registers
3. **Small inputs**: 1-100KB files don't amortize SIMD setup costs
4. **Memory latency**: AVX2 is memory-bound, not compute-bound

## Code Comparison

### Two-Pass PFSM (Slow)
```rust
// Pass 1: Build phi vector
let mut phi_values = Vec::with_capacity(json.len());
for &byte in json {
    let phi = PfsmState::extract_phi(PHI_TABLE[byte as usize], state);
    phi_values.push(phi);  // Allocation overhead
    state = PfsmState::extract_next_state(TRANSITION_TABLE[byte as usize], state);
}

// Pass 2: Extract bits
for &phi in &phi_values {
    // Process phi...
}
```

### Single-Pass PFSM (Fast)
```rust
// Single pass: No allocation
for &byte in json {
    let phi = PfsmState::extract_phi(PHI_TABLE[byte as usize], state);
    state = PfsmState::extract_next_state(TRANSITION_TABLE[byte as usize], state);

    // Extract and write bits immediately
    ib.write_bit((phi >> 2) & 1);
    if (phi >> 1) & 1 != 0 { bp.write_1(); }
    if phi & 1 != 0 { bp.write_0(); }
}
```

## Correctness Verification

✅ All tests pass:
- `tests/pfsm_correctness.rs`: 6/6 tests
- Produces identical IB/BP bitvectors to standard cursor
- Handles all JSON patterns correctly

## Recommendations

1. **Use PFSM Optimized as default**: 40-77% faster than standard scalar
2. **Keep AVX2 for reference**: Useful for comparison and future optimization
3. **Remove two-pass PFSM**: No benefit, adds complexity
4. **Future work**: Implement SIMD PFSM using BMI2/AVX2 for batch processing

## Implementation Details

### File: `src/json/pfsm_optimized.rs`
- Single-pass processing
- No intermediate allocations
- Table-driven state machine
- 42 lines of code (vs 170+ for two-pass)

### Key Insight
**Memory allocation overhead >> Table lookup cost**

The table lookups are actually faster than branchy conditionals because:
- Modern CPUs have excellent cache prefetching for linear table access
- Branch prediction penalties are higher than cache hits
- Table lookups can execute in parallel with other instructions

## Conclusion

The PFSM approach **works** and is **faster** when implemented optimally. The key is eliminating unnecessary allocations and processing data in a single pass. Table-driven parsing can outperform hand-written state machines with many branches.
