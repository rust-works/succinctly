# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Opt-in strict YAML validation** (#223): a new `succinctly::yaml::validate`
  pass, exposed as `succinctly yaml validate [FILES]...` and `syq --validate`,
  that rejects invalid YAML. It mirrors `json validate` — a separate pass run
  before indexing, so the default non-validating loader path is unchanged and
  pays nothing. It rejects 59 of the 83 previously-accepted-but-invalid YAML
  Test Suite cases (reject conformance 11/94 → 70/94) with no false positives on
  the valid corpus; the remaining structurally-deep cases stay on record.

### Removed

- Removed four never-constructed `YamlError` variants — `InvalidEscape`,
  `InvalidIndentation`, `ExplicitKeyNotSupported`, and `ColonWithoutSpace`
  (#223). **Breaking**: exhaustive `match`es on the public `YamlError` lose four
  arms. The opt-in validator's `YamlValidationError` is the real rejection
  surface.

### Fixed

- **CRLF and lone-CR line breaks in YAML** (#324): a `\r` was folded into every
  *plain* scalar as a trailing space, which also destroyed type resolution — a
  Windows-authored `a: 1` loaded as the string `"1 "` rather than the number `1`,
  and `a: true` as `"true "`. There was no error and no warning, and the output
  was well-formed JSON, so nothing downstream could detect it. Quoted scalars and
  LF input were unaffected, which is why the whole suite missed it: every fixture
  and benchmark input in the repo uses LF. The fix treats `\r\n` and a lone `\r`
  as line breaks throughout — plain scalar and key extents, document markers,
  blank lines, comment termination, block-scalar content and chomping, `raw_bytes`,
  and the strict validator — per YAML 1.2 §5.4. `succinctly yq` now produces
  byte-identical output for a document whichever of the three break forms it uses.
  Correctness here has a measured price on LF input: `yaml_bench` index build is
  +14.9% median on x86 (7950X) and +6.9% on ARM (M4 Pro) excluding block scalars,
  which are 8–18% *faster* on x86; end-to-end `yq` on a 1 MB document moves
  +1.8% (`.`) to +6.4% (`.[].name`). See `docs/parsing/yaml.md` for the
  per-change attribution and the const-generic option that would buy it back.
- **YAML anchors on sequence items whose value is a collection** (#328): `- &m`
  followed by an indented mapping was read as a multi-line plain scalar, so
  `list:\n  - &m\n    k: v\n  - *m` came out as `{"list":["k"],"v":["k"]}` — a
  well-formed but wrong document, with no error raised. The flow form
  `- &first {id: 1}` corrupted differently, swallowing the anchor into the key
  text so the alias resolved to `null`. `parse_sequence_item_inner` now consumes
  the anchor before deciding the item's node type, and `- &a k: v` binds the
  anchor to the key as `yq` does. Sequences as explicit-key values (`? k` /
  `: - &m`) route through the same parser instead of an inlined copy of its
  dispatch, so they are fixed too.
- **YAML anchors that never named a node** (#328): three further anchor-target
  bugs, found by a new whole-corpus invariant that every anchor must point at a
  node's opening parenthesis.
  - An anchor on a **flow mapping key** (`a: { &e e: f }`) bound to the value
    rather than the key, so `*e` yielded `"f"` where `yq` gives `"e"`.
  - An anchor on an **explicit value that turns out to be null** (`? e` / `: &a`)
    had nothing to point at, so `*a` resolved to the following key — and inside
    a sequence it landed on the alias's own node and raised a spurious
    `AliasCycle` error on a valid document.
  - A **block sequence at a lower indent than a mapping key** was treated as
    that key's value, leaving the key's anchor dangling.
- `jq -R -s` now yields the entire input as a single string instead of an array of per-line strings, matching jq (#176)
- `yq -R -s` now yields the entire input as a single string instead of an array of per-line strings, matching jq and `jq -R -s` (#271)
- YAML alias cycles (`a: &anchor {self: *anchor}`) are rejected at index build with the
  new `YamlError::AliasCycle` variant instead of aborting with a stack overflow when the
  value is materialized (#153). Matches `yq`, which fails at decode time on the same
  input. Note: exhaustive `match`es on `YamlError` need a new arm.
- **BalancedParens L2 excess overflow** (#188): the per-L2-block excess
  counters were `i16` and overflowed at nesting depth > 32,767 (debug panic /
  silent wrap in release). Widened to `i32` across the scalar, NEON, and
  SSE4.1 build paths; deep nesting is no longer bounded by the index.
- **BalancedParens stray-bit over-count** (#188): constructors did not mask
  1-bits above `len` in the final partial word, inflating
  `total_ones`/`rank1`/`select1`. Owned constructors now canonicalize the
  final word in place; borrowed (`from_words*`, mmap) paths mask on read.
- **SelectIndex sample overflow** (#188): `SampleEntry` counters were `u32`
  and wrapped past 2^32 set bits (~512 MB of ones); widened to `u64`.

### Changed

- **A tag on an anchored sequence item is now rejected** (#328): `- &a !!str x`
  previously parsed as the plain scalar `"!!str x"`; consuming the anchor before
  dispatching means the tag is seen rather than absorbed, so it now returns
  `YamlError::TagNotSupported`. Consistent with `a: !!str 1`, which already
  errored. Tags remain documented non-support (#224).
- **Self-referential anchors on sequence items are now rejected** (#328):
  `- &m\n  - *m` records a real alias edge for the first time and is caught by
  the existing `AliasCycle` check, where it previously produced garbage. `yq`
  instead emits a depth-limited expansion; rejecting cycles is the documented
  policy (see `docs/compliance/yaml/limitations.md`).
- **4 GiB input ceilings enforced** (#188): instead of silently truncating
  `u32` counters, builds now fail loudly for inputs over `u32::MAX` bytes —
  `YamlIndex::build` returns the new `YamlError::InputTooLarge` variant
  (minor API addition: exhaustive matches on `YamlError` gain an arm), while
  `JsonIndex` and `DsvIndexLightweight` constructors panic with a documented
  message. `BalancedParens` constructors assert a `u32::MAX`-bit ceiling.
  See [docs/reference/limits.md](docs/reference/limits.md).
- **SelectIndex sample entries doubled** from 8 to 16 bytes (one entry per
  `sample_rate` set bits; ~6% of set-bit count at the default rate 256, up
  from ~3%). Serialized (`serde`) representations of `BitVec`,
  `BalancedParens`, and `SelectIndex` change accordingly.

## [0.7.0] - 2026-04-05

### Added

- **JSON Validation**
  - Strict RFC 8259 JSON validator with CLI command (`succinctly json validate`)
  - `--validate` flag for `jq` command to enforce strict validation before processing
  - Comprehensive RFC 8259 compliance test suite

- **Benchmark Infrastructure**
  - JSON validation benchmark suite with criterion
  - Criterion extra args support in benchmark runner

### Performance

- **Zero-copy JSON string output**: eliminates allocation for unescaped strings
- **SIMD-accelerated string escaping**: faster JSON output with vectorized escape detection
- **Lazy string slicing**: defers string slice operations for reduced allocation

### Fixed

- `popcount_words` return type changed from `u32` to `usize` to prevent overflow on large bitvectors (#139)
- JSON container range lookup replaced BP-based method with correct linear scan (#138)

### CI

- ARM64 runner added to coverage matrix
- Separate coverage reports for default, simd, and portable-popcount feature flags

## [0.6.0] - 2026-02-03

### Added

- **jq Compatibility Enhancements**
  - Comprehensive null handling: array indexing, object operations, and built-in functions return null instead of errors
  - String slicing with character-based indexing, negative indices, and Unicode support
  - Overflow handling that converts to float on integer overflow
  - Division and modulo by zero error handling with proper error messages
  - `has()` and `in()` functions properly reject negative array indices
  - `split()` handles empty delimiter by splitting into individual characters
  - `@uri`, `@html`, `@sh` format functions accept and convert non-string types
  - `first`, `last`, `nth` return null for empty/null inputs
  - `reverse()` returns empty array for null input
  - `getpath()` traverses null values gracefully

- **yq Compatibility Enhancements**
  - yq-compatible evaluation mode with different arithmetic semantics (wrapping overflow, infinity for division by zero)
  - Compile-time evaluation semantics system via `EvalSemantics` trait
  - `JqSemantics` and `YqSemantics` marker types for zero-cost abstraction
  - Negative array indexing support for `has()` and `in()` in yq mode

### Changed

- **Breaking**: `eval()` and `eval_lenient()` now require a semantics type parameter (`JqSemantics` or `YqSemantics`)
- **Breaking**: Removed `set_eval_mode()` and `get_eval_mode()` functions (replaced by compile-time generics)
- Replaced runtime evaluation mode switching with compile-time generic semantics for zero-overhead mode selection

### Performance

- Eliminated runtime mode checking branches in arithmetic operations through monomorphization

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

[Unreleased]: https://github.com/rust-works/succinctly/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/rust-works/succinctly/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/rust-works/succinctly/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/rust-works/succinctly/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/rust-works/succinctly/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/rust-works/succinctly/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/rust-works/succinctly/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/rust-works/succinctly/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/rust-works/succinctly/releases/tag/v0.1.0
