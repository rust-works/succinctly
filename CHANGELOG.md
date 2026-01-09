# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Core Data Structures**
  - `BitVec` with O(1) rank and O(log n) select operations
  - 3-level Poppy-style rank directory with ~3% space overhead
  - Sampled select index with configurable sample rate (~1-3% overhead)
  - `RankSelect` trait for generic rank/select operations

- **Balanced Parentheses**
  - `BalancedParens` structure for succinct tree navigation
  - RangeMin hierarchical min-excess index (~6% overhead)
  - O(1) `find_close`, `find_open`, `enclose` operations
  - Tree navigation: `first_child`, `next_sibling`, `parent`, `subtree_size`

- **JSON Semi-Indexing**
  - Interest Bits (IB) and Balanced Parentheses (BP) encoding
  - Table-driven PFSM parser achieving 950 MiB/s throughput
  - `JsonIndex` for building semi-indices from JSON bytes
  - `StandardJson` cursor for lazy navigation without full parsing

- **SIMD Acceleration**
  - AVX2 SIMD JSON parser (32 bytes/iteration, 78% faster than SSE2)
  - AVX-512 VPOPCNTDQ popcount (5.2x faster than scalar)
  - SSE4.2 with PCMPISTRI for character classification
  - ARM NEON support (mandatory on aarch64)
  - Runtime CPU feature detection for optimal dispatch

- **jq Query Language**
  - Path expressions: `.foo`, `.[0]`, `.[-1]`, `.[]`
  - Array slicing: `.[2:5]`, `.[2:]`, `.[:5]`
  - Chained access: `.foo.bar`, `.foo[0].bar`
  - Optional access: `.foo?`
  - Comma operator: `.foo, .bar`
  - Array/object construction: `[.foo]`, `{foo: .bar}`
  - Recursive descent: `..`
  - Literals: `null`, `true`, `false`, numbers, strings
  - Arithmetic: `+`, `-`, `*`, `/`, `%`
  - Comparison: `==`, `!=`, `<`, `<=`, `>`, `>=`
  - Boolean operators: `and`, `or`, `not`
  - Alternative operator: `//`
  - Conditionals: `if-then-else-end`
  - Error handling: `try-catch`, `error()`

- **CLI Tool**
  - `json generate` - Generate synthetic JSON for benchmarking
  - `jq` - jq-compatible command-line JSON processor
  - Multiple output formats and memory-mapping support

- **Platform Support**
  - `no_std` compatible (with `alloc`)
  - x86_64 with AVX2, AVX-512, SSE4.2, SSE2
  - aarch64 with NEON
  - Optional `serde` serialization support

### Performance

- JSON semi-indexing: 950 MiB/s (PFSM), 732 MiB/s (AVX2)
- Rank queries: ~3 ns (O(1))
- Select queries: ~50 ns (O(log n))
- Popcount: 96.8 GiB/s (AVX-512), 18.5 GiB/s (scalar)

[Unreleased]: https://github.com/rust-works/succinctly/compare/v0.1.0...HEAD
