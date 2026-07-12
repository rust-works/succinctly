---
name: yaml-semi-indexing
description: YAML semi-indexing implementation details and debugging patterns. Use when working on YAML parsing, semi-index construction, or cursor navigation. Triggers on terms like "YAML index", "YAML parser", "sequence item", "block sequence", "YAML cursor".
---

# YAML Semi-Indexing Skill

Implementation details for YAML semi-indexing using succinct data structures.

**Comprehensive documentation**: See [docs/parsing/yaml.md](../../../docs/parsing/yaml.md) for full parsing architecture.

## Semi-Index Structure

YAML uses more components than JSON due to its richer structure:

- **Interest Bits (IB)**: Marks structural positions (keys, values, items)
- **Balanced Parentheses (BP)**: Encodes tree structure for navigation
- **Type Bits (TY)**: Distinguishes mappings (0) from sequences (1)
- **Sequence Item Bits (seq_items)**: Marks BP positions that are sequence item wrappers

### Key Insight: Sequence Items vs Containers

Sequence items have BP open/close pairs but NO TY entry. This affects TY index calculations:

```rust
// WRONG: Direct rank gives incorrect TY index when seq_items exist
let ty_idx = bp.rank1(bp_pos);

// CORRECT: Subtract sequence items to get real container index
let bp_opens_before = bp.rank1(bp_pos);
let seq_items_before = count_seq_items_before(bp_pos);
let ty_idx = bp_opens_before.saturating_sub(seq_items_before);
```

## Block Sequence Parsing

### Multi-line Sequence Items

For YAML like:
```yaml
-
  name: Mark
  hr: 65
```

The sequence item must remain open so content on subsequent lines becomes the item's value:

1. Parse `-` at indent 0 → open sequence, open item, push `(indent+1, SequenceItem)` to stacks
2. `at_line_end()` is true → return (item stays open)
3. Next line `  name: Mark` at indent 2 → `parse_mapping_entry(2)`
4. Mapping opened inside the still-open item
5. Subsequent lines at same indent add to mapping
6. When indent returns to 0, `close_deeper_indents` closes mapping, item, etc.

### Inline Compact Mappings

For `- name: Mark\n  hr: 65`:

1. Parse `- ` → open sequence, open item
2. `looks_like_mapping_entry()` is true → call `parse_compact_mapping_entry(indent+2)`
3. Mapping opened but NOT closed after first entry
4. Item also NOT closed
5. Next line `hr: 65` at indent 2 adds to the same mapping

**Critical**: Don't close compact mappings eagerly. Let `close_deeper_indents` handle it.

### Nested Sequences

For `- - item`:

Check for nested sequence BEFORE checking for mapping:
```rust
if self.peek() == Some(b'-') && matches!(self.peek_at(1), Some(b' ') | ...) {
    // Nested sequence - recurse with indent+2
    self.parse_sequence_item(indent + 2)?;
} else if self.looks_like_mapping_entry() {
    // Compact mapping
}
```

## Common Debugging Patterns

### Tracing BP Structure

```rust
for bp_pos in 0..30 {
    let is_open = index.bp().is_open(bp_pos);
    if is_open {
        if let Some(text_pos) = index.bp_to_text_pos(bp_pos) {
            let is_seq_item = index.is_seq_item(bp_pos);
            println!("BP[{}] = OPEN at text[{}] seq_item={}", bp_pos, text_pos, is_seq_item);
        }
    } else {
        println!("BP[{}] = CLOSE", bp_pos);
    }
}
```

### Checking Stack State

When debugging incorrect structure, trace indent_stack and type_stack:
- After each `parse_*` call, verify stacks have expected entries
- `close_deeper_indents` should be called BEFORE checking `need_new_sequence/mapping`

### Order of Operations Bug

**Wrong**:
```rust
let need_new = type_stack.last() != expected;  // Check first
if need_new {
    close_deeper_indents(indent);  // Then close
    // But need_new was computed with OLD stack state!
}
```

**Correct**:
```rust
close_deeper_indents(indent);  // Close first
let need_new = type_stack.last() != expected;  // Then check
```

## Test Suite Notes

The YAML test suite (`tests/yaml_test_suite.rs`) is generated from the official YAML test suite.

**Important**: Tests must compare JSON output, not just parse success. A test that only checks `result.is_ok()` doesn't verify correctness.

When regenerating tests, ensure the generator properly escapes JSON for Rust string literals:
```python
json_escaped = json_normalized.replace('\\', '\\\\').replace('"', '\\"')
```

## See Also

- [docs/parsing/yaml.md](../../../docs/parsing/yaml.md) - Full YAML parsing documentation
- [src/yaml/parser.rs](../../../src/yaml/parser.rs) - Parser implementation
- [src/yaml/light.rs](../../../src/yaml/light.rs) - Cursor and value extraction
- [src/yaml/index.rs](../../../src/yaml/index.rs) - Index structure and TY calculations
