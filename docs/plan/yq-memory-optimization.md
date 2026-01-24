 # yq Memory Optimization Plan

This document analyzes memory usage in the `yq` command and proposes optimizations to reduce memory consumption.

## Problem Statement

The `yq` command uses more memory than expected given that succinct data structures are designed for space efficiency. While benchmarks show succinctly uses **7-14x less memory** than system `yq` on large files, the absolute memory usage is still higher than the theoretical minimum.

**Current memory usage (100MB YAML file, Apple M1 Max):**

| Pattern       | succinctly | system yq | Ratio |
|---------------|------------|-----------|-------|
| nested        | 250 MB     | 4 GB      | 0.06x |
| comprehensive | 491 MB     | 7 GB      | 0.07x |
| users         | 545 MB     | 9 GB      | 0.06x |

**Expected**: If succinct structures are ~10-15% overhead, a 100MB file should use ~115MB total.

**Actual**: 250-545MB = **2.5-5.5× the input size**.

## Root Cause Analysis

### 1. YamlIndex Memory (Efficient - ~15-25% overhead)

The succinct data structures are working as designed:

| Structure                  | Size Formula     | Purpose                    |
|----------------------------|------------------|----------------------------|
| `ib` (interest bits)       | N/8 bytes        | Structural position markers|
| `ib_rank`                  | N/16 bytes       | Cumulative popcount        |
| `bp` (balanced parens)     | ~2M/8 bytes      | Tree structure             |
| `bp` indices (L0/L1/L2)    | ~5% of BP        | Min-excess navigation      |
| `ty` (type bits)           | M/8 bytes        | Container type markers     |
| **`bp_to_text`**           | **M × 4 bytes**  | BP→text offset mapping     |
| **`bp_to_text_end`**       | **M × 4 bytes**  | Scalar end positions       |
| `seq_items`, `containers`  | M/8 bytes each   | Marker bits                |
| `newlines` (BitVec)        | N/8 + indices    | Line/column lookup         |

Where:
- `N` = input size in bytes
- `M` = number of nodes (BP opens)

For typical YAML, `M ≈ N/10` (average ~10 bytes per node), so:
- `bp_to_text` + `bp_to_text_end` ≈ 0.8× input size
- Total YamlIndex overhead ≈ 15-25% of input size

**Verdict**: YamlIndex is efficient. The overhead comes from elsewhere.

### 2. OwnedValue DOM Materialization (Primary Problem - 3-5× overhead)

The **major memory consumer** is converting the indexed representation to `OwnedValue`:

```rust
pub enum OwnedValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),                         // 24 bytes + heap allocation
    Array(Vec<OwnedValue>),                 // 24 bytes + heap allocation
    Object(IndexMap<String, OwnedValue>),   // ~72 bytes + heap allocation
}
```

**Memory overhead per value type:**

| Type      | Stack Size | Heap Overhead          | Notes                       |
|-----------|------------|------------------------|-----------------------------|
| Null/Bool | 16 bytes   | 0                      | Enum discriminant + padding |
| Int/Float | 16 bytes   | 0                      | 8-byte value + padding      |
| String    | 24 bytes   | len + capacity padding | Copies source text          |
| Array     | 24 bytes   | 24×len + capacity      | Recursive overhead          |
| Object    | ~72 bytes  | ~40×entries + keys     | IndexMap hash table         |

**Why OwnedValue is 3-5× the source text:**

1. **String duplication**: Every string value copies bytes from source
2. **IndexMap overhead**: ~40 bytes per key-value pair for hash table
3. **Vec capacity**: Vecs often allocate 2× needed capacity
4. **Recursive overhead**: Each nested value has its own allocations
5. **Alignment padding**: Rust aligns to 8 bytes

### 3. Result Buffering (Additional 1-2×)

Non-fast-path code buffers all results before output:

```rust
// yq_runner.rs:1643
let mut all_results: Vec<Vec<Vec<OwnedValue>>> = Vec::new();
```

This triple-nested structure holds **all query results in memory** before writing any output.

### 4. Cloning for Exit Status (Additional copies)

```rust
// yq_runner.rs:1698
last_output = Some(result.clone());
```

Every result is cloned just to track the last value for `--exit-status`.

### 5. JSON Serialization Intermediates

```rust
// value.rs:194 - to_json()
let parts: Vec<String> = elements.iter().map(|e| e.to_json()).collect();
parts.join(",")
```

Creates intermediate `Vec<String>` during serialization.

## Memory Flow Diagram

