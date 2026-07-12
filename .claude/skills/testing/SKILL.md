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
