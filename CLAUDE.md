# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Skills Reference

Detailed guidance is organized into skills in `.claude/skills/`. Claude will auto-invoke these based on context:

| Skill                  | Triggers                                    | Purpose                                    |
|------------------------|---------------------------------------------|--------------------------------------------|
| **benchmark-docs**     | "benchmark", "update benchmarks"            | Platform-specific benchmark documentation  |
| **markdown-tables**    | "format table", "align table"               | Fixed-width table formatting               |
| **simd-optimization**  | "SIMD", "AVX", "NEON", "vectorization"      | SIMD patterns and learnings                |
| **bit-optimization**   | "rank", "select", "popcount", "lookup table"| Bit-level optimization patterns            |
| **json-semi-indexing** | "JSON index", "semi-index", "cursor"        | JSON parsing implementation details        |
| **yaml-semi-indexing** | "YAML index", "YAML parser", "sequence item"| YAML parsing and debugging patterns        |
| **testing**            | "test", "assert", "coverage", "regression"  | Test quality patterns and anti-patterns    |
| **commit-msg**         | "commit message", "amend commit"            | Conventional commits format                |
| **knowledge-map**      | "knowledge map", "wiki", "update wiki"      | Maintain docs/ knowledge wiki              |
| **skill-writing**      | "create skill", "SKILL.md", "write skill"   | Best practices for writing Claude skills   |

## Knowledge Wiki

A structured knowledge base for this codebase lives in `docs/`. Start at [docs/index.md](docs/index.md) for a concept-oriented map of how the data structures, algorithms, SIMD implementations, and benchmarks relate to each other. The wiki pages cross-link to existing architecture docs, parsing docs, source files, and academic papers.

## AI Scratch Directory

Use `.ai/scratch/` for temporary files (git-ignored):

```bash
mkdir -p .ai/scratch
```

When manually exercising `jq`/`yq` CLI behavior (trying flags, checking output, `-i`
inplace edits), use a file under `.ai/scratch/`, not a tracked fixture elsewhere in the
repo (e.g. `users.yaml`) — those are committed samples, not scratch space.

## Project Overview

Succinctly is a high-performance Rust library implementing succinct data structures with fast rank and select operations, optimized for both x86_64 (POPCNT) and ARM (NEON) architectures.

### Semi-Indexing Architecture

Succinctly uses **semi-indexing** rather than traditional DOM parsing. Instead of building a complete in-memory tree, it creates a lightweight structural index (~3-6% overhead) and extracts values lazily. This enables:

- **18-46x less memory** than DOM parsers
- **Fast queries** because only accessed values are materialized
- **Streaming output** without intermediate allocations

**Trade-off**: Semi-indexing performs minimal validation compared to full parsers (jq, yq). The benchmark comparisons to jq/yq reflect both architectural advantages and reduced validation work. See [docs/architecture/semi-indexing.md](docs/architecture/semi-indexing.md) for details.

## Common Commands

### Building and Testing

```bash
cargo build                              # Standard build
cargo build --features simd              # With SIMD popcount
cargo test                               # Run tests
cargo test --features large-tests        # 1GB bitvector tests
cargo bench                              # Run benchmarks
```

### Coverage