```
Input (YAML file)
  │
  ▼
Vec<u8> ─────────────────────────────────────── [1× input size]
  │
  ▼
YamlIndex::build()
  │
  ├─ ib, bp, ty, seq_items, containers ──────── [~0.2× input size]
  ├─ bp_to_text, bp_to_text_end ─────────────── [~0.8× input size]
  └─ rank indices, newlines ─────────────────── [~0.1× input size]
  │                                              ─────────────────
  │                                              [~1.1× input size]
  ▼
┌─────────────────────────────────────────────────────────────────┐
│ FAST PATH (identity + JSON compact output)                      │
│                                                                 │
│   stream_json() ──────────────────────────── [~0× additional]   │
│   (writes directly to output, no OwnedValue)                    │
│                                                                 │
│   TOTAL: ~2.1× input size                                       │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│ STANDARD PATH (all other queries)                               │
│                                                                 │
│   evaluate_yaml_cursor()                                        │
│     └─ to_owned() ────────────────────────── [3-5× input size]  │
│                                                                 │
│   all_results: Vec<Vec<Vec<OwnedValue>>> ─── [held in memory]   │
│                                                                 │
│   for result in results {                                       │
│       last_output = Some(result.clone()); ── [+1× result size]  │
│       output_value(&result) {                                   │
│           to_json() ────────────────────────  [+1× result size] │
│       }                                                         │
│   }                                                             │
│                                                                 │
│   TOTAL: ~6-8× input size                                       │
└─────────────────────────────────────────────────────────────────┘
```

## Code Locations

