# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.1] - 2026-02-02

### Fixed

- `jq`: Return `null` for missing fields on objects instead of error (issue #61)

### Changed

- Refactored `json_simd` benchmark into focused components for better maintainability
- Removed unused `count_seq_items_before` method from YAML parser

### Documentation

- Added Apple M4 Pro benchmark results
- Updated ARM Neoverse-V2 benchmark results

## [0.5.0] - 2026-01-31

### Added

- **CLI Enhancements**
  - Multi-call binary support: `sjq`, `syq`, `sjq-locate`, `syq-locate` symlinks
  - `succinctly install-aliases` command to create symlinks
  - Unified benchmark runner (`succinctly bench run`) with comprehensive metadata tracking
  - Default memory collection for CLI benchmarks

- **YAML Streaming (M2.5)**
  - Direct YAML→JSON streaming for navigation queries (`.[0]`, `.[]`)
  - Eliminates intermediate `OwnedValue` DOM for 2-3x faster identity queries
  - 3-4% of yq's memory usage on large files

- **Memory Optimizations**
  - Advance Index: memory-efficient `bp_to_text` mapping with ~1.5x compression
  - EndPositions: 2-bitmap encoding for scalar end positions
  - Sequential cursor optimization for amortized O(1) position lookups
  - Elias-Fano encoding for monotone integer sequences
  - CompactRank two-level directory for O(1) rank queries
  - In-place builder for cache-aligned L1L2 storage

- **SIMD Optimizations**
  - AVX2-accelerated JSON escape scanning for YAML→JSON output on x86_64
  - ARM64 NEON escape scanning for JSON output (4-12x faster on long strings)
  - BMI2 PDEP support for O(1) select-in-word on x86_64

- **yq Compatibility**
  - Key ordering in yq mode: object keys output in document order (matching `jq -yy`)

### Changed

- Build regression mitigation: inline zero-filling and lazy newline index (P12-A)

### Fixed

- `keys` function ordering now compatible with yq mode (returns keys in document order)
- `no_std` compatibility: added missing `alloc::boxed::Box` import
- Elias-Fano: fixed `no_std` and rustdoc compatibility
- Flaky CI: implemented cargo retry logic for test stability

### Performance

- yq identity queries: 20-25% faster on 1MB files (P12 Advance Index)
- yq small-medium files: 3-13% faster (O1 sequential cursor)
- YAML parsing: 11-85% faster build times (P12-A mitigations)
- Escape scanning: 4-12x faster with SIMD (O3)

## [0.4.0] - 2026-01-24

### Added

- **jq Language Enhancements**
  - `at_offset(n)` builtin for position-based navigation to node at byte offset
  - `at_position(line; col)` builtin for navigation to node at line/column position

- **SIMD Optimizations**
  - SSE4.1 PHMINPOSUW optimization for balanced parentheses index building on x86_64
  - SVE2 BDEP `select_in_word` with runtime dispatch on ARM64
  - NEON VMINV L1/L2 index building optimizations for ARM64
  - 256-byte popcount unrolling for improved ARM performance
  - NEON PMULL carryless multiply for prefix XOR optimization

- **Balanced Parentheses Enhancements**
  - Zero-cost `SelectSupport` trait abstraction (`NoSelect` for JSON, `WithSelect` for YAML)
  - O(log n) BP lookup via binary search on `bp_to_text` mapping
  - Unrolled lookup optimization for min excess computation

### Fixed

- YAML `yq-locate` text-position-to-BP mapping now returns correct nodes (issue #26)
- Flaky `cargo run` in jq CLI tests with retry logic

### Performance

- BP select1 queries: 2.5-5.9x faster with sampled select index
- `yq-locate` offset queries: 16-492x speedup with indexed `find_open`

## [0.3.0] - 2026-01-21

### Added

- **YAML Enhancements**
  - `yq-locate` command for finding YAML positions by offset or line/column
  - Multi-document stream support with `--doc N` and `--slurp` options
  - Quoted string type preservation (yq-compatible output)
  - YAML metadata access: `tag`, `anchor`, `alias`, `style`, `kind`, `key`, `line`, `column`
  - Handle explicit empty keys and implicit null values in YAML mappings

- **jq Language Enhancements**
  - `load(file)` operator for external YAML/JSON file loading
  - `split_doc` operator for multi-document YAML output
  - `@props` format encoder for Java properties output
  - `@yaml` format function for YAML encoding
  - yq date extensions: `from_unix`, `to_unix`, `tz(zone)` with IANA timezone support
  - `pivot` builtin for array/object transposition
  - `shuffle` operator for random array reordering
  - `document_index`/`di` operator for multi-doc YAML indexing
  - `omit(keys)` operator for objects and arrays
  - Generic evaluator for direct YAML evaluation without JSON conversion
  - `skip(n; expr)` iteration control builtin
  - `combinations` function for generating combinations
  - Non-local control flow with `label $name | ... | break $name`
  - Regular expressions: `match`, `capture`, `scan`, `splits`, `sub`, `gsub`
  - `$__loc__` for source location tracking, `$ENV` for environment access
  - Module system with `import`, `include`, and namespace support
  - `trunc` math function for truncation toward zero
  - `toboolean` type conversion builtin
  - `pick()` function for selective key extraction
  - Comment support with `#` hash syntax
  - Quoted field access and bracket string notation
  - `key` function for yq iteration context
  - `kind` function for yq node type classification
  - `tojson` and `fromjson` builtins

- **CLI Improvements**
  - `--raw-input` (`-R`) option for line-by-line processing in yq
  - `--slurp` (`-s`) option for collecting all inputs into array
  - `--doc N` option for multi-document selection in yq

### Fixed

- Handle explicit empty keys in YAML mappings
- Emit explicit null nodes for implicit null values in YAML mappings
- Make `paths` and `paths(filter)` stream individual results correctly
- Correct `repeat` builtin to evaluate with original input
- Support any type for `indices`/`index`/`rindex` on arrays
- Make `leaf_paths` stream individual paths
- Enable postfix operations on builtin expressions
- Negative index support for `getpath`
- Replace std with core/alloc for no_std compatibility

### Performance

- YAML identity queries: 90-217 MiB/s (2.3x improvement with direct streaming)
- yq vs system yq: 16-40x faster on x86_64, 1.9-8.7x faster on ARM

## [0.2.0] - 2026-01-18

### Added

- **YAML Semi-Indexing**
  - Complete YAML parser with oracle-style parsing (~250-400 MiB/s)
  - `yq` CLI command for YAML processing with jq syntax
  - Direct YAML-to-JSON streaming (2.3x faster than DOM conversion)
  - Multi-document stream support with virtual root wrapper
  - Anchor and alias resolution at parse time
  - Block scalar support (literal `|` and folded `>` styles)
  - Flow style parsing (inline arrays and objects)
  - Explicit key/value indicators (`?` and `:`)
  - SIMD optimizations: anchor/alias scanning (6-17% improvement), block scalar parsing (19-25% improvement)

- **DSV/CSV Semi-Indexing**
  - High-performance CSV/TSV parser with succinct indexing (85-1676 MiB/s API, 11-169 MiB/s CLI)
  - `--input-dsv` flag for jq command to read CSV/TSV input
  - `@dsv(delimiter)` format function for custom delimiter output
  - BMI2 PDEP acceleration for quote masking on x86_64
  - Lightweight cumulative rank index (1.8-4.3x faster than BitVec)
  - SIMD-accelerated parsing on both x86_64 (AVX2) and ARM (NEON)

- **jq Enhancements**
  - `jq-locate` command for finding JSON positions by offset or line/column
  - Assignment operators: `=`, `|=`, `+=`, `-=`, `*=`, `/=`, `%=`, `//=`, `del()`
  - Path operations: `path()`, `paths`, `leaf_paths`, `getpath`, `setpath`, `delpaths`
  - Date/time functions: `now`, `gmtime`, `localtime`, `strftime`, `strptime`, `todate`, `fromdate`
  - Type filters: `values`, `nulls`, `booleans`, `numbers`, `strings`, `arrays`, `objects`, `scalars`, `iterables`
  - Math functions: all 34 standard jq math functions
  - Lazy evaluation with identity fast path (zero-allocation for `.` queries)
  - JSON sequence format (RFC 7464) support with `--seq`
  - ASCII escaping (`-a` flag) and ANSI color syntax highlighting (`-C` flag)
  - `$ARGS` variable and positional argument support (`--args`, `--jsonargs`)
  - Build configuration reporting flag (`--build-configuration`)
  - Unary minus operator for expression negation

- **SIMD Enhancements**
  - Portable broadword module for non-SIMD platforms
  - Block scalar SIMD optimization with AVX2 newline scanning
  - SWAR (SIMD Within A Register) classification for ARM64

### Changed

- jq-compatible number formatting is now the default behavior
- Renamed `--no-jq-compat` to `--preserve-input` for clarity

### Fixed

- `enclose()` word boundary bug with zero-excess words in balanced parentheses
- `no_std` compatibility issues in SIMD modules

### Performance

- YAML parsing: 250-400 MiB/s (oracle parser)
- DSV parsing: 85-1676 MiB/s (API), 11-169 MiB/s (CLI)

## [0.1.0] - 2026-01-11

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
  - Table-driven PFSM parser achieving 880 MiB/s throughput on x86_64 (AMD Zen 4)
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
  - `--jq-compat` flag and `SUCCINCTLY_JQ_COMPAT=1` env var for exact jq output compatibility
  - Multiple output formats and memory-mapping support

- **Platform Support**
  - `no_std` compatible (with `alloc`)
  - x86_64 with AVX2, AVX-512, SSE4.2, SSE2
  - aarch64 with NEON
  - Optional `serde` serialization support

### Performance (x86_64 AMD Ryzen 9 7950X)

- JSON semi-indexing: 880 MiB/s (PFSM), 732 MiB/s (AVX2)
- Rank queries: ~3 ns (O(1))
- Select queries: ~50 ns (O(log n))
- Popcount: 96.8 GiB/s (AVX-512), 18.5 GiB/s (scalar)

[Unreleased]: https://github.com/rust-works/succinctly/compare/v0.5.1...HEAD
[0.5.1]: https://github.com/rust-works/succinctly/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/rust-works/succinctly/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/rust-works/succinctly/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/rust-works/succinctly/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/rust-works/succinctly/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/rust-works/succinctly/releases/tag/v0.1.0
