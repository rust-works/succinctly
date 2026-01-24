# yq Query Language Reference

[Home](/) > [Docs](../) > [Reference](./) > yq Language

## Overview

The `yq` command provides YAML querying using jq-compatible syntax. It supports all features from the [jq Language Reference](jq-language.md) plus YAML-specific extensions.

## Implementation Status Summary

| Category                  | Status            | Coverage |
|---------------------------|-------------------|----------|
| All jq features           | Fully implemented | 100%     |
| YAML metadata functions   | Fully implemented | 100%     |
| yq-specific operators     | Fully implemented | 95%      |
| Date/time operators       | Fully implemented | 95%      |
| Format encoders           | Fully implemented | 90%      |
| Multi-document support    | Fully implemented | 100%     |
| Anchors/aliases           | Fully implemented | 95%      |

---

## Performance

### Apple M1 Max

| Size   | succinctly          | system yq           | Speedup   |
|--------|---------------------|---------------------|-----------|
| 10KB   | 4.2 ms (2.3 MiB/s)  | 8.4 ms (1.2 MiB/s)  | **2.0x**  |
| 100KB  | 5.5 ms (16.8 MiB/s) | 20.6 ms (4.5 MiB/s) | **3.8x**  |
| 1MB    | 15.3 ms (60.2 MiB/s)| 120.7 ms (7.6 MiB/s)| **7.9x**  |

### AMD Ryzen 9 7950X

| Size   | succinctly            | system yq             | Speedup   |
|--------|-----------------------|-----------------------|-----------|
| 10KB   | 1.63 ms (6.0 MiB/s)   | 64.6 ms (155 KiB/s)   | **40x**   |
| 100KB  | 2.78 ms (33.1 MiB/s)  | 79.6 ms (1.2 MiB/s)   | **29x**   |
| 1MB    | 13.2 ms (69.7 MiB/s)  | 210.5 ms (4.4 MiB/s)  | **16x**   |

---

## jq Compatibility

All features from the [jq Language Reference](jq-language.md) work with yq, including:

- Path expressions (`.foo`, `.[0]`, `.[]`, `..`)
- Operators (arithmetic, comparison, boolean, pipe, comma)
- Array/object operations (`keys`, `values`, `sort`, `group_by`, etc.)
- String functions (`split`, `join`, `test`, `match`, etc.)
- Control flow (`if-then-else`, `try-catch`, `reduce`, `foreach`)
- User-defined functions (`def name: body;`)
- Variable binding (`as $var`)
- Assignment operators (`=`, `|=`, `+=`, etc.)
- Format strings (`@json`, `@csv`, `@base64`, etc.)
- Module system (`import`, `include`)

---

## YAML-Specific Features

### Metadata Functions

These functions access YAML-specific metadata not available in JSON:

| Function   | Description                        | Example Output                          |
|------------|------------------------------------|-----------------------------------------|
| `tag`      | YAML type tag                      | `!!str`, `!!int`, `!!map`               |
| `anchor`   | Anchor name for nodes with `&name` | `"myanchor"` or `""`                    |
| `style`    | Scalar/collection style            | `"double"`, `"literal"`, `"flow"`       |
| `kind`     | Node kind                          | `"scalar"`, `"seq"`, `"map"`, `"alias"` |
| `key`      | Current key when iterating         | `"name"` or `0`                         |
| `line`     | 1-based line number                | `5`                                     |
| `column`   | 1-based column number              | `3`                                     |

```bash
# Get line numbers of all items
succinctly yq '.items[] | {name: .name, line: line}' file.yaml

# Find nodes with anchors
succinctly yq '.. | select(anchor != "")' file.yaml

# Get style of scalars
succinctly yq '.description | style' file.yaml
# Output: "literal" (for block scalar |)
```

### Multi-Document Support

| Function           | Description                                   |
|--------------------|-----------------------------------------------|
| `document_index`   | 0-indexed document position in stream         |
| `di`               | Alias for `document_index`                    |
| `split_doc`        | Mark outputs as separate YAML documents       |

```bash
# Select second document only
succinctly yq 'select(di == 1)' multi.yaml

# Filter by document index
succinctly yq 'select(document_index == 0) | .metadata' multi.yaml

# Split array into separate documents
succinctly yq '.items[] | split_doc' file.yaml
# Output:
# item1
# ---
# item2
# ---
# item3
```

### yq-Specific Operators