| Location                                                             | Function                | Memory Impact                 |
|----------------------------------------------------------------------|-------------------------|-------------------------------|
| [yq_runner.rs:130-211](../src/bin/succinctly/yq_runner.rs#L130-L211) | `yaml_to_owned_value()` | Full DOM materialization      |
| [eval_generic.rs:29-64](../src/jq/eval_generic.rs#L29-L64)           | `to_owned()`            | Query result materialization  |
| [yq_runner.rs:1643](../src/bin/succinctly/yq_runner.rs#L1643)        | `all_results`           | Triple-nested buffer          |
| [yq_runner.rs:1698](../src/bin/succinctly/yq_runner.rs#L1698)        | `last_output.clone()`   | Redundant cloning             |
| [value.rs:194](../src/jq/value.rs#L194)                              | `to_json()`             | Intermediate strings          |
| [yq_runner.rs:596-700](../src/bin/succinctly/yq_runner.rs#L596-L700) | `emit_yaml_value()`     | Output string building        |

## Proposed Optimizations

### Phase M1: Streaming Output for Non-Identity Queries

**Goal**: Eliminate `all_results` buffering by streaming results as they're generated.

**Current code:**
```rust
// Collect all results first
let mut all_results: Vec<Vec<Vec<OwnedValue>>> = Vec::new();
for (bytes, format) in &input_sources {
    let (doc_results, _) = evaluate_yaml_direct_filtered(...)?;
    all_results.push(doc_results);
}

// Then output
for doc_results in all_results {
    for results in doc_results {
        for result in results {
            output_value(&result)?;
        }
    }
}
```

**Proposed:**
```rust
// Stream results directly
for (bytes, format) in &input_sources {
    evaluate_yaml_direct_streaming(bytes, &expr, |result| {
        output_value(&result)?;
        Ok(())
    })?;
}
```

**Impact**: Eliminates holding all results in memory. Memory becomes O(max_single_result) instead of O(total_results).

**Complexity**: Medium. Requires refactoring output logic to handle streaming.

---

### Phase M2: Reference-Based Results for Navigation Queries

**Goal**: Avoid `to_owned()` for queries that just navigate to subtrees.

For queries like `.users[0]` or `.config.database`, the result is a subtree of the input. Instead of materializing it as OwnedValue, return a cursor reference.

**Current code:**
```rust
fn evaluate_yaml_cursor(cursor, expr) -> Vec<OwnedValue> {
    let result = eval_with_cursor(expr, cursor);
    match result {
        GenericResult::OneCursor(c) => vec![to_owned(&c.value())],  // Materializes!
        ...
    }
}
```

**Proposed:**
```rust
enum StreamableResult<'a, W> {
    Cursor(YamlCursor<'a, W>),  // Can stream directly
    Owned(OwnedValue),           // Must serialize from memory
}

fn evaluate_yaml_cursor_lazy(cursor, expr) -> Vec<StreamableResult> {
    let result = eval_with_cursor(expr, cursor);
    match result {
        GenericResult::OneCursor(c) => vec![StreamableResult::Cursor(c)],  // No copy!
        GenericResult::Owned(v) => vec![StreamableResult::Owned(v)],
        ...
    }
}
```

**Impact**: For navigation queries (majority of real-world usage), eliminates OwnedValue entirely.

**Complexity**: High. Requires:
- New `StreamableResult` enum
- Lifetime management for cursor references
- Streaming serialization for cursor results

---

### Phase M3: Eliminate Exit Status Cloning

**Goal**: Replace `last_output = Some(result.clone())` with a flag or lightweight tracking.

**Current code:**
```rust
for result in results {
    last_output = Some(result.clone());  // Clones entire result!
    output_value(&result)?;
}

// Later:
if let Some(OwnedValue::Null | OwnedValue::Bool(false)) = last_output {
    return Ok(exit_codes::FALSE_OR_NULL);
}
```

**Proposed:**
```rust
enum LastOutputStatus {
    None,
    FalseOrNull,
    Other,
}

for result in results {
    last_status = match &result {
        OwnedValue::Null | OwnedValue::Bool(false) => LastOutputStatus::FalseOrNull,
        _ => LastOutputStatus::Other,
    };
    output_value(&result)?;
}
```

**Impact**: Eliminates O(result_size) cloning per result.

**Complexity**: Low. Simple refactor.

---

### Phase M4: Streaming JSON Serialization

**Goal**: Write JSON directly to output without intermediate strings.

**Current code:**
```rust
fn to_json(&self) -> String {
    match self {
        OwnedValue::Array(elements) => {
            let parts: Vec<String> = elements.iter().map(|e| e.to_json()).collect();
            format!("[{}]", parts.join(","))
        }
        ...
    }
}
```

**Proposed:**
```rust
fn write_json<W: Write>(&self, writer: &mut W) -> io::Result<()> {
    match self {
        OwnedValue::Array(elements) => {
            writer.write_all(b"[")?;
            for (i, elem) in elements.iter().enumerate() {
                if i > 0 { writer.write_all(b",")?; }
                elem.write_json(writer)?;
            }
            writer.write_all(b"]")
        }
        ...
    }
}
```

**Impact**: Eliminates intermediate `Vec<String>` and string concatenation.

**Complexity**: Low-Medium. Straightforward refactor of serialization.

---

### Phase M5: Lazy IndexMap Construction

**Goal**: Defer hash table construction for objects until needed.

For large objects that are immediately serialized, building the full `IndexMap` is wasteful.

**Proposed**: Use a lazy wrapper that only builds the hash table on `.get()`:

```rust
enum LazyObject {
    // Fields stored as Vec, no hash table yet
    Pending(Vec<(String, OwnedValue)>),
    // Hash table built on first lookup
    Indexed(IndexMap<String, OwnedValue>),
}
```

**Impact**: Reduces memory for objects that are serialized without random access.

**Complexity**: Medium. Requires changes to OwnedValue and query evaluation.

---

### Phase M6: String Interning for Repeated Keys

**Goal**: Share memory for common key strings.

YAML configs often have repeated keys (`name`, `value`, `metadata`, `spec`, etc.). String interning would deduplicate these.

**Proposed:**
```rust
struct InternedString {
    data: Arc<str>,
}

// Or use a string interner crate
```

**Impact**: Significant for configs with many repeated keys.

**Complexity**: Medium-High. Requires integration throughout codebase.

---

## Implementation Priority

| Phase | Impact    | Complexity  | Priority                     |
|-------|-----------|-------------|------------------------------|
| M3    | Low       | Low         | **P1** (quick win)           |
| M4    | Medium    | Low-Medium  | **P1** (quick win)           |
| M1    | High      | Medium      | **P2** (major improvement)   |
| M2    | Very High | High        | **P3** (requires design work)|
| M5    | Medium    | Medium      | **P4** (nice to have)        |
| M6    | Medium    | Medium-High | **P4** (nice to have)        |

## Expected Results

After implementing M1-M4:

| File Size | Current | Expected | Improvement |
|-----------|---------|----------|-------------|
| 100KB     | 8 MB    | 4 MB     | 2×          |
| 1MB       | 12 MB   | 6 MB     | 2×          |
| 10MB      | 58 MB   | 25 MB    | 2.3×        |
| 100MB     | 491 MB  | 150 MB   | 3.3×        |

After implementing M2 (reference-based results for navigation):

| File Size           | Current | Expected | Improvement |
|---------------------|---------|----------|-------------|
| 100MB (`.`)         | 491 MB  | 115 MB   | 4.3×        |
| 100MB (`.users[0]`) | 491 MB  | 115 MB   | 4.3×        |

## Comparison with Fast Path

The fast path (identity + JSON compact) already achieves optimal memory:

| File Size | Fast Path    | Standard Path | Ratio |
|-----------|--------------|---------------|-------|
| 100KB     | 200 KB index | 8 MB          | 40×   |
| 1MB       | 2 MB index   | 12 MB         | 6×    |
| 10MB      | 20 MB index  | 58 MB         | 3×    |
| 100MB     | 200 MB index | 491 MB        | 2.5×  |

The goal is to bring standard path closer to fast path performance.

## Related Documents

- [yq.md](yq.md) - yq command implementation plan
- [../parsing/yaml.md](../parsing/yaml.md) - YAML parser optimizations
- [../benchmarks/yq.md](../benchmarks/yq.md) - Benchmark results

## Changelog

| Date       | Change                                |
|------------|---------------------------------------|
| 2026-01-24 | Initial analysis and optimization plan|