CI runs coverage via the [`action-works/omni-dev-coverage-check`](https://github.com/action-works/omni-dev-coverage-check)
action (x86_64 + ARM64 matrix), which wraps `cargo-llvm-cov` + `omni-dev coverage diff` and
posts PR patch coverage as a sticky comment. To reproduce the CI line-coverage number locally,
use the same feature set CI uses:

```bash
# TOTAL line % (matches the CI `test-args`; the fail-under-lines gate is set just below this)
cargo llvm-cov --features cli,simd,regex,serde --workspace --summary-only --fail-under-lines 0

# PR diff / patch coverage (added lines covered + uncovered file:line list), like the CI comment
omni-dev coverage diff
```

### CLI Tool

```bash
cargo build --release --features cli

# Install short aliases (sjq, syq, sjq-locate, syq-locate)
./target/release/succinctly install-aliases          # symlinks next to binary
./target/release/succinctly install-aliases --dir ~/bin  # or specify directory

# JSON operations (sjq is an alias for succinctly jq)
sjq '.users[].name' input.json
sjq -r '.users[] | [.name, .age] | @csv' input.json
sjq -r '.users[] | [.name, .age] | @dsv("|")' input.json
sjq --validate '.users[]' input.json  # Strict RFC 8259 validation before processing
sjq-locate input.json --offset 42
sjq-locate input.json --line 5 --column 10

# JSON validation (RFC 8259 strict)
./target/release/succinctly json validate input.json
./target/release/succinctly json validate --quiet input.json  # Exit code only

# YAML operations (syq is an alias for succinctly yq)
syq '.users[].name' config.yaml
syq -o json '.' config.yaml         # Output as JSON
syq '.spec.containers[]' k8s.yaml
syq --doc 0 '.' multi-doc.yaml      # First document only
syq --jq-extensions '[paths]' config.yaml  # opt into jq-only builtins yq's lexer rejects (#1512)
syq-locate config.yaml --offset 42
syq-locate config.yaml --line 5 --column 10

# DSV/CSV operations
sjq --input-dsv ',' '.[] | select(.[0] == "Alice")' data.csv
./target/release/succinctly dsv generate 1mb -p users -o users.csv
./target/release/succinctly dsv generate 1mb -p users --no-header -o users.csv  # without header row

# Data generation
./target/release/succinctly json generate 10mb -o benchmark.json

# Benchmarks (requires: cargo build --release --features bench-runner)
./target/release/succinctly bench run jq_bench
./target/release/succinctly bench run yq_bench
./target/release/succinctly dev bench yq --queries all  # M2 streaming comparison (memory collected by default)
./target/release/succinctly bench run dsv_bench

# Distributed benchmark orchestration across SSH nodes (issue #98)
cp nodes.yaml.example nodes.yaml && $EDITOR nodes.yaml  # gitignored, machine-specific
./target/release/succinctly bench nodes --config nodes.yaml --status
./target/release/succinctly bench sync --config nodes.yaml
./target/release/succinctly bench orchestrate --config nodes.yaml --all --dry-run
./target/release/succinctly bench report --current data/bench/distributed/<run_id> --baseline <prior-run>
```

### Multi-call Aliases

The binary supports multi-call invocation via symlinks. When invoked as `sjq`, `syq`, etc., it dispatches directly to the corresponding subcommand:

| Alias         | Equivalent              | Installed by default |
|---------------|-------------------------|----------------------|
| `sjq`         | `succinctly jq`         | Yes                  |
| `syq`         | `succinctly yq`         | Yes                  |
| `sjq-locate`  | `succinctly jq-locate`  | Yes                  |
| `syq-locate`  | `succinctly yq-locate`  | Yes                  |
| `jq`          | `succinctly jq`         | No (recognized only) |
| `yq`          | `succinctly yq`         | No (recognized only) |

Run `succinctly install-aliases` to create symlinks, or create them manually:

```bash
ln -s $(which succinctly) ~/bin/sjq
```

## Code Architecture

### Module Structure

```
src/
├── lib.rs              # Public API, RankSelect trait
├── bits/               # BitVec, rank/select, popcount
├── trees/              # Balanced parentheses
├── json/               # JSON semi-indexing (PFSM default)
├── yaml/               # YAML semi-indexing (oracle parser)
├── dsv/                # DSV/CSV semi-indexing (BMI2/SIMD)
├── jq/                 # jq query language and evaluator
└── bin/                # CLI tool (jq, yq, jq-locate, yq-locate)
```

### Public API

```rust
use succinctly::bits::BitVec;
use succinctly::trees::BalancedParens;
use succinctly::json::JsonIndex;
use succinctly::yaml::YamlIndex;
use succinctly::dsv::DsvIndex;
use succinctly::jq::{parse, eval};
```

### Core Data Structures

| Structure         | Description                            | Performance (x86_64 Zen 4) |
|-------------------|----------------------------------------|----------------------------|
| **BitVec**        | O(1) rank, O(log n) select             | ~28-48% overhead           |
| **BalancedParens**| Succinct tree navigation               | ~6% overhead               |
| **JsonIndex**     | JSON semi-indexing with PFSM parser    | ~880 MiB/s                 |
| **YamlIndex**     | YAML semi-indexing with oracle parser  | ~250-400 MiB/s             |
| **DsvIndex**      | DSV semi-indexing with lightweight rank| 85-1676 MiB/s (API)        |

### jq Format Functions

The jq implementation supports format functions for converting values to strings:

| Format       | Syntax              | Description                                       | Example Output      |
|--------------|---------------------|---------------------------------------------------|---------------------|
| **@csv**     | `@csv`              | Comma-separated values (fixed delimiter)          | `"a","b","c"`       |
| **@tsv**     | `@tsv`              | Tab-separated values (fixed delimiter)            | `a\tb\tc`           |
| **@dsv**     | `@dsv(delimiter)`   | Generic DSV with custom delimiter                 | `"a"\|"b"\|"c"`     |
| **@json**    | `@json`             | JSON format                                       | `{"a":1}`           |
| **@text**    | `@text`             | Convert to string (same as tostring)              | `42`                |
| **@uri**     | `@uri`              | URI percent encoding                              | `hello%20world`     |
| **@urid**    | `@urid`             | URI percent decoding                              | `hello world`       |
| **@base64**  | `@base64`           | Base64 encoding                                   | `aGVsbG8=`          |
| **@base64d** | `@base64d`          | Base64 decoding                                   | `hello`             |
| **@html**    | `@html`             | HTML entity escaping                              | `&lt;script&gt;`    |
| **@sh**      | `@sh`               | Shell quoting                                     | `'hello world'`     |
| **@yaml**    | `@yaml`             | YAML flow-style encoding (yq)                     | `{a: 1, b: 2}`      |
| **@props**   | `@props`            | Java properties format (yq)                       | `key = value`       |

**@uri/@base64/@html on arrays/objects (jq mode only):** a container is JSON-encoded
first, then formatted, matching real jq (`[1,2] | @uri` => `"%5B1%2C2%5D"`) — `succinctly
yq` still rejects a container outright for all three, matching real yq's own
`cannot encode !!seq as URI/base64...` error (#1096).

**@dsv(delimiter) specifics:**
- Custom delimiters: Any single or multi-character string
- Always-quoted strings: every string field is double-quoted (matching jq's `@csv`), regardless of content; non-strings are bare and null is empty
- CSV-compatible: `@dsv(",")` produces identical output to `@csv`
- Escape handling: Inner `"` is doubled (`""`)

```bash
# Examples
echo '["a","b","c"]' | jq -r '@dsv("|")'        # Output: "a"|"b"|"c"
echo '["a","b|c","d"]' | jq -r '@dsv("|")'      # Output: "a"|"b|c"|"d"
echo '["a","b","c"]' | jq -r '@dsv(";")'        # Output: "a";"b";"c"
```

### jq Assignment Operators

The jq implementation supports assignment operators for modifying JSON in-place:

| Operator  | Syntax             | Description                                                    | Example                   |
|-----------|--------------------|----------------------------------------------------------------|---------------------------|
| **=**     | `.path = value`    | Simple assignment                                              | `.a = 42`                 |
| **\|=**   | `.path \|= filter` | Update assignment (applies filter to current value)            | `.a \|= . + 1`            |
| **+=**    | `.path += value`   | Compound add (equivalent to `.path \|= . + value`)             | `.count += 10`            |
| **-=**    | `.path -= value`   | Compound subtract                                              | `.health -= 25`           |
| ***=**    | `.path *= value`   | Compound multiply, or recursive object/array merge (see below) | `.scale *= 2`             |
| **/=**    | `.path /= value`   | Compound divide                                                | `.total /= 4`             |
| **%=**    | `.path %= value`   | Compound modulo                                                | `.index %= 10`            |
| **//=**   | `.path //= value`  | Alternative assignment (sets only if null/false)               | `.default //= "fallback"` |
| **del()** | `del(.path)`       | Delete field or array element                                  | `del(.temporary)`         |

```bash
# Examples
echo '{"a": 1}' | succinctly jq '.a = 42'           # {"a": 42}
echo '{"x": 5}' | succinctly jq '.x |= . * 2'       # {"x": 10}
echo '{"n": 10}' | succinctly jq '.n += 5'          # {"n": 15}
echo '{"a": null}' | succinctly jq '.a //= "default"'  # {"a": "default"}
echo '{"a": 1, "b": 2}' | succinctly jq 'del(.a)'   # {"b": 2}
echo '[1, 2, 3]' | succinctly jq '.[] |= . * 2'     # [2, 4, 6]
```

`*`/`*=` on two objects recursively merges them (matching values are merged if both are objects, otherwise the right side wins) — this works in both `jq` and `yq` mode, since real jq does the same. On two arrays, plain `*`/`*=` replaces the left side wholesale with the right (yq mode only; `succinctly jq` still errors on array `*`, matching real jq, which has no array-merge concept):

```bash
echo '{"a": {"x": 1}, "b": {"x": 2, "y": 3}}' | succinctly jq '.a *= .b'  # {"a":{"x":2,"y":3},...}
printf 'a: [1, 2]\nb: [3, 4]\n' | succinctly yq '.a *= .b'               # a: [3, 4]
```

### The rule the divergence sections below are exceptions to

**`succinctly jq` follows jq; `succinctly yq` follows yq — and the *mode* decides, never the
input format.** Bug-for-bug fidelity is the default, a reference tool's own inconsistencies
included. Divergence is permitted only where the reference emits output it cannot itself read
back, where matching would corrupt data or discard a write, or where matching would take the
host process down — and every divergence must be recorded in
[docs/compliance/jq/limitations.md](docs/compliance/jq/limitations.md) or
[docs/compliance/yq/limitations.md](docs/compliance/yq/limitations.md). A fork between
candidate behaviours is decided in that order, in both modes: first whether one of those
conditions lets us refuse the reference's behaviour, then which option matches the pinned
reference more closely, and only when neither separates them, which option performs better
or uses less memory (ADR-0018's #2416 amendment). Behavioural rules
therefore belong on `EvalSemantics` (per-mode), not on per-format traits like
`DocumentFields`.

Never state a jq/yq behaviour from memory — capture it from the pinned binary (`/usr/bin/jq`
1.7.1, Homebrew `yq` v4.53.3). That applies to "neither tool has this" as much as to any
other claim: `--front-matter`, `--split-exp` and cross-file evaluation all look like
succinctly extensions and are all real yq features (#715). Calling reference surface an
extension is the costly direction of that mistake, because extensions are exempt from the
divergence rule above. See [docs/adrs/adr-0018.md](docs/adrs/adr-0018.md).

### yq Merge-Flag Suffixes on `*`/`*=` (yq mode only)

Real yq extends `*`/`*=` with combinable flag suffixes that control merge semantics. They go directly after `*` for the plain (non-assign) form, or after `*=` for the in-place form — never between `*` and `=` (`.a *+= .b` is not valid; the flags belong after the `=`):

| Flag | Meaning                                                                                    |
|------|--------------------------------------------------------------------------------------------|
| `+`  | Append arrays instead of replacing them                                                    |
| `?`  | Only update fields/indices that already exist; never create new                            |
| `n`  | Only write fields/indices that don't already exist (or are `null`)                         |
| `d`  | Deep-merge arrays: treat them like objects, merging by index                               |
| `c`  | Clobber custom tags (parsed but a no-op today — no tag data exists to preserve or clobber) |

Flags combine freely, in any order (`*+d` and `*d+` are identical):

```bash
printf 'a: [1, 2]\nb: [3, 4]\n' | succinctly yq '.a *=+ .b'   # a: [1, 2, 3, 4]  (append)
printf 'a:\n  x: 1\nb:\n  x: 2\n  y: 3\n' | succinctly yq '.a *=? .b'  # a: {x: 2}        (only-existing)
printf 'a:\n  x: 1\nb:\n  x: 2\n  y: 3\n' | succinctly yq '.a *=n .b'  # a: {x: 1, y: 3}  (only-new)
```

`?`/`n` propagate through every nesting depth: a parent key that already exists still gets recursed into so its own new children can be added or blocked individually — the gate never blocks recursion into a matching nested object/array itself, only the leaf writes within it. Combining `?` and `n` is an AND of both gates (net effect: only touch a field that already exists and is currently `null`).

`+`/`d` combined on the same array is a documented simplification: real yq has a surprising, untested-upstream double-effect here; succinctly makes `+` take clean priority (pure append, `d` ignored) instead.

`null` acts as an empty container on either side of a yq-mode merge (jq mode has no such exception -- every `null`-involving `*` pairing errors there, #1175): a null/absent *left* operand merges as if starting from `{}`/`[]` (`.a *=n .b` on `a: null` writes the full `.b` in; `.a *=? .b` on an absent `.a` leaves `a: {}`, blocked field-by-field rather than staying `null`), and a null *right* operand is always a no-op (`.a *= null` leaves `.a` untouched).

### `sub` divergence (yq mode only) — `gsub`/`scan`/`splits` aren't real yq builtins at all

Real yq's bare, 2-arg `sub(re; s)` replaces *every* match, not just the first — jq's `sub` = first match only, `gsub` = all matches; yq's bare `sub` behaves like jq's `gsub` unconditionally (`"aaa" | sub("a";"X")` => `"XXX"` in yq, `"Xaa"` in jq, confirmed against yq v4.53.3). `gsub` is not a real yq builtin at any arity — its lexer rejects `gsub(...)` outright, same as `scan`/`splits` (confirmed live against yq v4.53.3, #1436) — so there's no jq-model "match" to speak of for succinctly's own `gsub` in yq mode; it's a succinctly extension (ADR-0018 rule 5), gated behind `--jq-extensions` and off by default since #1512, not a verified divergence.

Real yq's 3-arg `sub(re; replacement; flags)` never evaluates `replacement` or `flags` at all — it always performs a global replace-with-empty-string using only the pattern (near-certainly an upstream Go bug reading its replacement from a fixed AST slot that's empty once arity exceeds 2, not a designed feature; confirmed live that an `error(...)` in either position never fires). Per ADR-0018 rule 3, succinctly reproduces this bug-for-bug rather than "fixing" it into jq's model (`"aaa" | sub("a";"X";"g")` => `""` in both yq and succinctly yq mode; flags like `"i"` are silently ignored, and a 4th+ argument is accepted and discarded, matching real yq's own parser leniency — #1122).

### Anchor/alias preservation and its soundness rule (yq mode only)

YAML-target output re-emits `&anchor`/`*alias` on both the cursor-streaming path and the
DOM path taken by a write (`=`, `|=`, `+=`, `del()`) or a DOM-forcing flag (`-P`,
`--arg`) — ADR-0017's mechanism 2, #763. `-o=json` still expands aliases, matching real
yq. A node's mark rides `NodeMeta` in the `CommentTree` side-tree alongside its comment
and style; `enforce_anchor_soundness` (`yq_runner.rs`) then drops any `*name` the
document cannot resolve.

Two routes carry no `CommentTree` at all and so preserve nothing — not anchors, not
comments, not style. Both predate #763 and neither is specific to anchors: a filter
yielding *multiple* results (anything with a comma) loses its cursor to
`GenericResult::Many` before `evaluate_yaml_cursor` can capture one, and `--inplace`
never builds a tree in the first place (#1349).

**succinctly never emits YAML it cannot read back — real yq does.** A `*name` is written
only when a matching `&name` exists, is emitted *earlier*, and holds an *equal* value.
Real yq fails all three in ordinary use (both verified against v4.53.3):

```bash
printf 'a: &x 1\nb: *x\n' | yq 'del(.a)'          # b: *x   <- no &x anywhere; yq then
                                                  #            rejects its own output with
                                                  #            "unknown anchor 'x' referenced"
printf 'a: &x 1\nb: *x\n' | succinctly yq 'del(.a)'   # b: 1
```

Real yq's `sort_keys` diverges the same way when sorting would move an alias above its
declaration — and succinctly's own `--sort-keys` currently *reproduces* that unsound output,
because the soundness pass doesn't reach the streaming path (#1350). That is a bug against
the rule, not a second exception to it, so the "never" above is the intent and #1350 is the
one place it does not hold. The equal-value rule also contains a real gap rather than
papering over it:
succinctly's alias sync is one-directional (anchor → aliases), so a write *through* an
alias (`.b.p = 9`) updates only `.b`, where yq mutates the shared node. Emitting `b: *x`
there would silently discard the write, so the mark is dropped and the computed value
printed instead. True alias node identity remains unimplemented.

### jq Position-Based Navigation (succinctly extension)

Succinctly extends jq with position-based navigation builtins that allow jumping directly to a node at a specific byte offset or line/column position. This is unique to succinctly and not available in standard jq or yq.

| Builtin                    | Description                                                  | Example             |
|----------------------------|--------------------------------------------------------------|---------------------|
| **at_offset(n)**           | Jump to node at byte offset `n` (0-indexed)                  | `at_offset(10)`     |
| **at_position(line; col)** | Jump to node at line/column (1-indexed, semicolon-separated) | `at_position(2; 3)` |

**Use cases:**
- IDE integration: Jump to node under cursor position
- Error investigation: Navigate to specific byte offset from error message
- Programmatic navigation: Build tools that work with document positions

```bash
# Jump to node at byte offset 10 (inside "Alice" string)
echo '{"name": "Alice", "age": 30}' | succinctly jq 'at_offset(10)'
# Output: "Alice"

# Jump to node at line 1, column 1 (the root object)
echo '{"name": "Alice"}' | succinctly jq 'at_position(1; 1)'
# Output: {"name": "Alice"}

# Navigate from offset position - get array at offset 10, then access element
echo '{"users": [{"name": "Alice"}, {"name": "Bob"}]}' | succinctly jq 'at_offset(10) | .[1].name'
# Output: "Bob"

# Works with multiline JSON - line 2, column 3 is the "name" key
echo '{
  "name": "Alice",
  "age": 30
}' | succinctly jq 'at_position(2; 3)'
# Output: "name"

# Works with YAML via yq
succinctly yq 'at_offset(6)' config.yaml

# Combine with other jq operations
echo '{"data": {"nested": {"value": 42}}}' | succinctly jq 'at_offset(9) | .nested.value'
# Output: 42
```

**Notes:**
- `at_offset(n)` uses 0-indexed byte offset (same as `jq-locate --offset`)
- `at_position(line; col)` uses 1-indexed line and column (same as `jq-locate --line --column`)
- Returns error if offset/position is out of bounds or doesn't correspond to a valid node
- Use `at_offset(n)?` or `at_position(l; c)?` for optional (returns empty on invalid position)

### jq `leaf_paths` (succinctly extension)

`leaf_paths` is **not a real jq builtin** — it errors with `leaf_paths/0 is
not defined` in real jq (confirmed against jq 1.7.1 and 1.8.2). It's a
succinctly extension modeled on the community recipe
`def leaf_paths: paths(scalars);`, but intentionally broader: it returns
paths to every **leaf** (childless) node in the tree, not just paths whose
`type` is scalar.

```bash
echo '{"a": {"b": 1, "c": null}, "d": [], "e": [2, 3]}' | succinctly jq -c 'leaf_paths'
# ["a","b"]
# ["a","c"]
# ["d"]
# ["e",0]
# ["e",1]
```

**Diverges from `paths(scalars)` on two cases:**
- `null` counts as a leaf here; `paths(scalars)` excludes it (an accidental
  side effect of `select()` treating a `null` yield as falsy, not a
  deliberate design choice in jq).
- Empty `{}`/`[]` count as leaves here; `paths(scalars)` excludes them too,
  since their `type` is `"object"`/`"array"`, not scalar — even though they
  have no children to recurse into either.

This divergence is intentional: `leaf_paths` uses a tree-structural
definition of "leaf" (no children), which the community recipe's
type-based `scalars` filter doesn't fully capture. See
[collect_leaf_paths](src/jq/eval.rs) and issue #771 for the full rationale.

**In `succinctly yq`**, `leaf_paths` (along with `paths`, `getpath`, `limit`,
`gsub`/`scan`/`splits`, and the rest of the jq-only surface real yq's lexer
rejects) is gated behind `--jq-extensions` and off by default since #1512 —
`succinctly jq` above is unaffected either way. See
[docs/reference/yq-language.md](docs/reference/yq-language.md#gated-jq-builtins---jq-extensions)
for the full list.

## Feature Flags

| Feature             | Description                               |
|---------------------|-------------------------------------------|
| `std`               | Standard library (default, CPU detection) |
| `simd`              | Explicit SIMD intrinsics                  |
| `portable-popcount` | Portable bitwise popcount                 |
| `serde`             | Serde serialize/deserialize support       |
| `regex`             | Regex builtins in jq (included in `cli`)  |
| `cli`               | CLI tool (jq, yq, locate, generators)     |
| `bench-runner`      | Unified benchmark runner (bench list/run) |
| `large-tests`       | 1GB bitvector tests                       |
| `huge-tests`        | 5GB bitvector tests                       |
| `mmap-tests`        | Memory-mapped bitvector tests             |
| `broadword-yaml`    | Portable broadword (SWAR) YAML on ARM64   |
| `scalar-yaml`       | Pure scalar YAML parsing (no SIMD)        |

## Testing Strategy

- **Unit tests**: In each module's `#[cfg(test)] mod tests`
- **Property tests**: `tests/property_tests.rs`, `tests/bp_properties.rs`
- **Integration tests**: `tests/json_indexing_tests.rs`, `tests/simd_level_tests.rs`

## `no_std` Support

The library is `no_std` compatible:
- Uses `#![cfg_attr(not(test), no_std)]`
- Depends on `alloc` for `Vec<u64>` storage

## CI/CD

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo test
./scripts/build.sh
```

## Key Documentation

| Document                                                          | Purpose                              |
|-------------------------------------------------------------------|--------------------------------------|
| [ARCHITECTURE.md](ARCHITECTURE.md)                                | Top-level architecture entry point   |
| [docs/STYLE_GUIDE.md](docs/STYLE_GUIDE.md)                        | Tagged, stable-ID coding conventions |
| [docs/adrs/README.md](docs/adrs/README.md)                        | Architecture Decision Records (why)  |
| [docs/guides/release.md](docs/guides/release.md)                  | Release process and checklist        |
| [docs/optimizations/](docs/optimizations/)                        | Optimization techniques reference    |
| [docs/benchmarks/jq.md](docs/benchmarks/jq.md)                    | JSON jq benchmark results            |
| [docs/benchmarks/yq.md](docs/benchmarks/yq.md)                    | YAML yq benchmark results            |
| [docs/benchmarks/dsv.md](docs/benchmarks/dsv.md)                  | DSV input performance benchmarks     |
| [docs/adrs/adr-0011.md](docs/adrs/adr-0011.md)                    | Why rank/select is built in-crate    |

## Performance Summary

### jq Query Performance (Apple M1 Max)

| Size      | succinctly            | jq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  5.7 ms  (1.7 MiB/s)  |  6.0 ms  (1.6 MiB/s)  | **1.0x**   |
| **100KB** |  8.2 ms (11.9 MiB/s)  | 12.0 ms  (8.1 MiB/s)  | **1.5x**   |
| **1MB**   | 38.9 ms (25.7 MiB/s)  | 70.9 ms (14.1 MiB/s)  | **1.8x**   |

### jq Query Performance (Apple M4 Pro)

| Size      | succinctly            | jq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  3.3 ms  (3.0 MiB/s)  |  3.3 ms  (3.0 MiB/s)  | **1.0x**   |
| **100KB** |  4.1 ms (24.4 MiB/s)  |  5.7 ms (17.5 MiB/s)  | **1.4x**   |
| **1MB**   | 13.0 ms (76.9 MiB/s)  | 32.0 ms (31.3 MiB/s)  | **2.5x**   |

To regenerate: `succinctly bench run jq_bench`

### jq Query Performance (ARM Neoverse-V2)

| Size      | succinctly            | jq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  1.8 ms (5.3 MiB/s)   |  2.6 ms  (3.6 MiB/s)  | **1.4x**   |
| **100KB** |  4.3 ms (21.1 MiB/s)  |  6.3 ms (14.4 MiB/s)  | **1.5x**   |
| **1MB**   | 28.0 ms (28.9 MiB/s)  | 46.1 ms (17.5 MiB/s)  | **1.6x**   |

### jq Query Performance (ARM Neoverse-V1)

| Size      | succinctly            | jq                    | Speedup    |
|-----------|-----------------------|-----------------------|------------|
| **10KB**  |  1.4 ms  (6.7 MiB/s)  |  2.6 ms  (3.6 MiB/s)  | **1.8x**   |
| **100KB** |  4.0 ms (21.5 MiB/s)  |  6.4 ms (13.3 MiB/s)  | **1.6x**   |
| **1MB**   | 27.9 ms (29.0 MiB/s)  | 45.6 ms (17.7 MiB/s)  | **1.6x**   |

### yq Query Performance (Apple M1 Max)

| Size       | succinctly             | yq                     | Speedup     | Mem Ratio  |
|------------|------------------------|------------------------|-------------|------------|
| **10KB**   |   5.5 ms  (1.8 MiB/s)  |   9.1 ms  (1.1 MiB/s)  | **1.7x**    | **0.51x**  |
| **100KB**  |   6.1 ms (16.0 MiB/s)  |  19.5 ms  (5.0 MiB/s)  | **3.2x**    | **0.33x**  |
| **1MB**    |  18.6 ms (53.8 MiB/s)  | 113.7 ms  (8.8 MiB/s)  | **6.1x**    | **0.12x**  |
| **10MB**   | 119.8 ms (83.5 MiB/s)  |   1.02 s  (9.8 MiB/s)  | **8.5x**    | **0.05x**  |
| **100MB**  |   1.09 s (91.7 MiB/s)  |   9.74 s (10.3 MiB/s)  | **8.9x**    | **0.04x**  |

### yq Query Performance (Apple M4 Pro)

| Size       | succinctly             | yq                     | Speedup     | Mem Ratio  |
|------------|------------------------|------------------------|-------------|------------|
| **10KB**   |   3.5 ms  (2.9 MiB/s)  |   6.5 ms  (1.5 MiB/s)  | **1.9x**    | **0.34x**  |
| **100KB**  |   4.6 ms (21.7 MiB/s)  |  16.0 ms  (6.3 MiB/s)  | **3.5x**    | **0.21x**  |
| **1MB**    |  13.2 ms (75.8 MiB/s)  |  96.7 ms (10.3 MiB/s)  | **7.3x**    | **0.08x**  |
| **10MB**   |  97.1 ms (103.0 MiB/s) | 888.4 ms (11.3 MiB/s)  | **9.2x**    | **0.04x**  |
| **100MB**  | 908.9 ms (110.0 MiB/s) |   9.68 s (10.3 MiB/s)  | **10.6x**   | **0.04x**  |

### yq Query Performance (ARM Neoverse-V2)

| Size      | succinctly              | yq                    | Speedup    |
|-----------|-------------------------|-----------------------|------------|
| **10KB**  | 1.8 ms  (5.4 MiB/s)     | 5.1 ms (1.9 MiB/s)    | **2.9x**   |
| **100KB** | 3.1 ms (29.7 MiB/s)     | 20.0 ms (4.6 MiB/s)   | **6.5x**   |
| **1MB**   | 14.7 ms (62.6 MiB/s)    | 152.2 ms (6.1 MiB/s)  | **10.3x**  |

### yq Query Performance (ARM Neoverse-V1)

Note: System `yq` not installed; showing succinctly-only performance.

| Size      | succinctly              |
|-----------|-------------------------|
| **10KB**  | 1.27 ms  (7.7 MiB/s)    |
| **100KB** | 2.87 ms (32.0 MiB/s)    |
| **1MB**   | 17.7 ms (52.1 MiB/s)    |

### yq Query Performance (AMD Ryzen 9 7950X)

| Size      | succinctly              | yq                    | Speedup    |
|-----------|-------------------------|-----------------------|------------|
| **10KB**  | 2.7 ms   (3.6 MiB/s)    | 62.9 ms (156 KiB/s)   | **23x**    |
| **100KB** | 3.7 ms  (26.3 MiB/s)    | 78.2 ms (1.2 MiB/s)   | **21x**    |
| **1MB**   | 13.8 ms (72.5 MiB/s)    |203.3 ms (4.9 MiB/s)   | **15x**    |

To regenerate: `succinctly bench run yq_bench` (includes memory) or `cargo bench --bench yq_comparison` (time only)

### M2 Streaming Navigation Performance (Apple M1 Max, 100MB navigation file)

| Query       | Path          | succinctly | yq       | Speedup     | succ Mem | yq Mem  |
|-------------|---------------|------------|----------|-------------|----------|---------|
| `.`         | P9 streaming  | 1.21s      | 11.31s   | **9.4x**    | 275 MB   | 7 GB    |
| `.[0]`      | M2 streaming  | 444ms      | 5.84s    | **13.2x**   | 254 MB   | 5 GB    |
| `.[]`       | M2 streaming  | 2.53s      | 13.30s   | **5.3x**    | 269 MB   | 8 GB    |
| `length`    | OwnedValue    | 445ms      | 5.84s    | **13.1x**   | 254 MB   | 5 GB    |

M2 streaming (`.[0]`) is **2.7x faster** than identity (`.`), with **3-4% of yq's memory**.

To benchmark: `succinctly dev bench yq --queries all` (memory collected by default)

### Optimization Techniques

For detailed documentation on optimization techniques used in this project, see [docs/optimizations/](docs/optimizations/):

| Category | Document | Key Techniques |
|----------|----------|----------------|
| Bit-level | [bit-manipulation.md](docs/optimizations/bit-manipulation.md) | Popcount, CTZ, PDEP/PEXT |
| SIMD | [simd.md](docs/optimizations/simd.md) | AVX2, AVX-512, NEON |
| Memory | [cache-memory.md](docs/optimizations/cache-memory.md) | Alignment, prefetching |
| Data structures | [hierarchical-structures.md](docs/optimizations/hierarchical-structures.md) | Rank/select indices |
| Parsing | [state-machines.md](docs/optimizations/state-machines.md) | PFSM, lookup tables |

**Key insights** (see [docs/optimizations/README.md](docs/optimizations/README.md) for full details):
- Wider SIMD != automatically faster (AVX-512 JSON was 10% slower than AVX2)
- Algorithmic improvements beat micro-optimizations (cumulative index: 627x speedup; YAML streaming: 2.3x speedup)
- Simpler data structures often outperform complex ones due to cache behaviour
- Caching hot values eliminates repeated lookups (type checking: 1-17% improvement)
- Hardware prefetchers beat software prefetch for sequential access (prefetch: +30% regression!)
- SIMD newline scanning + indentation checking enables fast block boundary detection (block scalars: 19-25% improvement!)
- Micro-benchmark wins ≠ real-world improvements (threshold tuning: +8-15% regression despite micro-bench suggesting improvement)
- Eliminating phases beats optimizing them (YAML streaming: removed DOM conversion entirely for 2.3x gain)
- Derive, don't store, what the text already encodes (seq_items bitvector elimination: −12.5% build peak memory)
- Duplicated predicates diverge silently — one definition, plus a test that the call sites agree (#106: three copies of one predicate, one of them quadratic)
- Check where data physically lives before proposing to tag it (#106: the proposal assumed a `Vec<u32>`; no real file has one)
- Re-derive a break-even before trusting it (#106: the issue's stated 12.5% dropped a bits-to-bytes conversion; the real gate was 3.125%)
- A hash table is not automatically cheaper than a sort — above L3 a sort streams and a table does not, and which wins is architecture-dependent (#1514: the same table beat the sort on an M4 Pro and lost 24% to it on a 7950X at 7.1M keys)
- To attribute a cost, build the binary again with the feature *disabled* and measure that — timings alone cannot separate what a check costs from what the code around it costs (#1514: it proved #1385's cost was 100% its probe, and caught a larger regression introduced by the fix)

### Benchmarking Discipline

**Read [docs/guides/benchmarking.md § A/B Benchmarking Method](docs/guides/benchmarking.md#ab-benchmarking-method) before measuring a before/after change.** The naive method reported a 16x win as a regression (#106). The rules that matter most:

- **Interleave the two binaries within each repetition.** Running all of A then all of B made an improved binary measure up to **2x slower** on every workload — the second half starts thermally loaded. Not fixable with more reps; min and median can agree with each other and both be wrong.
- **Process-spawn A/B needs inputs ≥ 1 MB.** Startup is ~4-6 ms, the entire runtime at 1kb/10kb/100kb — which are in `dev bench yq`'s defaults.
- **Report the scaling curve over 2-3 sizes**, not one ratio. Growth with input size is the signature of an algorithmic fix and corroborates the claimed mechanism.
- **Gate on output identity before believing any timing**, and confirm both still match `jq`/`yq`.
- **Memory-bound effects do not port across architectures** — measure ARM *and* x86_64, and name the chip in every table. #106 was 6.1x on M4 Pro, 16.4x on Zen 4.
- **Verify the box is idle; macOS load average lies** (read 1.4 on a machine using 2.4% CPU — it counts uninterruptible-wait threads). Never benchmark a laptop on battery.
- **A benchmark cannot measure a shape it does not generate** — "all neutral" is not evidence. Add the generator pattern first.
- **A share of runtime is not a comparison.** Name a baseline commit that predates the change being judged — not a sibling from the same series (#1514: "cut the probe from 10.4% to 3.9%" shipped inside a 2-3x regression).
- **Measure a precheck where the guarded code is already fastest.** It is charged to every input including the ones it cannot help, so the workload with least work to save is where its cost shows (#1514: the same detector read +14% on record-shaped input and +141% on one wide object).
- **Attribute the cost before you A/B it.** A perf issue's named culprit is a hypothesis — #1213 and #1301 both named the wrong one. When the issue blames quantity A and you suspect B, build inputs holding A *provably constant* while varying B; if the time moves, A is not the term. #1301 blamed the resolved-path count, so holding it at 80,000 while widening the fan-out from 2 to 512 branches settled it in one table (890ms → 95ms). Unlike a profile, which only samples the window it runs in, that table has no escape hatch — and fitting its model predicts the post-fix floor before you change a line.

**Recent YAML optimizations:**
- ✅ P2.5 (Cached Type Checking): 1-17% improvement depending on nesting depth
  - See [docs/parsing/yaml.md#p25-cached-type-checking---implemented-](docs/parsing/yaml.md#p25-cached-type-checking---implemented-) for details
  - Best for deeply nested YAML (Kubernetes configs, CI/CD files)
- ❌ P2.6 (Software Prefetching): **REJECTED** - 30% regression on large files
  - Modern CPU hardware prefetchers are superior for sequential parsing
  - Software prefetch causes cache pollution and interferes with hardware
  - See [docs/parsing/yaml.md#p26-software-prefetching-for-large-files---rejected-](docs/parsing/yaml.md#p26-software-prefetching-for-large-files---rejected-) for analysis
- ✅ P2.7 (Block Scalar SIMD): **19-25% improvement** on block scalar parsing - **largest Phase 2 optimization!**
  - AVX2 scans 32-byte chunks for newlines, checks indentation using SIMD
  - 100x100 lines: 247 µs → 195 µs (-21%, 1.27x faster)
  - Throughput: 1.39 GiB/s → 1.76 GiB/s (+27%)
  - See [docs/parsing/yaml.md#p27-block-scalar-simd---accepted-](docs/parsing/yaml.md#p27-block-scalar-simd---accepted-) for full analysis
- ❌ P2.8 (SIMD Threshold Tuning): **REJECTED** - 8-15% regression on quoted strings
  - Micro-benchmarks suggested SIMD threshold tuning would help (showed 2-4% gain)
  - End-to-end tests showed severe regressions instead
  - Modern CPUs (branch prediction, inlining) make threshold tuning counterproductive
  - See [docs/parsing/yaml.md#p28-simd-threshold-tuning---rejected-](docs/parsing/yaml.md#p28-simd-threshold-tuning---rejected-) for full analysis
- ❌ P3 (Branchless Character Classification): **REJECTED** - 25-44% regression on simple parsing
  - Replaced `matches!` macros with lookup tables to eliminate branches
  - Micro-benchmarks showed 3-29% improvement for character classification
  - End-to-end tests showed catastrophic 25-44% regressions
  - Modern branch predictors (93-95% accuracy) beat lookup tables
  - `.map_or()` overhead and cache pollution from 256-byte tables dominated any benefit
  - Third consecutive optimization where micro-benchmarks mislead (P2.6, P2.8, P3 all failed)
  - See [docs/parsing/yaml.md#p3-branchless-character-classification---rejected-](docs/parsing/yaml.md#p3-branchless-character-classification---rejected-) for full analysis
- ✅ P4 (Anchor/Alias SIMD): **6-17% improvement** on anchor-heavy workloads
  - AVX2 SIMD scans for anchor name terminators in 32-byte chunks
  - anchors/100: 13.60µs → 11.65µs (-14.6%, 1.17x faster)
  - anchors/1000: 155.5µs → 139.2µs (-10.1%, 1.11x faster)
  - k8s_100: 33.92µs → 31.40µs (-7.4%, 1.08x faster)
  - Micro-benchmarks confirmed: 9-12x faster for 32-64 byte anchor names
  - **First successful optimization since P2.7** - proves SIMD string searching wins when targeting real bottlenecks
  - Best for: Kubernetes manifests, CI/CD configs with many anchors/aliases
  - See [docs/parsing/yaml.md#p4-anchoralias-simd---accepted-](docs/parsing/yaml.md#p4-anchoralias-simd---accepted-) for full analysis
- ❌ P5 (Flow Collection Fast Path): **REJECTED** - aborted at analysis stage before implementation
  - Micro-benchmarks showed 8-14x SIMD wins for 64-128 byte flow collections
  - Real YAML flow collections are typically 10-30 bytes (too small for SIMD to win)
  - SIMD only wins at 32+ bytes, but 90% of real flow collections are < 30 bytes
  - **Lessons from P2.6/P2.8/P3 applied** - rejected during analysis to avoid predicted regression
  - Key insight: Micro-benchmark wins only translate when optimizing inputs that exist in real workloads
  - See [docs/parsing/yaml.md#p5-flow-collection-fast-path---rejected-](docs/parsing/yaml.md#p5-flow-collection-fast-path---rejected-) for full analysis
- ❌ P6 (BMI2 Operations): **REJECTED** - YAML's grammar prevents DSV-style quote indexing
  - **Primary reason**: YAML cannot use BMI2 like DSV does (grammar incompatibility)
    - DSV: Quotes are context-free (`""` escape), can build global quote index in one pass using BMI2 `toggle64`
    - YAML: Backslash escaping (`\"`) requires escape preprocessing BEFORE quote detection
    - Circular dependency: Need escape mask to build quote mask, but escape handling is what we're optimizing
    - Multi-pass approach (escape → quote → parse) slower than current early-exit SIMD
  - **Wrong approach tested**: Micro-benchmarks tested per-string parsing (not index building)
    - DSV BMI2 builds global document index (scans everything once)
    - Tested YAML per-string parsing (early-exit SIMD wins on short strings)
    - Apples-to-oranges comparison revealed fundamental grammar difference
  - **Key insight**: BMI2 quote indexing only works when escaping is context-free (CSV's `""`, not YAML's `\"`)
  - **Additional context-sensitivity**: YAML has two quote types (`"` vs `'`), block scalars (`|` `>`), and context-dependent quote meaning
  - See [docs/parsing/yaml.md#p6-bmi2-operations-pdeppext---rejected-](docs/parsing/yaml.md#p6-bmi2-operations-pdeppext---rejected-) for full analysis
- ❌ P7 (Newline Index): **REJECTED** - use case mismatch (CLI feature, not parsing optimization)
  - ⚠️ **Premise corrected by #228**: JSON had *two* newline structures and P7 found only one.
    `JsonIndex::newlines` **was** built eagerly in every `JsonIndex::build` — an O(n) scan and ~15.9%
    of the input on every jq query. P7's conclusion held; JSON was violating it. Both are now
    `text::LineIndex`, built lazily (ADR-0012).
  - **Discovery** (as believed at the time): JSON's `NewlineIndex` is NOT built during parsing
    - Only used in `jq-locate` CLI tool for `--line X --column Y` → byte offset conversion
    - Never built during JSON parsing, jq queries, or benchmarks
    - Zero performance impact on actual JSON processing
  - **YAML's current approach**: Lazy O(n) `current_line()` only on error paths
    - 4 call sites, all in error reporting (TabIndentation, UnexpectedToken, etc.)
    - Comment: "Only called on error paths, so we pay the cost only when needed"
    - Benchmarks use valid YAML (no errors, so `current_line()` never called)
  - **Two possible interpretations**:
    - Option A (CLI feature): Add `yq-locate` tool with on-demand NewlineIndex (no benchmark impact)
    - Option B (misguided): Build index during parsing (O(n) overhead with zero benefit for valid YAML)
  - **Why Option B would fail**: Adds build cost to hot path (parsing) to optimize cold path (errors/CLI)
  - **Pattern recognition**: Not all JSON features are optimizations - NewlineIndex is CLI UX, not performance
  - See [docs/parsing/yaml.md#p7-newline-index---rejected-](docs/parsing/yaml.md#p7-newline-index---rejected-) for full analysis
- ❌ P8 (AVX-512 Variants): **REJECTED** - benchmark design flaw + JSON precedent (7% slower at realistic sizes)
  - **Primary reason**: Memory-bound workload doesn't benefit from wider SIMD
    - JSON AVX-512 was **7-17% slower** than AVX2 and removed from codebase
    - YAML is similarly memory-bound (sequential text parsing)
    - Memory bandwidth bottleneck prevents wider vectors from helping
  - **Micro-benchmark design flaw**:
    - Measured loop iterations instead of work performed
    - AVX2: 8 iterations for 256B (32B chunks)
    - AVX-512: 4 iterations for 256B (64B chunks)
    - "2x speedup" was artificial - half the iterations, not twice the efficiency
  - **Realistic size results** (AMD Ryzen 9 7950X):
    - 64B: **7% slower** (18.59ns vs 17.34ns) - memory bandwidth bottleneck
    - 128B: **Neutral** (18.94ns vs 18.95ns) - break-even point
    - 256B+: "Wins" misleading (fewer iterations, not faster per-byte)
  - **Zen 4 limitation**: Splits 512-bit ops into two 256-bit paths (overhead without benefit)
  - **Pattern recognition**: P5/P6/P7/P8 all rejected for mismatch (size/grammar/use case/benchmark)
  - **Key lesson**: Wider SIMD ≠ faster for memory-bound workloads (AVX2 already saturates RAM bandwidth)
  - See [docs/parsing/yaml.md#p8-avx-512-variants---rejected-](docs/parsing/yaml.md#p8-avx-512-variants---rejected-) for full analysis
- ✅ P9 (Direct YAML-to-JSON Streaming): **2.3x improvement** on `yq` identity queries - **largest Phase 2 optimization!**
  - Eliminated intermediate OwnedValue DOM by streaming directly from YAML cursor to JSON
  - 10KB: 257µs → 108µs (38.1 → 90.5 MiB/s, **+137%**)
  - 100KB: 1.93ms → 828µs (47.7 → 111.1 MiB/s, **+133%**)
  - Single-pass YAML→JSON escape transcoding without intermediate string allocation
  - **Bottleneck shift**: Parsing now 40% of time (was 15%), DOM conversion eliminated
  - Best for: `yq '.'` identity queries, format conversion, streaming output
  - See [docs/parsing/yaml.md#p9-direct-yaml-to-json-streaming---accepted-](docs/parsing/yaml.md#p9-direct-yaml-to-json-streaming---accepted-) for full analysis
- ✅ P10 (Type Preservation): **Full yq compatibility** (correctness fix)
  - Fixed quoted string type preservation: `"1.0"` stays as string, not converted to number `1`
  - Added early-exit for quoted strings, skipping expensive `parse::<i64>()` and `parse::<f64>()` calls
  - Current performance: 10KB: 1.63ms, 100KB: 2.78ms, 1MB: 13.2ms (with correct output)
  - Byte-for-byte output verified against pinned-`yq` golden fixtures (`tests/yq_golden_tests.rs`, captured from mikefarah/yq via `scripts/sync-yq-golden.sh`) that run hermetically on every CI leg, plus a `yq-drift` CI job that re-checks the goldens against the pinned `yq` — see #227
  - **Key achievement**: `succinctly yq` is now a drop-in replacement for `yq` for supported arguments
  - See [docs/parsing/yaml.md#p10-type-preservation-for-yq-compatibility---accepted-](docs/parsing/yaml.md#p10-type-preservation-for-yq-compatibility---accepted-) for full analysis
- ✅ P11 (BP Select1 for yq-locate): **2.5-5.9x faster** select1 queries, fixes issue #26
  - Added zero-cost generic `SelectSupport` trait to `BalancedParens<W, S>` (NoSelect for JSON, WithSelect for YAML)
  - `find_bp_at_text_pos()` now uses O(1) sampled select1 instead of O(log n) binary search on rank1
  - **Micro-benchmark speedups** (10K queries):
    - 1K opens: 326µs vs 820µs (**2.5x** faster)
    - 10K opens: 318µs vs 1.31ms (**4.1x** faster)
    - 100K opens: 308µs vs 1.68ms (**5.4x** faster)
    - 1M opens: 356µs vs 2.10ms (**5.9x** faster)
  - **End-to-end yq benchmarks**: 1MB identity query 3.1% faster (14.45ms → 14.00ms)
  - **Trade-off**: 2-4% regression in yaml_bench (SelectIndex build cost) but benefits `yq-locate` use case
  - **Zero-cost for JSON**: Uses `NoSelect` (ZST) - no memory or runtime overhead
  - **Fixes GitHub issue #26**: YAML `at_offset` and `yq-locate` now return correct nodes
  - See [docs/parsing/yaml.md#p11-bp-select1-for-yq-locate---accepted-](docs/parsing/yaml.md#p11-bp-select1-for-yq-locate---accepted-) for full analysis
- ✅ P12 (Advance Index for bp_to_text): **20-25% faster** yq identity queries on 1MB files, fixes issue #62
  - Replaced `Vec<u32>` with memory-efficient `BpToTextPositions` enum (IB + Advance bitmaps)
  - **~1.5× memory compression** for bp_to_text structure (measured with 33% duplicates)
  - **Automatic fallback** to `Vec<u32>` for non-monotonic positions (explicit keys)
  - **End-to-end yq benchmarks**:
    - users/1mb: **-24.6%** time (+32.7% throughput)
    - sequences/1mb: **-24.8%** time (+33.0% throughput)
    - nested/1mb: **-21.5%** time (+27.4% throughput)
  - **yaml_bench improvements**: sequences/1000 -5.0%, large/1mb -3.2%, block_scalars -3.5%
  - **Minor regression**: ~1.5-2% on tiny (10-element) quoted string benchmarks
  - Better cache locality from compact bitmap representation
  - See [docs/parsing/yaml.md#p12-advance-index-for-memory-efficient-bp_to_text---accepted-](docs/parsing/yaml.md#p12-advance-index-for-memory-efficient-bp_to_text---accepted-) for full analysis
- ✅ P12-A (Build Regression Mitigation): **11-85% faster** `yaml_bench` build times, fixes issue #72
  - A1: Inline zero-filling in `EndPositions::build()` — eliminates temp `Vec<u32>` allocation
  - A2: Combined monotonicity check — merged into `try_build()`, eliminates separate O(N) scan
  - A4: Lazy newline index via `OnceCell` — removes O(N) text scan from `build()`
  - Largest gains on newline-heavy content (long strings: -44% to -85%, block scalars: -42% to -57%)
  - Broad improvements across all categories (simple_kv: -11% to -22%, nested: -15% to -24%)
  - A3 from issue #72 remains as future opportunity
  - See [docs/parsing/yaml.md#p12-a-build-regression-mitigation-a1--a2--a4---accepted-](docs/parsing/yaml.md#p12-a-build-regression-mitigation-a1--a2--a4---accepted-) for full analysis
- ✅ O1 (Sequential Cursor for AdvancePositions): **3-13% faster** yq queries on small-medium files, issue #74
  - Applied `Cell<SequentialCursor>` pattern from `CompactEndPositions` to `AdvancePositions`
  - Three-path dispatch: sequential (amortized O(1)), forward-gap (linear advance), random (full recomputation)
  - Duplicate-detection cache: `last_ib_arg`/`last_ib_result` for O(1) return on shared positions (~33% of nodes)
  - **End-to-end yq benchmarks** (users/ workload):
    - 1KB: **-9 to -13%** time (strongest improvement)
    - 10KB: **-8%** time
    - 100KB: **-3%** time
    - 1MB: neutral (get() is smaller fraction of total streaming time)
  - **Neutral on strings/ and nested/** — unique positions reduce duplicate-cache hit rate
  - Best for: small-medium YAML with container-heavy structure (Kubernetes manifests, CI/CD configs)
  - See [docs/parsing/yaml.md#o1-sequential-cursor-for-advancepositions--accepted-](docs/parsing/yaml.md#o1-sequential-cursor-for-advancepositions--accepted-) for full analysis
- ✅ O2 (Gap-Skipping via advance_rank1): **2-6% faster** yq queries on nested/users at small-medium sizes, issue #74
  - Replaced O(G) linear loop in `advance_cursor_to()` with O(1) `advance_rank1(target)` call
  - Applied to both `CompactEndPositions` and `AdvancePositions` cursor forward-gap paths
  - Reuses existing cumulative rank array — zero additional memory overhead
  - **End-to-end yq benchmarks** (against O1 baseline):
    - nested/1kb: **-6.1%**, users/10kb: **-6.0%**, strings/100kb: **-5.5%**
    - Most workloads show noisy but directionally positive results
  - **yaml_bench**: No regression (query-path only optimization)
  - Results are noisy because the forward-gap path is infrequently hit during typical streaming
  - See [docs/parsing/yaml.md#o2-gap-skipping-via-advance_rank1--accepted-](docs/parsing/yaml.md#o2-gap-skipping-via-advance_rank1--accepted-) for full analysis
- ✅ O3 (SIMD Escape Scanning): **4-12x faster** micro-benchmark escape scanning on ARM64 NEON, issue #87
  - Added `find_json_escape_neon()` using NEON SIMD to scan for JSON escape characters (`"`, `\`, `< 0x20`)
  - Processes 16 bytes per iteration vs 1 byte scalar
  - **Micro-benchmark speedups** (Apple M1 Max, no-escape strings):
    - 16B: **4.1x** faster, 64B: **6.3x** faster, 256B: **11.8x** faster, 1024B: **12x** faster
  - **Realistic patterns** (escape every ~20 chars): 1.3-2.5x speedup
  - **End-to-end transcode**: 180-420 MiB/s throughput maintained
  - **16-byte threshold + `#[inline(always)]`**: Both required to prevent regression (threshold alone still caused 3-5% regression)
  - Best for: YAML with long string values (logs, templates, embedded content)
  - See [docs/parsing/yaml.md#o3-simd-escape-scanning-for-json-output--accepted-](docs/parsing/yaml.md#o3-simd-escape-scanning-for-json-output--accepted-) for full analysis
- ✅ O4 (seq_items Bitvector Elimination): **−12.5% build peak memory, 2-6% faster builds**, issues #75/#104/#106
  - Removed the stored `seq_items` bitvector; seq-item wrappers are derived from text (`-` + whitespace/EOI)
  - Branchless `matches!` derivation at both detection sites recovers the 7-15% query regression #106 found in the naive form
  - **Measured** (Apple M5 Max, c090e7f6 vs 2a41c2f5): build peak 2.00× → 1.75× of input (−L/4 transient scratch), retained index −3-5% (19-41 KB/MB), all 44 yaml_bench benchmarks improved or neutral, yq query side neutral-or-better (worst +1.8% noise, best −10.7%)
  - Outputs byte-identical pre↔post on all 80 yq A/B configurations
  - Key insight: derive, don't store, what the text already encodes; transient build allocations can dwarf the retained structure (2-bit-per-byte scratch was 6-13× the stored bitvector)
  - See [docs/parsing/yaml.md#o4-seq_items-bitvector-elimination--accepted-](docs/parsing/yaml.md#o4-seq_items-bitvector-elimination--accepted-) for full analysis
- ✅ O5 (Lazy-Keys Cursor+Value Reuse): **-5.3% to -8.4%** on yq/jq queries that walk every key, issues #1599/#1606/#1609
  - `each_lazy_keys_iterate_sink`'s streaming `keys_unsorted` arm (#1599) discarded each key's already-decoded value and let downstream re-derive it via a second `V::Cursor::value()` resolve — for YAML a full scalar decode, not a cheap re-read
  - New `GenericItem::OneCursorValue(V::Cursor, V)` variant carries the value through instead; 3 edits in `src/jq/eval_generic.rs`, no changes to `document.rs`/`yaml/light.rs`/`json/light.rs`
  - **Measured** (Apple M4 Pro, interleaved A/B, 100K/1M-key flat mappings): walks-all shape -5.3% to -8.4% (both yq and jq), early-exit shape unchanged (within noise) — #1599's -34% early-exit win fully preserved, batching-window idea #1609 floated was not needed
  - `size_of::<GenericItem<V>>()` measured unchanged (176/192 bytes) — the new variant fit inside the enum's existing footprint, no boxing needed
  - Key insight: a doc comment naming a redundancy (#1514) can be verified by reading the call graph, not just profiled; format-dependent magnitude (YAML full decode vs JSON single-byte dispatch) doesn't mean the underlying inefficiency isn't present in the cheaper format too
  - See [docs/parsing/yaml.md#o5-lazy-keys-cursorvalue-reuse--accepted-](docs/parsing/yaml.md#o5-lazy-keys-cursorvalue-reuse--accepted-) for full analysis
- ✅ O6 (`HAS_CR` Const-Generic Specialization): **recovers most of the #324 CRLF-correctness cost**, issue #340
  - `build_semi_index` SIMD-scans once for `\r`, then parses with `Parser::<false>` (LF-only) or `Parser::<true>` (#324 parser verbatim)
  - Interleaved `yaml_bench` vs `c5dab403`, excl. block scalars: **M4 Pro +4.0% → +0.7%**, **7950X +11.0% → +4.7%**; x86 block scalars 31-34% *faster* than pre-#324
  - End-to-end `dev bench yq` (32 configs) recovers fully: **7950X +5.0% → +0.8%**, **M4 Pro +2.3% → −0.2%** median; CRLF documents +1.1%/+0.5% (unchanged); binary +52 KB (+0.9%)
  - **The gate is one-directional**: `true` is always correct, and the whole-input precheck proves no `\r` arm is reachable under `false`. An un-gated site is a missed optimization, never a bug — so gating can be applied incrementally and measured
  - **Cost the estimate missed**: long quoted scalars regress +7-12%. The parser bulk-skips them at ~15 GB/s, so a second pass over the input is large next to the parse. Reusing the position-finding `define_escape_scanner!` first made it **+42%**; an existence-only scan (OR the chunk compares, reduce once) was needed
  - Key insight: a precheck that enables a fast path is charged to *every* input, including the ones it cannot help — measure it against the workload where the parser is already fastest, not the one it is slowest
  - See [docs/parsing/yaml.md#buying-it-back-the-has_cr-specialization](docs/parsing/yaml.md#buying-it-back-the-has_cr-specialization) for full analysis
- ✅ O7 (CS-Poppy Combined Sampling for BP Select): **4× smaller YAML select index**, mixed select1 speed by platform, issue #64
  - Replaced `WithSelect`'s sampled `(word, cumulative)` pairs with `WithCsPoppy`: `u32` entry points into BP's own rank directory (`rank_l1`/`rank_l2`) instead of a parallel structure
  - Step A (narrow `SelectIndex<u32>` for BP, `len <= u32::MAX` bits) + Step B (`WithCsPoppy`) together take the BP select index from 25% to 6.25% of the bitmap — a 4x reduction, measured via real `select_heap_size()`, not derived
  - **select1 speed is platform-dependent**: neutral-to-faster on Zen 4 (7950X), 5-15% slower on Apple M4 Pro — both measured via full (non-`--quick`) `bp_select_micro` runs on pinned hardware. Accepted despite the ARM regression because `BalancedParens::select1` on YAML's BP is reached only once per `at_offset`/`at_position`/`yq-locate` call, never the `.foo.bar` navigation hot path
  - `yaml_bench` build-side clean (< 2% on 38/39 groups, both platforms) except one x86-only `block_scalars` anomaly (12-28%, reproduced twice), investigated and attributed to binary code-layout rather than the new logic: the BP structure for that workload is a fixed 7 words regardless of document size, too small to mechanistically explain the delta, and the same workload is neutral on ARM — filed as #595
  - Also surfaced a pre-existing, unrelated `yaml_bench` bug: the `anchors` group panics with `UnknownAnchor` on unmodified `main` too — filed as #594
  - See [docs/plan/cspoppy.md](docs/plan/cspoppy.md#5a-results-2026-08-03) for full analysis

**UTF-8 validation optimizations:**
- ⚠️ Broadword (SWAR) UTF-8 accept scan: opt-in only, **not the default** — issue #134
  - Clears ASCII in 32- and 8-byte strides with plain 64-bit arithmetic, then validates each multi-byte sequence with independent range comparisons
  - **Corrected finding**: the original "2.01x/2.14x geomean vs scalar" claim was measured before #133 (which gave `validate_utf8_scalar` its own 8-byte ASCII skip) merged into `main`. This branch was rebased onto that merge with no conflict, so it went unnoticed — the doc numbers kept citing the pre-#133 scalar baseline. Against the *current* scalar validator, broadword's geomean is a **net loss**: ~0.89x on a 7950X, ~0.93x on an M4 Pro (interleaved, 9 runs, 11 realistic `generate-suite` patterns, 10MB, current `main` tip)
  - Wins clearly only on long/pure ASCII (~1.67x on 7950X, ~2.0x on M4 Pro); every other realistic pattern — source code, logs, JSON, accented text, CJK, emoji — is a wash-to-loss, some over 30% (e.g. source_code: 0.68-0.70x)
  - `validate_utf8()` therefore dispatches to AVX2 where available and to `validate_utf8_scalar` everywhere else (reverted from broadword); `validate_utf8_broadword` remains available for callers who know their input is ASCII-dominant
  - `validate_utf8_scalar` (from #133, not this issue) already beats `core::str::from_utf8` in geomean (~1.18-1.25x); broadword only adds a small further edge over std (~1.05-1.16x), far short of the originally claimed 2.01x/2.14x
  - Key insight: a wide ASCII-skip probe only pays for itself when runs are genuinely long; on realistic mixed content, attempting it is frequent, mostly-wasted overhead
  - See [docs/benchmarks/utf8-validate.md#engine-comparison-134](docs/benchmarks/utf8-validate.md#engine-comparison-134) for full analysis
- ❌ Höhrmann DFA for the multi-byte step: **REJECTED** — lost to whole-sequence validation on 9 of 11 realistic patterns, issue #134
  - Implemented as specified in #134 (9 states × 12 byte classes, packed-nibble transition table verified against Höhrmann's published `utf8d`), benchmarked head-to-head, and removed
  - Worst at emoji and mixed. The specific "x scalar" multipliers from that run predate #133's scalar ASCII skip and are not repeated here — but the relative finding (whole-sequence beats DFA) is unaffected, since both sides of that comparison shared the same denominator in the same run
  - **Cause is dependency structure, not table size**: a DFA carries `state -> step -> state` around its loop, retiring one byte per 2-3 cycle chain however compact the table; whole-sequence validation issues its range comparisons independently and retires 3-4 bytes per iteration
  - Same effect already visible in the baseline: the scalar validator was *faster* on emoji than on ASCII (pre-#133) precisely because it consumed a whole sequence per iteration
  - **Also**: #134's proposed 10-class byte table cannot work — lumping all of `0x80-0xBF` into one class makes a `(state, class)` table unable to distinguish `E0 80` (overlong) from `E0 A0` (valid), silently accepting overlong encodings and surrogates. 12 classes (split at `0x90`/`0xA0`) are the minimum.
  - Key lesson: for a validator, prefer the formulation with the shortest loop-carried dependency, not the smallest table
  - **Second lesson (from the broadword reversal above)**: a benchmark claim is only as fresh as its baseline — a clean, conflict-free rebase can silently improve the "control" side and flip a documented win into a loss with nothing to flag it
- ❌ Redundant-load removal in `broadword.rs`'s ASCII skip: **REJECTED** — 3.6x regression, issue #134
  - Hypothesis: the 32-byte block probe reloads its first 8 bytes a second time on failure (`load_word` re-reads what `load_block`'s first chunk already read), so probing narrow-first and widening only after the first word proves clean should save loads on short/isolated ASCII runs
  - Measured instead: ASCII throughput on an M-series Mac dropped from ~65 GiB/s to ~18 GiB/s — reproducible, not noise. Splitting one 4-word OR-reduction into "one word, then conditionally three more" likely defeated whatever auto-vectorization/unrolling LLVM was doing with the original single loop; load-count intuition does not predict codegen
  - Reverted immediately; needs real profiling (`perf`/disassembly), not another guess