| Operator      | Description                                  | Example                        |
|---------------|----------------------------------------------|--------------------------------|
| `pick(keys)`  | Select only specified keys                   | `pick(["a", "b"])`             |
| `omit(keys)`  | Remove specified keys (inverse of pick)      | `omit(["temp", "debug"])`      |
| `shuffle`     | Randomly shuffle array elements              | `[1,2,3] \| shuffle`           |
| `pivot`       | Transpose arrays/objects (SQL-style)         | `[[1,2],[3,4]] \| pivot`       |
| `load(file)`  | Load external YAML/JSON file                 | `load("config.yaml")`          |
| `parent`      | Return parent node                           | `.. \| select(.name) \| parent`|
| `parent(n)`   | Return nth parent node                       | `parent(2)`                    |

```bash
# Remove keys from object
echo '{a: 1, b: 2, c: 3}' | succinctly yq 'omit(["a", "c"])'
# Output: {b: 2}

# Transpose array of arrays
echo '[[a, b], [x, y]]' | succinctly yq 'pivot'
# Output: [[a, x], [b, y]]

# Transpose array of objects
echo '[{name: "Alice", age: 30}, {name: "Bob", age: 25}]' | succinctly yq 'pivot'
# Output: {name: ["Alice", "Bob"], age: [30, 25]}

# Load external file
succinctly yq '. + {config: load("defaults.yaml")}' input.yaml

# Get parent of matching node
succinctly yq '.. | select(.name == "target") | parent' file.yaml
```

### Date/Time Extensions

Beyond jq's standard date functions (`now`, `gmtime`, `strftime`, `strptime`), yq adds:

| Function        | Description                   | Example                             |
|-----------------|-------------------------------|-------------------------------------|
| `from_unix`     | Unix epoch to ISO 8601 string | `1705766400 \| from_unix`           |
| `to_unix`       | ISO 8601 string to Unix epoch | `"2024-01-20T16:00:00Z" \| to_unix` |
| `tz(zone)`      | Convert timestamp to timezone | `now \| tz("America/New_York")`     |

```bash
# Convert Unix timestamp to datetime
echo '1705766400' | succinctly yq 'from_unix'
# Output: "2024-01-20T16:00:00Z"

# Convert datetime to Unix timestamp
echo '"2024-01-20T16:00:00Z"' | succinctly yq 'to_unix'
# Output: 1705766400

# Convert to specific timezone
echo '1705314600' | succinctly yq 'tz("America/New_York")'
# Output: "2024-01-15T05:30:00-05:00"

echo '1705314600' | succinctly yq 'tz("Asia/Tokyo")'
# Output: "2024-01-15T19:30:00+09:00"
```

**Supported timezone formats:**
- IANA names: `America/New_York`, `Europe/London`, `Asia/Tokyo`
- Abbreviations: `EST`, `PST`, `JST`, `CET`, `UTC`, `GMT`
- Numeric offsets: `+05:30`, `-0800`, `+09`

### Format Encoders

Beyond jq's standard formats (`@json`, `@csv`, `@base64`, etc.), yq adds:

| Format    | Description                        | Example Output           |
|-----------|------------------------------------|--------------------------|
| `@yaml`   | YAML flow-style encoding           | `{a: 1, b: 2}`           |
| `@props`  | Java properties format             | `key = value`            |

```bash
# Encode as YAML string (flow-style)
echo '{a: 1, b: [2, 3]}' | succinctly yq '@yaml'
# Output: "{a: 1, b: [2, 3]}"

# Encode as Java properties
echo '{database: "postgres", port: 5432}' | succinctly yq '@props'
# Output:
# database = postgres
# port = 5432

# Nested objects use dot-notation
echo '{db: {host: "localhost", port: 5432}}' | succinctly yq '@props'
# Output:
# db.host = localhost
# db.port = 5432
```

### Position-Based Navigation (Succinctly Extension)

