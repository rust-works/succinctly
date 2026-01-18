# Plan: jq Query Language Completeness

## Overview

This document tracks all remaining work to achieve full jq query language compatibility.
The implementation is already production-ready for ~90% of jq use cases.

## Implementation Status Summary

| Category            | Status              | Coverage |
|---------------------|---------------------|----------|
| Core path expressions | Fully implemented | 100%     |
| Operators           | Fully implemented   | 100%     |
| Type functions      | Fully implemented   | 100%     |
| Array operations    | Fully implemented   | 95%      |
| Object operations   | Fully implemented   | 100%     |
| String functions    | Fully implemented   | 95%      |
| Control flow        | Fully implemented   | 90%      |
| Math functions      | Fully implemented   | 100%     |
| Path operations     | Fully implemented   | 90%      |
| Format strings      | Fully implemented   | 90%      |
| Variable binding    | Fully implemented   | 95%      |
| User functions      | Fully implemented   | 85%      |
| Regex functions     | Fully implemented   | 100%     |
| Module system       | Parsed only         | 10%      |
| I/O operations      | Limited             | 40%      |
| Assignment operators| Fully implemented   | 100%     |

---

## Fully Implemented Features ✅

### Core Path Expressions
- [x] `.` - Identity
- [x] `.foo` - Field access
- [x] `.[0]` - Array index (positive and negative)
- [x] `.[2:5]`, `.[2:]`, `.[:5]` - Array slicing
- [x] `.[]` - Array/object iteration
- [x] `.foo?` - Optional access
- [x] `.foo.bar[0]` - Chained access
- [x] `..` - Recursive descent

### Operators
- [x] Arithmetic: `+`, `-`, `*`, `/`, `%`
- [x] Comparison: `==`, `!=`, `<`, `<=`, `>`, `>=`
- [x] Boolean: `and`, `or`, `not`
- [x] Alternative: `//`
- [x] Pipe: `|`
- [x] Comma: `,`

### Type Functions
- [x] `type` - Returns type name
- [x] `isnull`, `isboolean`, `isnumber`, `isstring`, `isarray`, `isobject`

### Type Filters
- [x] `values` - Select non-null values
- [x] `nulls` - Select only null values
- [x] `booleans` - Select only boolean values
- [x] `numbers` - Select only number values
- [x] `strings` - Select only string values
- [x] `arrays` - Select only array values
- [x] `objects` - Select only object values
- [x] `iterables` - Select arrays and objects
- [x] `scalars` - Select non-iterables (null, bool, number, string)

### Selection & Filtering
- [x] `select(cond)` - Filter by condition
- [x] `empty` - Output nothing
- [x] `if-then-else` with `elif` support
- [x] `try-catch` - Error handling
- [x] `error` / `error(msg)` - Raise errors

### Object Operations
- [x] `keys` / `keys_unsorted`
- [x] `has(key)`
- [x] `in(obj)`
- [x] `to_entries` / `from_entries` / `with_entries(f)`
- [x] `pick(keys)` - select only specified keys (yq)
- [x] Object construction: `{foo: .bar}`, `{(expr): value}`, shorthand `{foo}`

### Array Operations
- [x] `length`
- [x] `first` / `last` / `nth(n)`
- [x] `reverse`
- [x] `flatten` / `flatten(depth)`
- [x] `sort` / `sort_by(f)`
- [x] `unique` / `unique_by(f)`
- [x] `group_by(f)`
- [x] `add`
- [x] `min` / `max` / `min_by(f)` / `max_by(f)`
- [x] `transpose`
- [x] `bsearch(x)`

### String Functions
- [x] `ascii_downcase` / `ascii_upcase`
- [x] `ltrimstr(s)` / `rtrimstr(s)`
- [x] `ltrim` / `rtrim` / `trim`
- [x] `startswith(s)` / `endswith(s)`
- [x] `split(s)` / `join(s)`
- [x] `contains(x)` / `inside(x)`
- [x] `tostring` / `tonumber`
- [x] `explode` / `implode`
- [x] `utf8bytelength`
- [x] `indices(s)` / `index(s)` / `rindex(s)`
- [x] `test(re)` (substring without regex feature)

### Regular Expressions (with `regex` feature)
- [x] `match(re)` / `match(re; flags)`
- [x] `capture(re)`
- [x] `scan(re)`
- [x] `splits(re)`
- [x] `sub(re; replacement)` / `gsub(re; replacement)`

### Format Strings
- [x] `@text` - Convert to string
- [x] `@json` - JSON encoding
- [x] `@csv` / `@tsv` - Delimited formats
- [x] `@dsv(delimiter)` - Custom delimiter with smart quoting
- [x] `@base64` / `@base64d`
- [x] `@uri` - Percent encoding
- [x] `@html` - HTML entity escaping
- [x] `@sh` - Shell quoting

### Variables & Control Flow
- [x] `as $var | expr` - Variable binding
- [x] Object/array destructuring patterns
- [x] `reduce expr as $x (init; update)`
- [x] `foreach expr as $x (init; update)` / `foreach ... (init; update; extract)`

### Advanced Control Flow
- [x] `limit(n; expr)`
- [x] `first` / `first(expr)` / `last` / `last(expr)`
- [x] `nth(n; expr)`
- [x] `until(cond; update)` / `while(cond; update)`
- [x] `repeat(expr)`
- [x] `range(n)` / `range(a;b)` / `range(a;b;step)`

