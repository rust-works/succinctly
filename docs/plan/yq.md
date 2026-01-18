# yq Command Implementation Plan

This document outlines the plan to add a `yq` subcommand to succinctly for querying YAML files using jq-compatible syntax.

## Overview

The `yq` command will provide YAML querying capabilities using the same query language as `jq`. It will leverage:
- The existing jq query evaluator (`src/jq/`)
- The YAML semi-indexing infrastructure (`src/yaml/`)
- A new `yq` subcommand in the CLI

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

| Flag | Description |
|------|-------------|
| `-r, --unwrapScalar` | Output raw strings without quotes |
| `-I 0` | Compact output (use indent level 0) |
| `-n, --null-input` | Don't read input |
| `-e, --exit-status` | Exit 1 if last output is false/null |
| `-p, --input-format` | Input format: auto, yaml, json |
| `-o, --output-format` | Output format: yaml, json, auto |
| `-i, --inplace` | Update file in place |
| `-0, --nul-output` | Use NUL separator instead of newline |
| `--arg NAME VALUE` | Set $NAME to string VALUE |
| `--argjson NAME JSON` | Set $NAME to JSON VALUE |

### YAML-Specific Flags

| Flag | Description |
|------|-------------|
| `--output-format` | Output format: `json` (default), `yaml` |
| `--preserve-comments` | Preserve comments in YAML output (Phase 3+) |
| `--explode-anchors` | Expand anchor/alias references (Phase 3+) |
| `--document N` | Select Nth document from multi-doc stream (Phase 4+) |
| `--all-documents` | Process all documents (Phase 4+) |

## Implementation Phases

### Phase 1: Basic yq Command

**Goal**: Minimal viable yq supporting common YAML config file patterns.

**Scope**:
- Parse YAML-lite subset (block style, simple scalars)
- Convert YAML to internal value representation
- Evaluate jq expressions using existing evaluator
- Output as JSON (simplest path)

**Deliverables**:
1. `YqCommand` struct in CLI (mirrors `JqCommand`)
2. `yq_runner.rs` module
3. Integration with `YamlIndex::build()`
4. Convert `YamlCursor` traversal to `OwnedValue`
5. Pipe `OwnedValue` through existing jq evaluator

**Files to Create/Modify**:
```
src/bin/succinctly/main.rs        # Add Yq(YqCommand) variant
src/bin/succinctly/yq_runner.rs   # New file (similar to jq_runner.rs)
```

**Example Implementation Sketch**:
```rust
// yq_runner.rs
pub fn run_yq(args: YqCommand) -> Result<i32> {
    // 1. Read YAML input
    let yaml_bytes = read_input(&args)?;

    // 2. Build YAML index
    let index = YamlIndex::build(&yaml_bytes)?;

    // 3. Convert to OwnedValue (tree traversal)
    let value = yaml_to_owned_value(&index)?;

    // 4. Parse and evaluate jq expression
    let program = jq::parse_program(&args.filter.unwrap_or(".".to_string()))?;
    let results = jq::eval(&program, &value, &context)?;

    // 5. Output results (JSON format initially)
    output_results(&results, &args)?;

    Ok(exit_codes::SUCCESS)
}
```

**Coverage**: ~70% of real-world YAML config files (Kubernetes, GitHub Actions, Docker Compose basics).

---

### Phase 2: Full YAML 1.2 Structural Support

**Goal**: Support all YAML structural features except anchors.

**New Features**:
- Flow style collections (`{key: value}`, `[item, ...]`)
- Block scalars (`|`, `|+`, `|-`, `>`, `>+`, `>-`)
- Explicit keys (`? key`)

**Dependencies**: Requires YAML parser Phase 2 from [yaml.md](../parsing/yaml.md).

**Deliverables**:
1. Handle flow/block distinction in `yaml_to_owned_value()`
2. Block scalar content extraction with chomping
3. Update cursor navigation for flow containers

**Coverage**: ~95% of real-world YAML files.

---

### Phase 3: Anchors, Aliases, and YAML Output

