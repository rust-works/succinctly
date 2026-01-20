# yq Command Implementation Plan

This document outlines the plan to add a `yq` subcommand to succinctly for querying YAML files using jq-compatible syntax.

## Implementation Status

| Phase | Goal | Status |
|-------|------|--------|
| **1** | Basic yq command | ✅ Complete |
| **2** | Full YAML 1.2 structural support | ✅ Complete |
| **3** | Anchors, aliases, YAML output | ✅ Mostly complete |
| **4** | Multi-document streams | ✅ Mostly complete |
| **5** | YAML-specific query extensions | 🔄 Partial |
| **6** | yq-specific operators | 🔜 In scope |
| **7** | Date/time operators | 🔜 In scope |
| **8** | Additional format encoders | 🔜 In scope |

### Performance (Apple M1 Max)

| Size | succinctly | system yq | Speedup |
|------|------------|-----------|---------|
| 10KB | 4.2 ms (2.3 MiB/s) | 8.4 ms (1.2 MiB/s) | **2.0x** |
| 100KB | 5.4 ms (17.1 MiB/s) | 20.3 ms (4.5 MiB/s) | **3.8x** |
| 1MB | 15.1 ms (61.0 MiB/s) | 118.5 ms (7.8 MiB/s) | **7.8x** |

## Overview

The `yq` command provides YAML querying capabilities using the same query language as `jq`. It leverages:
- The existing jq query evaluator (`src/jq/`)
- The YAML semi-indexing infrastructure (`src/yaml/`)
- Generic evaluator for direct YAML evaluation (`src/jq/eval_generic.rs`)

## Architecture Decision

**Chosen Approach**: Separate `yq` subcommand sharing the jq query engine internally.

```
src/
├── jq/              # Shared query language (parser, evaluator)
├── json/            # JSON semi-indexing
├── yaml/            # YAML semi-indexing
└── bin/succinctly/
    ├── jq_runner.rs # jq subcommand
    └── yq_runner.rs # yq subcommand (new)
```

**Rationale**:
- Matches ecosystem conventions (users expect `jq` for JSON, `yq` for YAML)
- Clean separation of format-specific concerns
- YAML-specific flags don't pollute jq namespace
- Shared query engine avoids code duplication

## Command Interface

### Basic Usage

```bash
# Query YAML file
succinctly yq '.users[].name' config.yaml

# Pipe from stdin
cat config.yaml | succinctly yq '.services[].image'

# Multiple files
succinctly yq '.spec.containers[]' deployment.yaml service.yaml
```

### Flags (yq-compatible)

| Flag | Description | Status |
|------|-------------|--------|
| `-r, --unwrapScalar` | Output raw strings without quotes | ✅ |
| `-I 0` | Compact output (use indent level 0) | ✅ |
| `-n, --null-input` | Don't read input | ✅ |
| `-e, --exit-status` | Exit 1 if last output is false/null | ✅ |
| `-p, --input-format` | Input format: auto, yaml, json | ✅ |
| `-o, --output-format` | Output format: yaml, json, auto | ✅ |
| `-s, --slurp` | Read all inputs into array | ✅ |
| `-i, --inplace` | Update file in place | ✅ |
| `-0, --nul-output` | Use NUL separator instead of newline | ✅ |
| `--tab` | Use tabs for indentation | ✅ |
| `--arg NAME VALUE` | Set $NAME to string VALUE | ✅ |
| `--argjson NAME JSON` | Set $NAME to JSON VALUE | ✅ |

### YAML-Specific Flags

| Flag | Description | Status |
|------|-------------|--------|
| `--output-format yaml` | Output format: `json` (default), `yaml` | ✅ |
| `--no-doc` | Omit document separators (`---`) | ✅ |
| `--preserve-comments` | Preserve comments in YAML output | ❌ Not planned |
| `--explode-anchors` | Expand anchor/alias references | ❌ Not planned |
| `--document N` | Select Nth document from multi-doc stream | ❌ Not planned |
| `--all-documents` | Process all documents | ✅ Default behavior |

## Implementation Phases

