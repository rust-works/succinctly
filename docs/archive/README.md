# Documentation Archive

[Home](/) > [Docs](../) > Archive

This directory preserves historical documentation that provides context for past decisions.

## What's Here

### Optimization History
The `optimizations/` subdirectory contains the complete history of optimization attempts:
- **SUMMARY.md** - Comprehensive optimization timeline
- **failed.md** - Failed optimization attempts with analysis
- **implemented.md** - Successfully implemented optimizations
- **Performance analysis** - Historical benchmark results

### Haskell Reference Implementations
The `haskell-works/` subdirectory contains notes on the Haskell libraries that inspired this project:
- hw-json, hw-json-simd
- hw-dsv
- hw-rankselect, hw-balancedparens

### Other Historical Docs
- Migration notes
- Feature planning (completed features)
- Implementation explorations

## Why Keep Archives?

Archives prevent repeated mistakes by documenting:
- Why certain approaches don't work
- Performance characteristics of rejected implementations
- Evolution of the codebase architecture

## Notable Archived Insights

- **AVX-512 is slower than AVX2** for memory-bound workloads (7-17% regression)
- **Simpler data structures often outperform complex ones** (DSV lightweight index)
- **Micro-benchmarks can be misleading** (YAML P2.8, P3, P5-P8 all failed despite micro-bench wins)
- **Wider SIMD != faster** when memory-bound
- **Branch prediction (93-95% accurate)** often beats lookup tables

## File Organization

```
archive/
├── optimizations/          # Optimization history
│   ├── SUMMARY.md         # Full timeline
│   ├── failed.md          # What didn't work
│   ├── implemented.md     # What worked
│   └── ...                # Detailed analyses
├── haskell-works/         # Reference implementations
│   ├── hw-json.md
│   ├── hw-dsv.md
│   └── ...
└── ...                    # Other historical docs
```