**Goal**: Full anchor/alias support and optional YAML output format.

**New Features**:
- Anchor definitions (`&anchor`)
- Alias references (`*anchor`)
- Merge keys (`<<: *alias`)
- `--output-format yaml` flag
- `--explode-anchors` flag
- `--preserve-comments` flag (YAML output only)

**Dependencies**: Requires YAML parser Phase 3 from [yaml.md](../parsing/yaml.md).

**Implementation Decisions**:

| Feature | Behavior |
|---------|----------|
| Alias traversal | Follow aliases transparently by default |
| `--explode-anchors` | Materialize aliases as copies |
| Circular references | Error with clear message |
| `--preserve-comments` | Only for YAML→YAML transforms |

**YAML Output Implementation**:
```rust
fn output_yaml(value: &OwnedValue, indent: usize) -> String {
    match value {
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(b) => b.to_string(),
        OwnedValue::Int(n) => n.to_string(),
        OwnedValue::Float(f) => format_float(*f),
        OwnedValue::String(s) => yaml_quote_string(s),
        OwnedValue::Array(arr) => format_yaml_array(arr, indent),
        OwnedValue::Object(obj) => format_yaml_object(obj, indent),
    }
}
```

---

### Phase 4: Multi-Document Streams

**Goal**: Support multi-document YAML files.

**New Features**:
- `---` document separator handling
- `--document N` to select specific document
- `--all-documents` to process each document
- Default: process first document only

**Dependencies**: Requires YAML parser Phase 4 from [yaml.md](../parsing/yaml.md).

**Behavior Matrix**:

| Input | Flag | Behavior |
|-------|------|----------|
| Single doc | (none) | Process document |
| Multi doc | (none) | Process first document |
| Multi doc | `--document 0` | Process first document |
| Multi doc | `--document 2` | Process third document |
| Multi doc | `--all-documents` | Process each, output separated by `---` |
| Multi doc | `--slurp` | All documents as array |

---

### Phase 5: YAML-Specific Query Extensions (Optional)

**Goal**: Add YAML-aware operators beyond standard jq.

**Potential Extensions**:

| Operator | Description | Example |
|----------|-------------|---------|
| `anchor` | Get anchor name if present | `.foo \| anchor` → `"my_anchor"` |
| `has_anchor` | Check if node has anchor | `.foo \| has_anchor` → `true` |
| `tag` | Get explicit tag | `.foo \| tag` → `"!!str"` |
| `style` | Get scalar style | `.foo \| style` → `"literal"` |
| `comments` | Get associated comments | `.foo \| comments` → `["comment text"]` |

**Decision**: Implement only if there's user demand. Standard jq operators cover most use cases.

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
| Basic queries | `.foo.bar` | `.foo.bar` |
| Array iteration | `.[]` | `.[]` |
| Select | `select(.active)` | `select(.active)` |
| In-place edit | `-i` | Not planned |
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

### Phase 1 Complete When:
- [ ] `succinctly yq '.' file.yaml` outputs JSON
- [ ] Basic field access works: `.metadata.name`
- [ ] Array iteration works: `.items[]`
- [ ] Filters work: `select(.kind == "Deployment")`
- [ ] Tests pass for Kubernetes manifests, GitHub Actions, Docker Compose

### Full Implementation Complete When:
- [ ] All jq operators work on YAML input
- [ ] Multi-document YAML supported
- [ ] YAML output format supported
- [ ] Anchors/aliases handled correctly
- [ ] Performance within 2x of Mike Farah's yq

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
   - *Recommendation*: JSON (matches jq behavior, more tooling compatible)

2. **Anchor expansion default**: Expand aliases automatically or preserve structure?
   - *Recommendation*: Expand automatically (matches user expectations from jq)

3. **Comment handling**: How to expose comments in queries?
   - *Recommendation*: Defer to Phase 5, low priority

4. **Schema validation**: Should yq validate against YAML schemas?
   - *Recommendation*: Out of scope (separate tool concern)
