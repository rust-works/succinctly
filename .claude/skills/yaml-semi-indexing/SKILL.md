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
- **Sequence item wrappers**: derived from text, not stored. A non-container node whose
  byte is `-` followed by whitespace or end-of-input. See `starts_seq_entry` in `src/yaml/mod.rs`
  (the single definition -- O4 removed the `seq_items` bitvector, #332 and O6 consolidated the
  five call sites onto it). Never re-spell this predicate at a call site: a narrower copy in
  `uncons_cursor` made `-\n` items re-read their own open index, which `AdvancePositions` treats
  as a backwards jump, resetting the sequential IB cursor and going quadratic

### Key Insight: Sequence Items vs Containers

Sequence items have BP open/close pairs but NO TY entry. This affects TY index calculations:

```rust
// WRONG: Direct rank gives incorrect TY index when seq-item wrappers exist
let ty_idx = bp.rank1(bp_pos);

// CORRECT: Count containers, which is what TY is indexed by
let ty_idx = index.count_containers_before(bp_pos);
```

`count_seq_items_before()` and the `seq_items` bitvector no longer exist (O4). The
`containers` bitvector plus `containers_rank` is the surviving per-node structure.

Note wrapper emission is **context-dependent**: the parser emits a wrapper only for items
with structured content, so not every child of a block sequence is a wrapper. Never cache
wrapper-ness across a sequence (O6).

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

### Never Test for `b'\n'` by Hand

YAML 1.2 §5.4 has three line breaks — `\n`, `\r\n`, and a lone `\r` — and §5.7
requires normalizing all of them to `\n`. A raw `\r` is never scalar content, so
a scan that stops only at `\n` is a bug in two distinct ways (#324):

- **CRLF**: the scan still stops on the right *line*, so nothing looks broken — it
  just leaves the `\r` inside the preceding token's extent. The plain-scalar
  folding decoder then turns that trailing break into a space, and `a: 1` resolves
  as the string `"1 "` instead of the number `1`. Silent and well-formed.
- **Lone CR**: there is no `\n` to stop on, so the scan runs to EOF and swallows
  the rest of the document.

Use the helpers instead of hand-rolling the test:

```rust
// src/yaml/line_break.rs — the rule, defined once, for every consumer (#341)
is_line_break(b)                // \n or \r
line_break_len(text, pos)       // 2 for \r\n, 1 for a lone \r or \n, 0 otherwise

// src/yaml/parser.rs — the oracle, where the HAS_CR gate lives (#340)
Self::is_break(b)               // is_line_break, or `== b'\n'` under !HAS_CR
self.break_len_at(pos)          // line_break_len, or the LF answer under !HAS_CR
Self::is_ws_or_break(b)         // ' ' | '\t' | break
Self::is_ws_break_or_eoi(next)  // the indicator lookahead: Option<u8>, EOI counts
self.at_break()
self.skip_line_break()          // consumes exactly one break, whatever its width
```

Inside the oracle prefer the `Self::`/`self.` forms over the free functions: they
are the same rule, but they are also where `HAS_CR` switches. Calling
`is_line_break` directly from a parser hot path silently opts that site out of the
specialization — correct, just slower. `current_line` does exactly that on
purpose, being an error path.

Going through a helper at all is not just tidiness. A hand-rolled
`matches!(b, b'\n' | b'\r')` is a site the const generic cannot reach.

Two extra traps:

- **Trailing-whitespace trims** must include `\r`, not just `' '` and `'\t'` — the
  SIMD classifier can skip a CR to land on the LF after it.
- **SIMD scans** need `\r` in their terminator set (`YamlCharClass::carriage_returns`,
  `find_block_scalar_end`), or a 32-byte chunk steps straight over a lone CR.

`tests/yaml_crlf_tests.rs` is the guard: it parses the whole YAML Test Suite
corpus under all three break forms and demands identical output. Run it after any
change to a line-scanning loop.

### The `HAS_CR` Specialization

`Parser<'a, const HAS_CR: bool>` (#340). `build_semi_index` runs `contains_cr`
over the input once — a SIMD byte scan — and picks `Parser::<false>` for the
LF-only documents that are nearly all of them, which compiles every `\r` arm out
and restores the pre-#324 codegen. `Parser::<true>` is the #324 parser verbatim.

The rule to internalize when editing a line scan:

- **`HAS_CR == true` is always correct.** Only `false` carries an obligation, and
  the whole-input precheck discharges it: no `\r` in the input means no `\r` arm
  is reachable, in any context. There is no quoted-string or block-scalar
  subtlety, because one `\r` anywhere forces the `true` path.
- **So gating is a performance knob, not a correctness one.** Leaving a new site
  un-gated is a missed optimization, never a bug. Gate the hot ones; don't
  contort cold code to reach them.
- **Match arms cannot be gated.** `b'\n' | b'\r' =>` is a pattern, and a const
  cannot reach into it. The hot loops use `b if Self::is_break(b) =>` guards
  instead. Cold sites keep the literal pattern deliberately.

`both_monomorphizations_agree_on_cr_free_input` in `src/yaml/parser.rs` is the
guard, and it is the only one: `yaml_crlf_tests` and the CRLF reruns in
`yq_golden_tests` all feed inputs containing a `\r`, which is exactly the set
that takes the `true` path. If you gate a site whose `\r` arm was doing something
other than handling a carriage return, that test is what catches it.

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
