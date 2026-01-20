# Plan: Generic jq Evaluator over DocumentValue

## Overview

Refactor the jq evaluator (`src/jq/eval.rs`) to be generic over `DocumentValue`/`DocumentCursor` traits, enabling direct YAML evaluation without intermediate YAML→OwnedValue→JSON→JsonIndex conversion.

## Goals

1. **Correct `line`/`column` metadata**: Position information preserved from source YAML
2. **Performance improvement**: Eliminate intermediate DOM conversion (~2x potential, similar to P9)
3. **Unified evaluation**: Same evaluator works for both JSON and YAML inputs
4. **Full yq compatibility**: Metadata functions (`anchor`, `alias`, `style`, `line`, `column`) return actual values

## Current State

### Evaluation Chain (yq)

```
YAML bytes → YamlIndex → YamlCursor → YamlValue → OwnedValue → JSON string → JsonIndex → JsonCursor → eval
                                                      ↑
                                              Position metadata lost here
```

### Target Chain

```
YAML bytes → YamlIndex → YamlCursor → YamlValue → eval (generic)
                                          ↑
                                   Position metadata preserved
```

### Dependencies (eval.rs: 15,763 lines)

| Type | Occurrences | Purpose |
|------|-------------|---------|
| `StandardJson` | 404 | Value type in pattern matching |
| `JsonCursor` | 4 | Cursor in `QueryResult::OneCursor` |
| `to_owned()` | 210 | Convert to OwnedValue |

## Performance Baseline

### Pre-refactor Benchmarks

Run before starting refactor to establish baseline:

```bash
# yq comparison benchmark
cargo bench --bench yq_comparison 2>&1 | tee .ai/scratch/baseline-yq-comparison.txt

# jq comparison benchmark (ensure JSON path not regressed)
cargo bench --bench jq_comparison 2>&1 | tee .ai/scratch/baseline-jq-comparison.txt

# Full benchmark suite for reference
cargo bench 2>&1 | tee .ai/scratch/baseline-full-bench.txt
```

Expected baseline (Apple M1 Max from docs):

| Size | succinctly yq | yq | Speedup |
|------|---------------|-----|---------|
| 10KB | 83.5 µs (116.8 MiB/s) | 57.4 ms | 687x |
| 100KB | 599 µs (153.6 MiB/s) | 71.3 ms | 119x |
| 1MB | 4.24 ms (217.6 MiB/s) | 191.1 ms | 45x |

### Post-refactor Target

