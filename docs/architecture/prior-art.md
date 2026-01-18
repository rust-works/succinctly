# Prior Art and References

[Home](/) > [Docs](../) > [Architecture](./) > Prior Art

This document records the academic papers, prior implementations, and key references that informed succinctly's design.

---

## Academic Foundations

### Rank and Select

| Paper | Authors | Year | Contribution |
|-------|---------|------|--------------|
| [Broadword Implementation of Rank/Select Queries](https://vigna.di.unimi.it/ftp/papers/Broadword.pdf) | Vigna | 2008 | Broadword algorithms for 64-bit operations |
| [Space-Efficient, High-Performance Rank & Select](https://www.cs.cmu.edu/~dga/papers/zhou-sea2013.pdf) | Zhou, Andersen, Kaminsky | 2013 | Poppy structure with 3% overhead |

**Succinctly implementation**: [src/bits/](../../src/bits/) implements Poppy-style 3-level rank directory with ~3% space overhead.

### Balanced Parentheses

| Paper | Authors | Year | Contribution |
|-------|---------|------|--------------|
| Fully-Functional Succinct Trees | Sadakane & Navarro | 2010 | RangeMin structure for O(1) tree navigation |
| Optimal Succinctness for Parentheses | Navarro & Sadakane | 2014 | Space-optimal BP representation |

**Succinctly implementation**: [src/trees/bp.rs](../../src/trees/bp.rs) implements hierarchical RangeMin with ~6% overhead.

### JSON Semi-Indexing

| Paper | Authors | Year | Contribution |
|-------|---------|------|--------------|
| [Parsing Gigabytes of JSON per Second](https://arxiv.org/abs/1902.08318) | Langdale & Lemire | 2019 | SIMD-accelerated JSON parsing (simdjson) |
| [Data-Parallel Finite-State Machines](https://www.microsoft.com/en-us/research/publication/data-parallel-finite-state-machines/) | Mytkowicz et al. | 2014 | PFSM for parallel parsing |

**Succinctly implementation**: [src/json/](../../src/json/) uses PFSM with table-driven state machine, achieving ~700 MiB/s throughput.

### Bit Manipulation

| Paper | Authors | Year | Contribution |
|-------|---------|------|--------------|
| [Faster Population Counts Using AVX2 Instructions](https://arxiv.org/abs/1611.07612) | Mula, Kurz, Lemire | 2016 | Harley-Seal popcount algorithm |

**Succinctly implementation**: [src/bits/popcount.rs](../../src/bits/popcount.rs) uses AVX-512 VPOPCNTDQ when available (5.2x speedup).

---

## Haskell-Works Heritage

Succinctly is a Rust reimplementation of techniques from the [haskell-works](https://github.com/haskell-works) ecosystem, originally developed by John Ky.

### Core Packages

| Haskell Package | Succinctly Module | Purpose |
|-----------------|-------------------|---------|
| [hw-rankselect](https://github.com/haskell-works/hw-rankselect) | `src/bits/` | Rank/select data structures |
| [hw-balancedparens](https://github.com/haskell-works/hw-balancedparens) | `src/trees/` | Balanced parentheses operations |
| [hw-json](https://github.com/haskell-works/hw-json) | `src/json/` | JSON semi-indexing |
| [hw-json-simd](https://github.com/haskell-works/hw-json-simd) | `src/json/pfsm*.rs` | SIMD-accelerated JSON parser |
| [hw-dsv](https://github.com/haskell-works/hw-dsv) | `src/dsv/` | DSV/CSV parsing |

### Key Concepts Ported

1. **Semi-indexing**: Build structural index without parsing values
2. **Interest Bits (IB)**: Track positions of structural characters
3. **Balanced Parentheses (BP)**: Encode document structure
4. **Cursor API**: Lazy navigation through indexed documents

### Differences from Haskell Implementation

| Aspect | Haskell | Rust |
|--------|---------|------|
| Memory model | GC-managed | Zero-copy, no_std compatible |
| SIMD | FFI to C | Native intrinsics via `std::arch` |
| ARM support | Limited (disabled) | Full NEON implementation |
| Streaming | Lazy evaluation | Explicit iterators |

### ARM/NEON Portability

The Haskell packages disable SIMD on ARM (`base < 0` constraint). Succinctly provides full ARM support:

| x86_64 Instruction | ARM/NEON Equivalent | Notes |
|--------------------|---------------------|-------|
| `_mm256_cmpeq_epi8` | `vceqq_u8` (128-bit) | Process 16 bytes vs 32 |
| `_mm256_movemask_epi8` | Manual extraction | Multi-instruction sequence |
| `_pdep_u64` | Software emulation | No ARM equivalent |
| `_pext_u64` | Software emulation | No ARM equivalent |

---

## Related Projects

### JSON Parsers

| Project | Language | Technique | Performance |
|---------|----------|-----------|-------------|
| [simdjson](https://github.com/simdjson/simdjson) | C++ | SIMD + tape | >2 GB/s |
| [simd-json](https://github.com/simd-lite/simd-json) | Rust | simdjson port | ~1 GB/s |
| [sonic-rs](https://github.com/cloudwego/sonic-rs) | Rust | SIMD + LazyValue | ~800 MiB/s |
| succinctly | Rust | Semi-indexing | ~700 MiB/s |

**Succinctly's advantage**: 18-46x less memory than DOM parsers due to lazy evaluation.

### Succinct Data Structure Libraries

| Library | Language | Focus |
|---------|----------|-------|
| [sdsl-lite](https://github.com/simongog/sdsl-lite) | C++ | Comprehensive succinct DS |
| [succinct](https://crates.io/crates/succinct) | Rust | Basic rank/select |
| [vers-vecs](https://crates.io/crates/vers-vecs) | Rust | Fast pure-Rust rank/select |

---

## Books and General References

### Books

- Knuth, D. E. *The Art of Computer Programming* (Volumes 1-4A)
- Warren, H. S. *Hacker's Delight* (2nd ed., 2012)
- Hennessy & Patterson *Computer Architecture: A Quantitative Approach*

### Online Resources

- Drepper, U. [What Every Programmer Should Know About Memory](https://people.freebsd.org/~lstewart/articles/cpumemory.pdf)
- Fog, A. [Optimizing Software in C++](https://www.agner.org/optimize/optimizing_cpp.pdf)
- [Intel Intrinsics Guide](https://www.intel.com/content/www/us/en/docs/intrinsics-guide)
- [ARM NEON Intrinsics Reference](https://developer.arm.com/architectures/instruction-sets/intrinsics/)
- Anderson, S. [Bit Twiddling Hacks](https://graphics.stanford.edu/~seander/bithacks.html) (Stanford Graphics)

---

## See Also

- [semi-indexing.md](semi-indexing.md) - How semi-indexing works
- [../optimizations/](../optimizations/) - Optimization techniques used
- [../optimizations/history.md](../optimizations/history.md) - Record of optimization attempts