### Phase 1: Basic yq Command ✅ COMPLETE

**Goal**: Minimal viable yq supporting common YAML config file patterns.

**Completed**:
- [x] `YqCommand` struct in CLI
- [x] `yq_runner.rs` module (~1500 lines)
- [x] Integration with `YamlIndex::build()`
- [x] Convert `YamlCursor` traversal to `OwnedValue`
- [x] Full jq expression evaluation
- [x] JSON and YAML output formats

**Key Files**:
- `src/bin/succinctly/main.rs` - `YqCommand` definition
- `src/bin/succinctly/yq_runner.rs` - Main yq implementation

---

### Phase 2: Full YAML 1.2 Structural Support ✅ COMPLETE

**Goal**: Support all YAML structural features except anchors.

**Completed**:
- [x] Flow style collections (`{key: value}`, `[item, ...]`)
- [x] Block scalars (`|`, `|+`, `|-`, `>`, `>+`, `>-`)
- [x] Multi-line strings with proper escaping

---

### Phase 3: Anchors, Aliases, and YAML Output ✅ MOSTLY COMPLETE

**Goal**: Full anchor/alias support and optional YAML output format.

**Completed**:
- [x] Anchor definitions (`&anchor`)
- [x] Alias references (`*anchor`)
- [x] `--output-format yaml` flag
- [x] YAML output with proper quoting and indentation

**Not Implemented** (low priority):
- [ ] `--explode-anchors` flag
- [ ] `--preserve-comments` flag
- [ ] Merge keys (`<<: *alias`)

---

### Phase 4: Multi-Document Streams ✅ MOSTLY COMPLETE

**Goal**: Support multi-document YAML files.

**Completed**:
- [x] `---` document separator handling
- [x] Process all documents by default
- [x] `--slurp` collects all documents into array
- [x] `--no-doc` omits separators in output

**Behavior Matrix**:

| Input | Flag | Behavior |
|-------|------|----------|
| Single doc | (none) | Process document |
| Multi doc | (none) | Process all documents |
| Multi doc | `--slurp` | All documents as array |
| Multi doc | `--no-doc` | No `---` separators in output |

**Not Implemented** (low priority):
- [ ] `--document N` to select specific document

---

### Phase 5: YAML-Specific Query Extensions 🔄 PARTIAL

**Goal**: Add YAML-aware operators beyond standard jq.

**Implemented**:
- [x] `line` - Get 1-based line number of node
- [x] `column` - Get 1-based column number of node

**Not Implemented** (implement on demand):

| Operator | Description | Status |
|----------|-------------|--------|
| `anchor` | Get anchor name if present | ❌ |
| `has_anchor` | Check if node has anchor | ❌ |
| `tag` | Get explicit tag | ❌ |
| `style` | Get scalar style | ❌ |
| `comments` | Get associated comments | ❌ |

---

### Phase 6: yq-Specific Operators 🔜 IN SCOPE

