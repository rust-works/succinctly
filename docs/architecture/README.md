# Architecture

[Home](/) > [Docs](../) > Architecture

Design documentation for succinctly's core data structures and algorithms.

## Core Concepts

| Document | Description |
|----------|-------------|
| [Core Concepts](core-concepts.md) | Introduction to succinct data structures |
| [BitVec](bitvec.md) | Bit vector with O(1) rank and O(log n) select |
| [Balanced Parentheses](balanced-parens.md) | Succinct tree encoding |
| [Semi-Indexing](semi-indexing.md) | JSON/YAML/DSV parsing approach |
| [Prior Art](prior-art.md) | Academic foundations and haskell-works heritage |

## Module Structure

```
src/
├── lib.rs              # Public API, RankSelect trait
├── bits/               # BitVec, rank/select, popcount
├── trees/              # Balanced parentheses
├── json/               # JSON semi-indexing (PFSM default)
├── yaml/               # YAML semi-indexing (oracle parser)
├── dsv/                # DSV (CSV/TSV) semi-indexing
├── jq/                 # jq query language
└── bin/                # CLI tool
```

## Key Design Decisions

1. **`no_std` Compatible**: Core library works without std
2. **Zero-Copy Parsing**: Parse without copying input data
3. **Platform-Specific SIMD**: AVX2 on x86_64, NEON on ARM
4. **Lazy Evaluation**: Only materialize accessed values

## See Also

- [Parsing Implementation](../parsing/) - Parser-specific details
- [Optimization Techniques](../optimizations/) - Performance patterns
- [Benchmarks](../benchmarks/) - Performance data
