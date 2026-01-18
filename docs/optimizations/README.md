# Optimisation Techniques

This directory documents optimization techniques used in the succinctly library, organized by category. Each document covers the theory, implementation patterns, prior art, and practical lessons learned.

## Quick Reference

| Category | Document | Key Techniques |
|----------|----------|----------------|
| **Bit-level** | [bit-manipulation.md](bit-manipulation.md) | Popcount, CTZ/CLZ, broadword select, PDEP/PEXT |
| **Lookup tables** | [lookup-tables.md](lookup-tables.md) | Byte tables, nibble tables, state machine tables |
| **SIMD** | [simd.md](simd.md) | AVX2, AVX-512, SSE4.2, NEON |
| **Memory** | [cache-memory.md](cache-memory.md) | Alignment, layout, prefetching, bandwidth |
| **Data structures** | [hierarchical-structures.md](hierarchical-structures.md) | Rank directory, select index, RangeMin |
| **Control flow** | [branchless.md](branchless.md) | CMOV, arithmetic selection, SIMD masking |
| **Access patterns** | [access-patterns.md](access-patterns.md) | Sequential, strided, random, search algorithms |
| **Allocation** | [zero-copy.md](zero-copy.md) | Type punning, memory mapping, streaming |
| **Parallelism** | [parallel-prefix.md](parallel-prefix.md) | Prefix XOR, cumulative index, carry propagation |
| **Parsing** | [state-machines.md](state-machines.md) | PFSM, fast-path bypass, two-stage pipeline |

---

## Performance Impact Summary

### Highest Impact Techniques

| Technique | Speedup | Category | Document |
|-----------|---------|----------|----------|
| Cumulative Index | **627x** | Hierarchical | [hierarchical-structures.md](hierarchical-structures.md) |
| RangeMin Index | **40x** | Hierarchical | [hierarchical-structures.md](hierarchical-structures.md) |
| Byte Lookup Tables | **11x** | Lookup | [lookup-tables.md](lookup-tables.md) |
| BMI2 PDEP Toggle | **10x** | Bit manipulation | [bit-manipulation.md](bit-manipulation.md) |
| Lightweight DSV Index | **5-9x** | Cache/Memory | [cache-memory.md](cache-memory.md) |
| AVX-512 VPOPCNTDQ | **5.2x** | SIMD | [simd.md](simd.md) |
| Exponential Search | **3.1x** | Access patterns | [access-patterns.md](access-patterns.md) |
| AVX2 JSON Parser | **1.78x** | SIMD | [simd.md](simd.md) |
| PFSM Tables | **1.77x** | State machines | [state-machines.md](state-machines.md) |
| NEON DSV Index | **1.8x** | SIMD | [simd.md](simd.md) |
| SIMD Unquoted Structural Skip | **3-8%** | SIMD | [simd.md](simd.md) |
| Lazy line + direct indexing | **2-6%** | State machines | [state-machines.md](state-machines.md) |

### Notable Failures (Instructive)

| Technique | Penalty | Reason | Document |
|-----------|---------|--------|----------|
| BMI2 PDEP BitWriter | **-71%** | Wrong use case (consecutive bits) | [bit-manipulation.md](bit-manipulation.md) |
| NEON PFSM Shuffle | **-47%** | Serial dependency overhead | [state-machines.md](state-machines.md) |
| BMI1 Mask Iteration | **-26%** | Optimised <1% of runtime | [simd.md](simd.md) |
| NEON Batched Popcount | **-25%** | Prefix sum inherently sequential | [parallel-prefix.md](parallel-prefix.md) |
| AVX-512 JSON Parser | **-10%** | Memory-bound, not compute-bound | [simd.md](simd.md) |
| AVX-512 YAML Parser | **-7%** | Memory bandwidth bottleneck + benchmark flaw | [simd.md](simd.md) |
| SIMD Lookahead Quote Skip | **-2 to -6%** | Short strings, SIMD overhead dominates | [state-machines.md](state-machines.md) |

---

## Decision Framework

### When to Use Each Technique