**Goal**: Operators unique to yq (Mike Farah's) not in standard jq.

These operators extend jq compatibility with yq-specific functionality commonly used in DevOps workflows.

| Operator | Description | Priority | Status |
|----------|-------------|----------|--------|
| `omit(keys)` | Remove keys from object/indices from array | High | ❌ |
| `shuffle` | Randomize array order | Medium | ❌ |
| `pivot` | Transpose arrays/objects (SQL-style) | Medium | ❌ |
| `document_index` / `di` | Get 0-indexed document position | High | ❌ |
| `split_doc` | Split into multiple YAML documents | Low | ❌ |
| `load(file)` | Load external YAML file | Low | ❌ |
| `eval(expr)` | Evaluate string as expression | Low | ❌ |

#### Operator Details

**`omit(keys)`** - Remove keys from object or indices from array
```bash
# Remove keys from object
{a: 1, b: 2, c: 3} | omit(["a", "c"])  # → {b: 2}

# Remove indices from array
[a, b, c, d] | omit([0, 2])  # → [b, d]

# Gracefully ignores non-existent keys
{a: 1} | omit(["b", "c"])  # → {a: 1}
```

**`shuffle`** - Randomize array element order
```bash
[1, 2, 3, 4, 5] | shuffle  # → [3, 1, 5, 2, 4] (random)
```

**`pivot`** - Transpose data structures (SQL PIVOT emulation)
```bash
# Transpose sequences
[[a, b], [x, y]] | pivot  # → [[a, x], [b, y]]

# Transpose objects (collect values by key)
[{name: "Alice", age: 30}, {name: "Bob", age: 25}] | pivot
# → {name: ["Alice", "Bob"], age: [30, 25]}

# Handles missing keys with null padding
[{a: 1}, {a: 2, b: 3}] | pivot  # → {a: [1, 2], b: [null, 3]}
```

**`document_index`** / **`di`** - Get document position in multi-doc stream
```bash
# Multi-doc YAML with ---
# Returns 0 for first doc, 1 for second, etc.
document_index  # or: di

# Filter by document index
select(document_index == 1)  # → only second document
```

**`split_doc`** - Split results into separate YAML documents
```bash
# Input: [a, b, c]
.[] | split_doc
# Output:
# a
# ---
# b
# ---
# c
```

**Implementation Notes:**
- `omit` is inverse of jq's `pick` (if implemented)
- `shuffle` uses non-cryptographic RNG
- `pivot` requires handling heterogeneous arrays
- `document_index` requires tracking document context during evaluation

---

### Phase 7: Date/Time Operators 🔜 IN SCOPE

**Goal**: Date/time manipulation operators from yq.

These operators use Go's time format strings (not POSIX strftime).

| Operator | Description | Priority | Status |
|----------|-------------|----------|--------|
| `now` | Current timestamp (RFC3339) | High | ❌ |
| `strftime(fmt)` | Format timestamp as string | High | ❌ |
| `strptime(fmt)` | Parse string to timestamp | High | ❌ |
| `from_unix` | Convert Unix epoch to datetime | Medium | ❌ |
| `to_unix` | Convert datetime to Unix epoch | Medium | ❌ |
| `tz(zone)` | Convert to timezone | Medium | ❌ |
| `with_dtf(fmt)` | Set datetime format context | Low | ❌ |

#### Operator Details

**`now`** - Current timestamp in RFC3339 format
```bash
now  # → "2024-01-20T15:04:05Z"
.timestamp = now
```

**`strftime(fmt)`** - Format datetime to string
```bash
# Go time format: "2006-01-02T15:04:05Z07:00"
.date | strftime("2006-01-02")  # → "2024-01-20"
.date | strftime("Monday, 02-Jan-06")  # → "Saturday, 20-Jan-24"
```

**`strptime(fmt)`** - Parse string to datetime
```bash
"2024-01-20" | strptime("2006-01-02")  # → datetime object
```

**`from_unix`** - Unix epoch to datetime
```bash
1705766400 | from_unix  # → "2024-01-20T16:00:00Z"
```

**`to_unix`** - Datetime to Unix epoch
```bash
now | to_unix  # → 1705766400
```

**`tz(zone)`** - Convert to timezone
```bash
now | tz("America/New_York")
now | tz("UTC")
now | tz("local")  # system timezone
```

**Date Arithmetic:**
```bash
.time += "3h10m"   # Add duration
.time -= "24h"     # Subtract duration
```

**Implementation Notes:**
- Go time format uses "2006-01-02 15:04:05" as reference date
- Requires `chrono` crate for Rust implementation
- Timezone handling via `chrono-tz` crate
- Duration parsing: "1h", "30m", "2h30m", etc.

---

### Phase 8: Additional Format Encoders 🔜 IN SCOPE

**Goal**: Format conversion operators beyond standard jq.

| Operator | Description | Priority | Status |
|----------|-------------|----------|--------|
| `@yaml` / `to_yaml` | Encode as YAML string | High | ❌ |
| `@props` / `to_props` | Encode as Java properties | Medium | ❌ |
| `@xml` / `to_xml` | Encode as XML string | Low | ❌ |

#### Operator Details

**`@yaml`** / **`to_yaml`** - Encode value as YAML string
```bash
{a: 1, b: 2} | @yaml
# → "a: 1\nb: 2\n"

# With custom indentation
{a: 1} | to_yaml(4)  # 4-space indent

# Embedding YAML in YAML (common use case)
.config = (.data | @yaml)
```

**`@props`** / **`to_props`** - Encode as Java properties
```bash
{database: "postgres", port: 5432} | @props
# → "database = postgres\nport = 5432\n"
```

**`@xml`** / **`to_xml`** - Encode as XML
```bash
{root: {item: "value"}} | @xml
# → "<root><item>value</item></root>"

# Compact (single line)
{root: {item: "value"}} | to_xml(0)
```

**Implementation Notes:**
- `@yaml` reuses existing YAML output formatting
- `@props` is straightforward key=value formatting
- `@xml` requires handling attributes (prefix convention: `+@attr`)
- All encoders should handle nested structures

---

### Generic Evaluator (Bonus) ✅ COMPLETE

**Goal**: Evaluate jq expressions directly on YAML without JSON conversion.

**Completed**:
- [x] `DocumentValue` / `DocumentCursor` traits in `src/jq/document.rs`
- [x] Generic evaluator in `src/jq/eval_generic.rs` (~750 lines)
- [x] Direct YAML→JSON streaming for identity queries
- [x] `line`/`column` builtins use cursor position metadata
- [x] 2-8x performance improvement over system yq

---

## Testing Strategy

### Unit Tests
- Each conversion path (YAML scalar types → OwnedValue)
- Edge cases (empty docs, deeply nested, wide objects)

### Integration Tests
- Run same queries through `yq` (Mike Farah's) and `succinctly yq`
- Compare outputs for YAML Test Suite files

### Property Tests
```
∀ yaml ∈ ValidYAML1.2:
  let json = succinctly yq '.' yaml_file
  json is valid JSON
  round_trip(json) preserves structure
```

### Compatibility Tests
```bash
# Generate comparison test
for f in test_cases/*.yaml; do
  yq '.users[].name' "$f" > expected.txt
  succinctly yq '.users[].name' "$f" > actual.txt
  diff expected.txt actual.txt
done
```

---

## Performance Considerations

### Indexing vs Direct Parse

| Approach | Throughput | Memory | Repeated Queries |
|----------|------------|--------|------------------|
| Semi-index | ~150 MB/s build | ~4% overhead | O(1) navigation |
| Direct parse | ~150 MB/s | Higher | Re-parse each time |

For single queries, both approaches are similar. Semi-indexing wins for:
- Multiple queries on same file
- `jq-locate` style offset→path lookups
- Large files with selective access

### Initial Implementation

Phase 1 will use direct conversion to `OwnedValue` for simplicity:
```rust
fn yaml_to_owned_value(index: &YamlIndex) -> OwnedValue {
    // Full tree traversal, materializes everything
}
```

Future optimization: lazy cursor-based evaluation (like JSON implementation).

---

## Error Handling

### YAML Parse Errors
```
Error: invalid YAML at line 15, column 3
  |
15|   - name: Alice
  |   ^
  = unexpected character '-' in mapping context
```

### Query Errors
```
Error: cannot index string with number
  |
  = in expression: .name[0]
  = value at .name is a string: "Alice"
```

### Multi-Document Errors
```
Error: document index 5 out of range
  = file contains 3 documents (indices 0-2)
```

---

## CLI Help Text

```
succinctly-yq
Command-line YAML processor (jq-compatible)

USAGE:
    succinctly yq [OPTIONS] [FILTER] [FILES]...

ARGS:
    <FILTER>      jq filter expression (default: ".")
    <FILES>...    Input YAML files (reads stdin if none)

INPUT OPTIONS:
    -n, --null-input     Don't read input; use null as input
    -p, --input-format   Input format: auto, yaml, json [default: auto]

OUTPUT OPTIONS:
    -r, --unwrapScalar   Output raw strings without quotes
    -I, --indent <N>     Indent level (0 for compact) [default: 2]
    -o, --output-format  Output format: yaml, json, auto [default: yaml]
    -i, --inplace        Update file in place
    -0, --nul-output     Use NUL separator instead of newline

YAML OPTIONS:
    --explode-anchors    Expand anchor/alias references
    --document <N>       Select Nth document (0-indexed)
    --all-documents      Process all documents in stream

VARIABLES:
    --arg <NAME> <VAL>   Set $NAME to string VAL
    --argjson <NAME> <JSON>  Set $NAME to JSON value

EXAMPLES:
    succinctly yq '.metadata.name' deployment.yaml
    succinctly yq '.spec.containers[].image' *.yaml
    cat config.yaml | succinctly yq '.services | keys'
    succinctly yq -r '.users[].email' --output-format yaml users.yaml
```

---

## Dependencies

### Existing (no new crates needed for Phase 1)
- `succinctly::yaml::YamlIndex` - YAML semi-indexing
- `succinctly::jq` - Query language parser and evaluator
- `clap` - CLI argument parsing
- `anyhow` - Error handling

### Future Phases
- None anticipated (YAML output is string formatting)

---

## Migration Path from Other yq Tools

### From Mike Farah's yq (Go)

| Feature | Farah yq | succinctly yq |
|---------|----------|---------------|
| Basic queries | `.foo.bar` | ✅ `.foo.bar` |
| Array iteration | `.[]` | ✅ `.[]` |
| Select | `select(.active)` | ✅ `select(.active)` |
| In-place edit | `-i` | ✅ `-i` |
| Slurp | `-s` | ✅ `-s` |
| Eval | `eval` | Not needed |
| XML/TOML | Supported | Not planned |

### From kislyuk/yq (Python)

| Feature | kislyuk yq | succinctly yq |
|---------|------------|---------------|
| Syntax | jq | jq |
| YAML output | `-y` | `--output-format yaml` |
| Streaming | Full jq | Partial |

---

## Success Criteria

### Phase 1 Complete When: ✅ DONE
- [x] `succinctly yq '.' file.yaml` outputs JSON
- [x] Basic field access works: `.metadata.name`
- [x] Array iteration works: `.items[]`
- [x] Filters work: `select(.kind == "Deployment")`
- [x] Tests pass for Kubernetes manifests, GitHub Actions, Docker Compose

### Full Implementation Complete When: ✅ DONE
- [x] All jq operators work on YAML input
- [x] Multi-document YAML supported
- [x] YAML output format supported
- [x] Anchors/aliases handled correctly
- [x] Performance within 2x of Mike Farah's yq (actually **2-8x faster**)

---

## Timeline Dependency

This plan depends on the YAML parser implementation phases defined in [parsing/yaml.md](../parsing/yaml.md):

| yq Phase | Requires YAML Parser Phase |
|----------|---------------------------|
| 1 (Basic) | 1 (YAML-lite) |
| 2 (Full structural) | 2 (Flow + Block scalars) |
| 3 (Anchors) | 3 (Anchors/Aliases) |
| 4 (Multi-doc) | 4 (Multi-document) |

---

## Open Questions

1. **Default output format**: Should default be JSON or YAML?
   - *Decision*: JSON (matches jq behavior, more tooling compatible) ✅ Implemented

2. **Anchor expansion default**: Expand aliases automatically or preserve structure?
   - *Decision*: Expand automatically (matches user expectations from jq) ✅ Implemented

3. **Comment handling**: How to expose comments in queries?
   - *Decision*: Deferred - implement on user demand

4. **Schema validation**: Should yq validate against YAML schemas?
   - *Decision*: Out of scope (separate tool concern)

---

## Changelog

| Date | Change |
|------|--------|
| 2026-01-20 | Phase 6-8 brought into scope with detailed operator specs |
| 2026-01-20 | Added Phase 6-8 for yq-specific features (not planned) |
| 2026-01-20 | Added `--slurp` CLI option |
| 2026-01-20 | Generic evaluator wired into main CLI path |
| 2026-01-20 | Phase 1-4 marked complete, updated status tables |
