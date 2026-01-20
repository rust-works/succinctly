# Parsing Documentation

This directory documents how the succinctly library parses structured data formats using semi-indexing techniques.

## Overview

Traditional parsers materialize data into memory structures (DOM trees, row arrays). Succinctly instead creates **semi-indices**: compact bit vectors that mark structural boundaries, enabling O(1) navigation without full materialization.

| Format | Document | Key Technique |
|--------|----------|---------------|
| JSON | [json.md](json.md) | Interest bits + balanced parentheses |
| CSV/TSV | [dsv.md](dsv.md) | Marker bits + quote state tracking |
| YAML | [yaml.md](yaml.md) | Feasibility analysis (not implemented) |

---

## Semi-Indexing Approach

### Traditional Parsing

```
Input → Full Parse → DOM/Rows → Query
         ↓
   10-50x memory overhead
```

### Semi-Indexing

```
Input → Bit Index → Lazy Navigation → Query
           ↓
      ~3-4% memory overhead
```

**Benefits**:
- Memory efficient: 3-4% overhead vs 10-50x
- Fast construction: SIMD-accelerated
- Lazy evaluation: Decode values only when accessed
- Random access: O(1) rank, O(log n) select

---

## Common Techniques

Both parsers share optimization techniques:

| Technique | JSON | DSV | Document |
|-----------|------|-----|----------|
| SIMD character detection | AVX2, NEON | AVX2, NEON | [simd.md](../optimizations/simd.md) |
| Lookup tables | PFSM tables | - | [lookup-tables.md](../optimizations/lookup-tables.md) |
| State machine | 4-state FSM | Quote toggle | [state-machines.md](../optimizations/state-machines.md) |
| Rank/select indices | BP navigation | Field lookup | [hierarchical-structures.md](../optimizations/hierarchical-structures.md) |
| Parallel prefix | - | Quote masking | [parallel-prefix.md](../optimizations/parallel-prefix.md) |
| PDEP/PEXT | - | toggle64_bmi2 | [bit-manipulation.md](../optimizations/bit-manipulation.md) |

---

## Performance Summary

### Parsing Throughput

| Format | Platform | CPU | Throughput |
|--------|----------|-----|------------|
| JSON | x86_64 PFSM | AMD Ryzen 9 7950X (Zen 4) | ~950 MiB/s |
| JSON | ARM NEON | Apple M1 Max | ~570 MiB/s |
| DSV | x86_64 BMI2 | AMD Ryzen 9 7950X (Zen 4) | 1.3-3.7 GB/s |
| DSV | ARM NEON | Apple M1 Max | ~800 MiB/s |

### Memory Overhead

| Approach | Overhead |
|----------|----------|
| Semi-index | ~3-4% of input |
| DOM parser (JSON) | 10-50x input |
| Materialized (CSV) | 6-40x input |

---

## Quick Links

- [JSON Parsing](json.md) - Semi-index structure, PFSM, SIMD classification
- [DSV Parsing](dsv.md) - Quote handling, PDEP carry propagation, lightweight index
- [YAML Analysis](yaml.md) - Feasibility study, ambiguity analysis, oracle model

### Optimisation Techniques

- [simd.md](../optimizations/simd.md) - AVX2, NEON character detection
- [lookup-tables.md](../optimizations/lookup-tables.md) - PFSM state tables
- [state-machines.md](../optimizations/state-machines.md) - Parser state machines
- [parallel-prefix.md](../optimizations/parallel-prefix.md) - Quote state with prefix XOR
- [bit-manipulation.md](../optimizations/bit-manipulation.md) - PDEP for 10x faster quotes
- [hierarchical-structures.md](../optimizations/hierarchical-structures.md) - Rank/select indices
- [cache-memory.md](../optimizations/cache-memory.md) - Why lightweight index wins

---

## References

- Langdale, G. & Lemire, D. "Parsing Gigabytes of JSON per Second" (2019)
- simdjson: https://github.com/simdjson/simdjson
- hw-json/hw-dsv (Haskell): https://github.com/haskell-works