```
Is the operation compute-bound or memory-bound?
├── Compute-bound → Consider wider SIMD (AVX-512)
└── Memory-bound → Stick with AVX2/NEON

Is the bottleneck a hot loop?
├── Yes → Profile which operation dominates
│   ├── Branch-heavy → Use lookup tables or branchless
│   ├── Many small allocations → Use buffer reuse or arenas
│   └── Random access → Consider sorting or cumulative index
└── No → Focus elsewhere

Is data access sequential or random?
├── Sequential → Hardware prefetcher handles it
│   └── Consider exponential search for queries
└── Random → This is expensive
    ├── Can you sort indices first?
    ├── Can you use a smaller data structure?
    └── Can you precompute (cumulative index)?

How many states in your FSM?
├── 2-4 states → Pack into single table lookup
├── 5-16 states → Standard table-driven
└── Many states → Consider minimisation
```

---

## Key Principles

### 1. Measure First

> "Premature optimization is the root of all evil" - Knuth

Always profile before optimising. The bottleneck is often not where you expect.

### 2. Simpler Is Often Faster

The DSV lightweight index (simple array) beat the theoretically optimal 3-level BitVec by 5-9x due to better cache behaviour.

### 3. Wider SIMD ≠ Faster

AVX-512 is slower than AVX2 for memory-bound workloads. The memory system can't keep up with wider vectors.

**Evidence:**
- JSON parsing: AVX-512 was 7-17% slower, removed from codebase
- YAML parsing: AVX-512 was 7% slower at realistic 64B chunks
- Pattern: Sequential text parsing saturates RAM bandwidth at AVX2 width already

### 4. Algorithmic > Micro

A 627x speedup from cumulative index (algorithmic) dwarfs any SIMD optimization.

### 5. Profile Against Production

PFSM batched was 40% faster than reference but 25% slower than production. Always benchmark against what you're replacing.

### 6. Benchmark Design Matters

Micro-benchmarks must reflect real usage patterns, not just measure primitive operations in isolation.

**YAML P8 lesson**: Benchmark measured loop iterations (AVX-512: 4 iterations for 256B, AVX2: 8 iterations) and showed "2x speedup" - but real parsing does the same total work for both. The apparent win was measuring *fewer function calls*, not *more work per call*.

**Pattern to avoid**: Comparing different loop structures instead of comparing equivalent work.

---

## Reading Order

For newcomers to performance optimization:

1. **[cache-memory.md](cache-memory.md)** - Understanding memory hierarchy is fundamental
2. **[access-patterns.md](access-patterns.md)** - How access patterns affect cache
3. **[hierarchical-structures.md](hierarchical-structures.md)** - Data structure design for performance
4. **[bit-manipulation.md](bit-manipulation.md)** - Foundational bit tricks
5. **[lookup-tables.md](lookup-tables.md)** - Trading memory for computation
6. **[branchless.md](branchless.md)** - Eliminating branch mispredictions
7. **[simd.md](simd.md)** - Parallel processing with vectors
8. **[zero-copy.md](zero-copy.md)** - Avoiding unnecessary work
9. **[parallel-prefix.md](parallel-prefix.md)** - Cumulative operations
10. **[state-machines.md](state-machines.md)** - Optimising parsers

---

## References

### Books

- Knuth, D. E. "The Art of Computer Programming" (Volumes 1-4A)
- Warren, H. S. "Hacker's Delight" (2nd ed., 2012)
- Hennessy & Patterson "Computer Architecture: A Quantitative Approach"

### Papers

- Vigna, S. "Broadword Implementation of Rank/Select Queries" (2008)
- Zhou et al. "Space-Efficient, High-Performance Rank & Select" (2013) - Poppy
- Langdale & Lemire "Parsing Gigabytes of JSON per Second" (2019)
- Mytkowicz et al. "Data-Parallel Finite-State Machines" (2014)
- Navarro & Sadakane "Fully Functional Succinct Trees" (2014)

### Online Resources

- Drepper, U. "What Every Programmer Should Know About Memory"
- Fog, A. "Optimizing Software in C++"
- Intel "Intrinsics Guide" - https://www.intel.com/content/www/us/en/docs/intrinsics-guide
- ARM "NEON Intrinsics Reference"
- Anderson, S. "Bit Twiddling Hacks" (Stanford Graphics)

---

## Contributing

When adding new optimization documentation:

1. Create a new `.md` file in this directory
2. Follow the existing structure:
   - Overview table
   - Technique explanations with code examples
   - Usage in succinctly (file locations, results)
   - Key lessons
   - References
3. Update this README with the new document
4. Link to relevant source files using relative paths

---

## See Also

- [../archive/OPTIMIZATION-SUMMARY.md](../archive/OPTIMIZATION-SUMMARY.md) - Historical record of all optimizations applied to succinctly
- [../../.claude/skills/](../../.claude/skills/) - Claude Code skill files for specific topics