With direct YAML evaluation (similar to P9's 2.3x gain):

| Size | Target | Improvement |
|------|--------|-------------|
| 10KB | ~40 µs (~240 MiB/s) | ~2x |
| 100KB | ~300 µs (~300 MiB/s) | ~2x |
| 1MB | ~2 ms (~460 MiB/s) | ~2x |

## Implementation Phases

### Phase 1: Add Helper Methods to DocumentValue Trait

**File**: `src/jq/document.rs`

Add convenience methods for type checking:

```rust
pub trait DocumentValue: Sized + Clone {
    // ... existing methods ...

    /// Check if this is a boolean value.
    fn is_bool(&self) -> bool { self.as_bool().is_some() }

    /// Check if this is a number value.
    fn is_number(&self) -> bool { self.as_i64().is_some() || self.as_f64().is_some() }

    /// Check if this is a string value.
    fn is_string(&self) -> bool { self.as_str().is_some() }

    /// Check if this is an array value.
    fn is_array(&self) -> bool { self.as_array().is_some() }

    /// Check if this is an object value.
    fn is_object(&self) -> bool { self.as_object().is_some() }

    /// Check if this is iterable (array or object).
    fn is_iterable(&self) -> bool { self.is_array() || self.is_object() }
}
```

**Verification**: Compile and run existing tests.

### Phase 2: Add Position Methods to DocumentCursor Trait

**File**: `src/jq/document.rs`

```rust
pub trait DocumentCursor: Sized + Copy + Clone {
    // ... existing methods ...

    /// Get the 1-based line number of this node's position.
    fn line(&self) -> usize { 0 }

    /// Get the 1-based column number of this node's position.
    fn column(&self) -> usize { 0 }
}
```

**Files to update**:
- `src/json/light.rs`: Implement for `JsonCursor` (return 0, JSON doesn't track position)
- `src/yaml/light.rs`: Implement for `YamlCursor` (delegate to existing methods)

**Verification**: Compile and run existing tests.

### Phase 3: Create Generic to_owned Function

**File**: `src/jq/eval.rs`

Create generic version alongside existing:

```rust
fn to_owned_generic<V: DocumentValue>(value: &V) -> OwnedValue {
    if value.is_null() {
        OwnedValue::Null
    } else if let Some(b) = value.as_bool() {
        OwnedValue::Bool(b)
    } else if let Some(i) = value.as_i64() {
        OwnedValue::Int(i)
    } else if let Some(f) = value.as_f64() {
        OwnedValue::Float(f)
    } else if let Some(s) = value.as_str() {
        OwnedValue::String(s.into_owned())
    } else if let Some(elements) = value.as_array() {
        let mut items = Vec::new();
        let mut elems = elements;
        while let Some((elem, rest)) = elems.uncons() {
            items.push(to_owned_generic(&elem));
            elems = rest;
        }
        OwnedValue::Array(items)
    } else if let Some(fields) = value.as_object() {
        let mut map = IndexMap::new();
        let mut f = fields;
        while let Some((field, rest)) = f.uncons() {
            if let Some(key) = field.key_str() {
                map.insert(key.into_owned(), to_owned_generic(&field.value));
            }
            f = rest;
        }
        OwnedValue::Object(map)
    } else {
        OwnedValue::Null
    }
}
```

**Verification**: Add test comparing `to_owned` vs `to_owned_generic` for same JSON input.

### Phase 4: Make QueryResult Generic

**File**: `src/jq/eval.rs`

```rust
pub enum QueryResult<V: DocumentValue> {
    /// Single value result (reference to original document).
    One(V),

    /// Single cursor result (for unchanged container values).
    OneCursor(V::Cursor),

    /// Multiple values (from iteration).
    Many(Vec<V>),

    /// No result (optional that was missing).
    None,

    /// Error during evaluation.
    Error(EvalError),

    /// Single owned value (from construction/computation).
    Owned(OwnedValue),

    /// Multiple owned values.
    ManyOwned(Vec<OwnedValue>),

    /// Break from a labeled scope.
    Break(String),
}
```

**Note**: This is the most disruptive change. Consider creating `QueryResultGeneric<V>` first, then renaming once working.

**Verification**: Compile incrementally.

### Phase 5: Convert Core Evaluation Functions

Convert in order of dependency:

1. **`type_name`** → Use `value.type_name()` directly
2. **`eval_single`** → Main evaluation dispatch
3. **`eval_builtin`** → Builtin dispatch
4. **Helper functions**: `find_field`, `get_element_at_index`, etc.

Pattern conversions:

| Current | Generic |
|---------|---------|
| `StandardJson::Null` | `value.is_null()` |
| `StandardJson::Bool(b)` | `if let Some(b) = value.as_bool()` |
| `StandardJson::Number(n)` | `if let Some(i) = value.as_i64()` or `as_f64()` |
| `StandardJson::String(s)` | `if let Some(s) = value.as_str()` |
| `StandardJson::Array(elements)` | `if let Some(elements) = value.as_array()` |
| `StandardJson::Object(fields)` | `if let Some(fields) = value.as_object()` |
| `matches!(v, StandardJson::Bool(_))` | `value.is_bool()` |

**Verification**: Run `cargo test` after each function conversion.

### Phase 6: Convert Builtin Functions

Convert `builtin_*` functions (~50 functions). Group by complexity:

**Simple (no iteration)**:
- `builtin_type`, `builtin_isnull`, `builtin_isnumber`, etc.
- `builtin_tostring`, `builtin_tonumber`
- `builtin_line`, `builtin_column` (update to use cursor position)

**Medium (single iteration)**:
- `builtin_length`, `builtin_keys`, `builtin_values`
- `builtin_first`, `builtin_last`, `builtin_nth`

**Complex (nested iteration/evaluation)**:
- `builtin_map`, `builtin_select`, `builtin_group_by`
- `builtin_sort_by`, `builtin_unique_by`

**Verification**: Run tests after each group.

### Phase 7: Update line/column Builtins

**File**: `src/jq/eval.rs`

Update to use cursor position when available:

```rust
fn builtin_line<V: DocumentValue>(cursor: Option<V::Cursor>) -> QueryResult<V> {
    let line = cursor.map(|c| c.line()).unwrap_or(0);
    QueryResult::Owned(OwnedValue::Int(line as i64))
}

fn builtin_column<V: DocumentValue>(cursor: Option<V::Cursor>) -> QueryResult<V> {
    let column = cursor.map(|c| c.column()).unwrap_or(0);
    QueryResult::Owned(OwnedValue::Int(column as i64))
}
```

**Note**: This requires threading cursor information through evaluation. May need to store cursor alongside value in `QueryResult::One`.

**Verification**: Add YAML tests verifying correct line/column values.

### Phase 8: Create YAML Entry Point

**File**: `src/bin/succinctly/yq_runner.rs`

Create new evaluation path that uses YAML cursor directly:

```rust
pub fn evaluate_yaml_direct<'a>(
    yaml: &'a [u8],
    index: &'a YamlIndex<Vec<u64>>,
    expr: &Expr,
) -> Result<Vec<QueryResult<YamlValue<'a, Vec<u64>>>>, EvalError> {
    let cursor = index.root(yaml);
    let value = cursor.value();
    eval_generic(expr, value)
}
```

**Verification**: Run yq CLI with both paths, compare output.

### Phase 9: Performance Validation

Run benchmarks and compare:

```bash
# Post-refactor benchmarks
cargo bench --bench yq_comparison 2>&1 | tee .ai/scratch/post-refactor-yq-comparison.txt

cargo bench --bench jq_comparison 2>&1 | tee .ai/scratch/post-refactor-jq-comparison.txt

# Compare
diff .ai/scratch/baseline-yq-comparison.txt .ai/scratch/post-refactor-yq-comparison.txt
```

**Success criteria**:
- yq performance: ≥1.5x improvement (target 2x)
- jq performance: No regression (±5%)
- All existing tests pass
- `line`/`column` return correct values for YAML

### Phase 10: Cleanup and Documentation

1. Remove old JSON-specific code paths (if fully replaced)
2. Update documentation in `docs/plan/jq-completeness.md`
3. Add benchmark results to `docs/benchmarks/yq.md`
4. Update CLAUDE.md with new architecture notes

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Large refactor scope | Incremental phases with tests after each |
| Performance regression | Benchmark baseline, validate each phase |
| Breaking existing JSON path | Keep old code until generic version verified |
| Complex generics | Start with simple functions, build up |

## Rollback Plan

Each phase is independently testable. If issues arise:
1. Revert phase-specific changes
2. Keep both generic and JSON-specific versions temporarily
3. Use feature flag to select implementation

## Estimated Scope

| Phase | Lines Changed | Risk |
|-------|---------------|------|
| 1-2 | ~50 | Low |
| 3 | ~100 | Low |
| 4 | ~200 | Medium |
| 5 | ~1000 | Medium |
| 6 | ~2000 | Medium |
| 7 | ~100 | Low |
| 8 | ~200 | Low |
| 9-10 | ~100 | Low |
| **Total** | ~3750 | Medium |

## Success Metrics

1. **Correctness**: All existing tests pass
2. **Metadata**: `line`/`column` return actual values for YAML (not 0)
3. **Performance**: yq ≥1.5x faster, jq no regression
4. **Code quality**: Reduced duplication between JSON/YAML paths

## Changelog

| Date | Change |
|------|--------|
| 2026-01-20 | Initial plan created based on eval.rs analysis |
