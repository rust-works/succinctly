# M2 Benchmark Improvements Plan

This document proposes enhancements to the yq benchmark suite to properly measure the M2 streaming optimization's impact.

## Problem Statement

The current yq benchmark (`succinctly bench run yq_bench`) only tests identity queries (`.`), which use the original P9 streaming fast path. This doesn't exercise the **M2 streaming path** for navigation queries.

**Current benchmark query:**
```bash
succinctly yq -o json -I 0 '.' file.yaml
```

**What M2 optimizes:**
- Field access: `.users`, `.config.database`
- Index access: `.[0]`, `.users[5]`
- Iteration: `.[]`, `.users[]`
- Chained navigation: `.users[0].name`
- First/Last builtins: `first`, `last`

## Measured Impact (Manual Testing)

Using a 100MB JSON file:

| Query | Path | Peak Memory | Time |
|-------|------|-------------|------|
| `.` (identity) | P9 streaming | 549 MB | 1.29s |
| `.[0]` (navigation) | M2 streaming | 549 MB | 0.43s |
| `length` (builtin) | OwnedValue | 1.95 GB | 4.46s |

**Key insight**: M2 streaming uses **3.5× less memory** than the OwnedValue path.

## Proposed Benchmark Enhancements

### Phase 1: Add Navigation Query Benchmarks

Add new benchmark queries to exercise M2:

```rust
// In yq_bench.rs

const BENCHMARK_QUERIES: &[(&str, &str)] = &[
    ("identity", "."),
    ("first_element", ".[0]"),
    ("field_access", ".users"),       // requires users field in test data
    ("iteration", ".[]"),
    ("chained", ".[0].name"),         // requires nested structure
    ("length", "length"),             // OwnedValue path for comparison
];
```

### Phase 2: Generate Appropriate Test Data

Current test data generators should be verified to produce data suitable for these queries:

- `comprehensive`: Has nested objects and arrays ✓
- `nested`: Deeply nested structures ✓
- `sequences`: Array-heavy data ✓

Consider adding a `navigation` pattern specifically designed for M2 testing:
```yaml
users:
  - name: Alice
    age: 30
    email: alice@example.com
  - name: Bob
    age: 25
    email: bob@example.com
config:
  database:
    host: localhost
    port: 5432
```

### Phase 3: Memory-Focused Benchmark Mode

**IMPLEMENTED**: Memory is now collected by default. Use `--no-memory` to skip:

```bash
# Memory collected by default
succinctly bench run yq_bench

# Skip memory collection (faster)
succinctly bench run yq_bench --no-memory
```

This:
1. Runs each query type on the same file
2. Compares memory usage between streaming and non-streaming paths
3. Outputs a memory comparison table (markdown includes memory columns by default)

### Phase 4: Document M2 Impact

Update `docs/benchmarks/yq.md` to include:

1. **Query type comparison table** showing which queries use streaming
2. **Memory comparison** between streaming and OwnedValue paths
3. **Guidance** on which query patterns benefit most from M2

## Implementation Priority

| Phase | Effort | Impact | Priority |
|-------|--------|--------|----------|
| Phase 1 | Low | High | P1 |
| Phase 2 | Low | Medium | P2 |
| Phase 3 | Medium | Medium | P3 |
| Phase 4 | Low | High | P1 |

## Expected Outcome

After implementation, the benchmark output should show:

```
Running yq benchmark suite...
  Query: . (identity, P9 streaming)
    comprehensive 100mb... succ=1083.9ms mem=549MB

  Query: .[0] (navigation, M2 streaming)
    comprehensive 100mb... succ=430.0ms mem=549MB

  Query: length (builtin, OwnedValue)
    comprehensive 100mb... succ=4460.0ms mem=1950MB
```

This clearly demonstrates the M2 optimization's memory benefits.

## Files to Modify

- `src/bin/succinctly/yq_bench.rs`: Add query variants
- `docs/benchmarks/yq.md`: Document M2 impact
- `CLAUDE.md`: Update benchmark section with M2 context

## Related

- [yq-memory-optimization.md](yq-memory-optimization.md): Original M2 design
- M2 implementation: `src/jq/stream.rs`, `src/jq/eval_generic.rs`, `yq_runner.rs`