Same as jq - see [jq Language Reference](jq-language.md#succinctly-extensions):

| Builtin                    | Description                                |
|----------------------------|--------------------------------------------|
| `at_offset(n)`             | Jump to node at byte offset n (0-indexed)  |
| `at_position(line; col)`   | Jump to node at line/column (1-indexed)    |

```bash
# Jump to node at byte offset
succinctly yq 'at_offset(10)' config.yaml

# Jump to node at line 5, column 3
succinctly yq 'at_position(5; 3)' config.yaml
```

---

## CLI Options

### Input Options

| Flag                  | Description                              |
|-----------------------|------------------------------------------|
| `-n, --null-input`    | Don't read input; use null               |
| `-p, --input-format`  | Input format: `auto`, `yaml`, `json`     |
| `-s, --slurp`         | Read all inputs into array               |
| `-R, --raw-input`     | Read lines as strings instead of YAML    |
| `--doc N`             | Select Nth document (0-indexed)          |

### Output Options

| Flag                  | Description                              |
|-----------------------|------------------------------------------|
| `-r, --unwrapScalar`  | Output raw strings without quotes        |
| `-I, --indent N`      | Indent level (0 for compact)             |
| `-o, --output-format` | Output format: `yaml`, `json`, `auto`    |
| `-i, --inplace`       | Update file in place                     |
| `-0, --nul-output`    | Use NUL separator instead of newline     |
| `--no-doc`            | Omit document separators (`---`)         |
| `--tab`               | Use tabs for indentation                 |

### Variables

| Flag                      | Description                          |
|---------------------------|--------------------------------------|
| `--arg NAME VALUE`        | Set $NAME to string VALUE            |
| `--argjson NAME JSON`     | Set $NAME to JSON VALUE              |

---

## Multi-Document Behavior

| Input       | Flag        | Behavior                           |
|-------------|-------------|------------------------------------|
| Single doc  | (none)      | Process document                   |
| Multi doc   | (none)      | Process all documents              |
| Multi doc   | `--slurp`   | All documents as array             |
| Multi doc   | `--doc N`   | Select Nth document only           |
| Multi doc   | `--no-doc`  | No `---` separators in output      |

---

## YamlCursor API (Programmatic Access)

For direct Rust API access, `YamlCursor` provides metadata methods:

```rust
cursor.anchor()   // Option<&str> - anchor name
cursor.alias()    // Option<&str> - referenced anchor for alias nodes
cursor.is_alias() // bool - check if node is an alias
cursor.style()    // &'static str - "double", "single", "literal", "folded", "flow", ""
cursor.tag()      // &'static str - inferred YAML type tag
cursor.kind()     // &'static str - "scalar", "seq", "map", "alias"
cursor.line()     // usize - 1-based line number
cursor.column()   // usize - 1-based column number
```

---

## Known Limitations

See [yq Remaining Work](../plan/yq-remaining.md) for incomplete features.

### Intentionally Not Implemented

1. **XML/TOML input/output** - Out of scope (separate tools)
2. **Comment preservation** - Comments not stored in semi-index
3. **Merge keys** (`<<: *alias`) - Rarely used, complex semantics
4. **Schema validation** - Separate tool concern

### Partial Implementation Notes

1. **`line`/`column` in complex expressions** - Work best with direct cursor access; may return 0 after DOM conversion
2. **Anchor metadata** - Available at cursor level; may be lost after complex jq operations

---

## Examples

### Kubernetes Manifest Processing

```bash
# Get all container images
succinctly yq '.spec.template.spec.containers[].image' deployment.yaml

# Filter deployments by label
succinctly yq 'select(.metadata.labels.app == "web")' *.yaml

# Update image tag
succinctly yq '.spec.template.spec.containers[0].image = "nginx:1.25"' -i deployment.yaml

# Get all resource requests
succinctly yq '.spec.template.spec.containers[] | {name: .name, cpu: .resources.requests.cpu}' deployment.yaml
```

### GitHub Actions Workflow

```bash
# List all job names
succinctly yq '.jobs | keys' .github/workflows/ci.yml

# Get all uses: actions
succinctly yq '.jobs[].steps[].uses | select(. != null)' .github/workflows/ci.yml

# Find steps with specific action
succinctly yq '.jobs[].steps[] | select(.uses | startswith("actions/checkout"))' .github/workflows/ci.yml
```

### Docker Compose

```bash
# List all service names
succinctly yq '.services | keys' docker-compose.yml

# Get all exposed ports
succinctly yq '.services[].ports[]' docker-compose.yml

# Find services with specific image
succinctly yq '.services | to_entries[] | select(.value.image | contains("postgres"))' docker-compose.yml
```

---

## Changelog

| Date       | Change                                                   |
|------------|----------------------------------------------------------|
| 2026-01-20 | Initial yq implementation complete                       |
| 2026-01-20 | Added date/time extensions: `from_unix`, `to_unix`, `tz` |
| 2026-01-20 | Added `@yaml`, `@props` format encoders                  |
| 2026-01-20 | Added `load(file)` operator                              |
| 2026-01-20 | Added `split_doc` operator                               |
| 2026-01-20 | Added multi-document support with `--doc N`              |
| 2026-01-24 | Document created from plan/yq.md                         |
