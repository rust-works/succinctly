---
name: testing
description: Testing patterns and anti-patterns for this codebase. Use when writing tests, reviewing test coverage, or debugging test failures. Triggers on terms like "test", "assert", "coverage", "test suite", "regression".
---

# Testing Skill

Guidelines for writing effective tests that actually verify correctness.

## Critical Anti-Pattern: Success-Only Tests

**Never write tests that only check for successful execution without verifying output.**

### Bad Example (YAML test suite bug)

```rust
// BAD: Only checks parse success, not correctness
#[test]
fn test_sequence_of_mappings() {
    let yaml = b"- name: Alice\n  age: 30";
    let result = YamlIndex::build(yaml);
    assert!(result.is_ok());  // Parser could return completely wrong structure!
}
```

This test passed even when the parser produced `["name", "Alice", "age", 30]` instead of `[{"name": "Alice", "age": 30}]`.

### Good Example

```rust
// GOOD: Verifies actual output matches expected
#[test]
fn test_sequence_of_mappings() {
    let yaml = b"- name: Alice\n  age: 30";
    let index = YamlIndex::build(yaml).unwrap();
    let json = index.to_json(yaml);
    assert_eq!(json, r#"[{"name":"Alice","age":30}]"#);
}
```

## Test Generator Pitfalls

When generating tests from external sources (like official test suites):

### 1. Don't Skip Cases Based on Content

```python
# BAD: Skips tests containing quotes - misses most real-world cases
if '"' not in expected_output:
    generate_comparison_test()
else:
    generate_parse_only_test()  # No output verification!
```

```python
# GOOD: Properly escape and compare all cases
json_escaped = expected.replace('\\', '\\\\').replace('"', '\\"')
generate_comparison_test(json_escaped)
```

### 2. Count Actual Comparisons

After generating tests, verify how many actually compare output:
```bash
grep -c "assert_eq!" tests/generated_suite.rs
grep -c "is_ok()" tests/generated_suite.rs
```

If `is_ok()` count >> `assert_eq!` count, tests aren't verifying correctness.

### 3. Benchmark Generators Are Test Coverage Too

A shape that no generator emits is invisible to the entire benchmark suite, so "all benchmarks
neutral" is not evidence about it. In #106 every YAML generator emitted `- ` (dash-space), so
block-sequence items written as a bare `-` on its own line had **zero** coverage — and hid a
quadratic path worth up to 16x. The omission was justified by a stale comment claiming the parser
required dash-space, which was never true.

Before concluding a change is neutral, confirm a generator actually produces the shape it
touches. If not, add the pattern first (`src/bin/succinctly/yaml_generators.rs` and the
`YamlPattern` enum, plus both `yq_bench` pattern lists), then measure.

## Characterization Tests: Pinning Known-Wrong Behaviour

When you find a pre-existing defect that is out of scope to fix, lock in the current behaviour
with a test that says so explicitly, rather than leaving it undocumented:

```rust
#[test]
fn test_crlf_sequence_items_characterize_preexisting_bug() {
    // CRLF is mishandled independently of this change: `\r` is folded into plain
    // scalars as a trailing space, so `a: 1\r\n` yields the string "1 " not 1.
    // Verified identical before and after, so this is characterization of an
    // existing defect. If CRLF handling is fixed, update the expectation.
    let agreed = assert_seq_paths_agree_on(b"-\r\n  a: 1\r\n");
    assert_eq!(agreed, "[\"a\",\"1 \"]",
        "CRLF handling changed - if this is the fix, update this test");
}
```

Three requirements: name it `*_characterize_preexisting_bug` or similar so nobody mistakes it for
desired behaviour, state in the comment that before/after were verified identical, and tell the
future fixer to update it. The failure message should point at the fix, not the assertion.

This is distinct from a regression test — it asserts what the code *does*, not what it *should*
do, and is expected to fail when someone fixes the bug.

## Invariant Tests Over Duplicated Logic

When the same predicate exists at several call sites, add a test that the sites **agree with each
other**, not just that each is individually correct. #106 had five seq-item detection sites with
three different predicates; one was quadratic and no test noticed, because every test exercised
one path at a time. The permanent guard asserts `uncons`, `get(i)` and `uncons_cursor` produce
identical values for the same sequence, so re-divergence fails a test instead of shipping.

Pair this with a structural cross-check where one is available. For a predicate derived from the
input text, assert it never disagrees with the balanced-parens structure the parser built — that
oracle held across the whole real-workload corpus and all 279 valid YAML-suite cases when #106
added it, and it catches classes of bug that example-based tests cannot reach.

## Testing Levels

### Unit Tests (in-module)

- Test individual functions in isolation
- Place in `#[cfg(test)] mod tests` within the module
- Fast, focused, good for edge cases

### Integration Tests (tests/ directory)

- Test public API behavior
- Verify end-to-end correctness
- Compare against reference implementations or expected outputs

### Property Tests

For data structure invariants:
```rust
#[test]
fn property_rank_select_inverse() {
    // For all valid inputs, select(rank(x)) should return x
    for pos in 0..bitvec.len() {
        if bitvec.get(pos) {
            let r = bitvec.rank1(pos);
            assert_eq!(bitvec.select1(r), Some(pos));
        }
    }
}
```

## SIMD Testing Pattern

Always verify SIMD produces identical results to scalar reference:

```rust
#[test]
fn simd_matches_scalar() {
    let inputs = generate_test_inputs();
    for input in inputs {
        let scalar = scalar::process(&input);
        let simd = simd::process(&input);
        assert_eq!(scalar, simd, "Mismatch for input: {:?}", input);
    }
}
```

Test at SIMD boundaries (16, 32, 64 bytes) and boundary-1, boundary+1.

## Regression Test Workflow

When fixing a bug:

1. **First write a failing test** that reproduces the bug
2. Fix the bug
3. Verify the test passes
4. Run full test suite to check for regressions

```rust
#[test]
fn regression_issue_123_nested_sequences() {
    // This specific input caused incorrect output before fix
    let yaml = b"- - item";
    let index = YamlIndex::build(yaml).unwrap();
    let json = index.to_json(yaml);
    // Should be nested array, not flat
    assert_eq!(json, r#"[["item"]]"#);
}
```

## Test Naming

Use descriptive names that explain what's being tested:

```rust
// BAD
fn test_1() { ... }
fn test_parse() { ... }

// GOOD
fn test_nested_sequence_produces_nested_json() { ... }
fn test_multiline_scalar_preserves_newlines() { ... }
fn test_empty_mapping_returns_empty_object() { ... }
```

## Common Assertions

```rust
// Equality with message
assert_eq!(actual, expected, "Context: {}", debug_info);

// Pattern matching for Result
assert!(result.is_ok(), "Failed: {:?}", result.err());
let value = result.unwrap();

// Floating point (avoid direct equality)
assert!((actual - expected).abs() < 1e-10);

// Collection contents (order-independent)
assert_eq!(set1, set2);  // HashSet comparison
```

## See Also

- [tests/yaml_test_suite.rs](../../../tests/yaml_test_suite.rs) - Generated from official YAML test suite
- [tests/json_indexing_tests.rs](../../../tests/json_indexing_tests.rs) - JSON parser integration tests
- [tests/property_tests.rs](../../../tests/property_tests.rs) - Property-based tests for data structures