### Path Operations
- [x] `path(expr)`
- [x] `path` (no-arg, yq) - returns current traversal path
- [x] `paths` / `paths(filter)` / `leaf_paths`
- [x] `getpath(path)` / `setpath(path; value)`
- [x] `delpaths(paths)` / `del(path)`
- [x] `parent` (yq) - returns parent node of current position
- [x] `parent(n)` (yq) - returns nth parent node

### Math Functions (32 total)
- [x] Basic: `floor`, `ceil`, `round`, `sqrt`, `fabs`
- [x] Exponential: `log`, `log10`, `log2`, `exp`, `exp10`, `exp2`
- [x] Trigonometric: `sin`, `cos`, `tan`, `asin`, `acos`, `atan`
- [x] 2-arg: `pow(x; y)`, `atan2(y; x)`
- [x] Hyperbolic: `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh`
- [x] Special: `infinite`, `nan`, `isinfinite`, `isnan`, `isnormal`, `isfinite`

### I/O & Debug
- [x] `debug` / `debug(msg)`
- [x] `env`, `$ENV.VAR`, `env(VAR)`, `strenv(VAR)`

### Assignment Operators
- [x] `.a = value` - Simple assignment
- [x] `.a |= f` - Update assignment
- [x] `.a += value`, `-=`, `*=`, `/=`, `%=` - Compound assignment
- [x] `.a //= value` - Alternative assignment
- [x] `del(.a)` - Delete path

### User Functions
- [x] `def name: body;`
- [x] `def name(args): body;`
- [x] Recursive function calls
- [x] String interpolation: `"Hello \(.name)"`

### Other
- [x] `any` / `all`
- [x] `recurse` / `recurse(f)` / `recurse(f; cond)`
- [x] `walk(f)`
- [x] `isvalid(expr)`
- [x] `modulemeta(name)` (stub)
- [x] `tojsonstream` / `fromjsonstream`
- [x] `map(f)` / `map_values(f)`

---

## TODO: Missing Features

### Priority 1: JSON String Conversion

- [ ] `tojson` - Convert value to JSON string
- [ ] `fromjson` - Parse JSON string to value

**Implementation notes:**
- `tojson` serializes a value to a JSON string (like `@json` but returns string)
- `fromjson` parses a JSON string into a value
- Useful for working with JSON embedded in strings

### Priority 2: I/O Operations

- [ ] `input` - Read next input from stdin/files
- [ ] `inputs` - Stream all remaining inputs
- [ ] `input_line_number` - Current line number

**Implementation notes:**
- Requires CLI integration to pass input iterator to evaluator
- Consider adding `EvalContext` struct to hold input state
- Low priority for single-file JSON processing use case

### Priority 3: Module System

Currently parsed but not evaluated:

- [ ] `import "path" as name;` - Import module
- [ ] `include "path";` - Include module inline
- [ ] `module {...}` - Module metadata

**Implementation notes:**
- Requires file system access
- Need module resolution strategy (relative paths, library paths)
- Consider `jq -L` style library paths
- May want to keep this optional/feature-gated

### Priority 4: Location & Debugging

- [ ] `$__loc__` - Current source location `{file, line}`
- [ ] Comments in jq expressions (`#` to end of line)

**Implementation notes:**
- `$__loc__` requires tracking source positions through parsing
- Comment support needs lexer changes

### Priority 5: Advanced Features

- [ ] Label-break: `label $name | ... | break $name`
- [ ] Array slicing with steps: `.[::2]` (every other element)
- [ ] `ascii` - ASCII character code (e.g., `"A" | ascii` → 65)
- [ ] `now` - Current Unix timestamp

### Priority 6: CLI Enhancements

These are CLI-level features, not expression language:

- [ ] `-s` / `--slurp` - Read all inputs into array
- [ ] `-R` / `--raw-input` - Read lines as strings
- [ ] `-n` / `--null-input` - Don't read input, use null
- [ ] `--tab` - Use tabs for indentation
- [ ] `--argjson` / `--slurpfile` / `--rawfile` enhancements

---

## Known Limitations

### Intentionally Not Implemented

1. **SQL-style operators** - Not in standard jq
2. **Multi-precision integers** - Uses Rust's i64/f64
3. **Streaming across multiple files** - Single-file focus
4. **Full jq module library** - Just core builtins

### Partial Implementation Notes

1. **Variable scoping** - May not perfectly match jq edge cases
2. **Error messages** - Don't always match jq format exactly
3. **Numeric overflow** - Uses wrapping arithmetic
4. **`$ENV` as bare object** - Only field access works (`$ENV.VAR`)

---

## Testing Strategy

### Compatibility Tests

Run against canonical jq to verify:

```bash
# Compare output
echo '{"a":1}' | jq '.a'
echo '{"a":1}' | succinctly jq '.a'

# Diff test script
./scripts/test-jq-compat.sh
```

### Priority Test Cases

1. Type filters (`values`, `nulls`, `strings`, etc.)
2. JSON string conversion (`tojson`, `fromjson`)
3. Complex path expressions
4. Error message format
5. Edge cases in arithmetic/comparison

---

## Changelog

| Date       | Change                                    |
|------------|-------------------------------------------|
| 2025-01-19 | Initial document created from audit       |
| 2025-01-19 | Added assignment operators (✅ complete)  |
| 2025-01-19 | Added env variable access (✅ complete)   |
| 2026-01-19 | Added pick() function for yq (✅ complete)|
| 2026-01-19 | Added path (no-arg) for yq (✅ complete)  |
| 2026-01-19 | Added parent / parent(n) for yq (✅ complete)|
| 2026-01-19 | Added type filters: values, nulls, booleans, numbers, strings, arrays, objects, iterables, scalars (✅ complete)|
