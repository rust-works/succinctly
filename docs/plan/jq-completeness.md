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
| Control flow        | Fully implemented   | 100%     |
| Math functions      | Fully implemented   | 100%     |
| Path operations     | Fully implemented   | 90%      |
| Format strings      | Fully implemented   | 90%      |
| Variable binding    | Fully implemented   | 95%      |
| User functions      | Fully implemented   | 85%      |
| Regex functions     | Fully implemented   | 100%     |
| Module system       | Fully implemented   | 95%      |
| I/O operations      | Won't implement     | N/A      |
| Assignment operators| Fully implemented   | 100%     |

---

## Fully Implemented Features ✅

### Core Path Expressions
- [x] `.` - Identity
- [x] `.foo` - Field access
- [x] `."key"` - Quoted field access (for special characters like kebab-case)
- [x] `.["key"]` - Bracket notation with string key
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
- [x] `toboolean` - Convert to boolean (accepts true, false, "true", "false")

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
- [x] `normals` - Select only normal numbers (not 0, infinite, NaN, or subnormal)
- [x] `finites` - Select only finite numbers (not infinite or NaN)

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
- [x] `tojson` / `fromjson` - JSON string conversion
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
- [x] `@uri` / `@urid` - Percent encoding / decoding
- [x] `@html` - HTML entity escaping
- [x] `@sh` - Shell quoting

### Variables & Control Flow
- [x] `as $var | expr` - Variable binding
- [x] Object/array destructuring patterns
- [x] `reduce expr as $x (init; update)`
- [x] `foreach expr as $x (init; update)` / `foreach ... (init; update; extract)`

### Advanced Control Flow
- [x] `limit(n; expr)`
- [x] `skip(n; expr)` - skip first n outputs from expr
- [x] `first` / `first(expr)` / `last` / `last(expr)`
- [x] `nth(n; expr)`
- [x] `until(cond; update)` / `while(cond; update)`
- [x] `repeat(expr)`
- [x] `range(n)` / `range(a;b)` / `range(a;b;step)`
- [x] `combinations` / `combinations(n)` - Cartesian product of arrays
- [x] `label $name | expr` / `break $name` - non-local control flow

### Path Operations
- [x] `path(expr)`
- [x] `path` (no-arg, yq) - returns current traversal path
- [x] `paths` / `paths(filter)` / `leaf_paths`
- [x] `getpath(path)` / `setpath(path; value)`
- [x] `delpaths(paths)` / `del(path)`
- [x] `parent` (yq) - returns parent node of current position
- [x] `parent(n)` (yq) - returns nth parent node

### Math Functions (34 total)
- [x] Basic: `floor`, `ceil`, `round`, `trunc`, `sqrt`, `fabs`, `abs`
- [x] Exponential: `log`, `log10`, `log2`, `exp`, `exp10`, `exp2`
- [x] Trigonometric: `sin`, `cos`, `tan`, `asin`, `acos`, `atan`
- [x] 2-arg: `pow(x; y)`, `atan2(y; x)`
- [x] Hyperbolic: `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, `atanh`
- [x] Special: `infinite`, `nan`, `isinfinite`, `isnan`, `isnormal`, `isfinite`

### I/O & Debug
- [x] `debug` / `debug(msg)`
- [x] `$__loc__` - Current source location `{file, line}` where `$__loc__` appears
- [x] Comments in jq expressions (`#` to end of line)
- [x] `env`, `$ENV.VAR`, `env(VAR)`, `strenv(VAR)`
- [x] `now` - Current Unix timestamp
- [x] `builtins` - List all builtin function names

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

### Date/Time Functions
- [x] `now` - Current Unix timestamp as float
- [x] `gmtime` - Convert Unix timestamp to broken-down UTC time
- [x] `localtime` - Convert Unix timestamp to broken-down local time
- [x] `mktime` - Convert broken-down time to Unix timestamp
- [x] `strftime(fmt)` - Format broken-down time as string
- [x] `strptime(fmt)` - Parse string to broken-down time
- [x] `todate` / `todateiso8601` - Convert Unix timestamp to ISO 8601 string
- [x] `fromdate` / `fromdateiso8601` - Parse ISO 8601 string to Unix timestamp
- [x] `from_unix` - Convert Unix epoch to ISO 8601 string (yq extension)
- [x] `to_unix` - Parse ISO 8601 string to Unix epoch (yq extension)
- [x] `tz(zone)` - Convert Unix timestamp to datetime in specified timezone (yq extension)

**Timezone support**: IANA names (`America/New_York`), abbreviations (`EST`, `PST`, `JST`), numeric offsets (`+05:30`, `-0800`), and `UTC`/`GMT`.

### YAML Metadata Functions (yq)
- [x] `tag` - return YAML type tag (!!str, !!int, !!map, etc.) - fully working
- [x] `anchor` - return anchor name for nodes with anchors (`&name`) - fully working at cursor level
- [x] `alias` - return referenced anchor name for alias nodes (`*name`) - fully working at cursor level
- [x] `style` - return scalar/collection style (double, single, literal, folded, flow, or empty) - fully working at cursor level
- [x] `kind` - return node kind (scalar, seq, map, alias) - fully working, matches yq behavior
- [x] `key` - return current key when iterating (string for objects, int for arrays) - fully working
- [x] `line` - return 1-based line number of current node - fully working at cursor level
- [x] `column` - return 1-based column number of current node - fully working at cursor level

**Note on anchor/alias/style/line/column metadata**: The YamlCursor type has full metadata access methods:
- `cursor.anchor()` returns `Option<&str>` with the anchor name (e.g., "myanchor" for `&myanchor value`)
- `cursor.alias()` returns `Option<&str>` with the referenced anchor name for alias nodes (e.g., "myanchor" for `*myanchor`)
- `cursor.is_alias()` returns `bool` indicating if the node is an alias
- `cursor.style()` returns `&'static str` indicating the style ("double", "single", "literal", "folded", "flow", or "" for plain)
- `cursor.tag()` returns `&'static str` with the inferred YAML type tag
- `cursor.kind()` returns `&'static str` indicating the structural kind ("scalar", "seq", "map", "alias")
- `cursor.line()` returns `usize` with the 1-based line number of the node
- `cursor.column()` returns `usize` with the 1-based column number of the node

The jq builtins return empty string for anchor/style and 0 for line/column because metadata is lost during YAML→OwnedValue→JSON conversion for evaluation. Direct YAML cursor access preserves all metadata.

### Module System
- [x] `import "path" as name;` - Import module with namespace
- [x] `include "path";` - Include module definitions into current scope
- [x] `module {...}` - Module metadata (parsed)
- [x] `-L path` / `--library-path` CLI option
- [x] `JQ_LIBRARY_PATH` environment variable
- [x] `~/.jq` auto-loading (file or directory)
- [x] `namespace::func` - Namespaced function calls
- [x] Parameterized functions in modules

---

## TODO: Missing Features

### Priority 1: Module System ✅

Module system is now fully implemented:

- [x] `import "path" as name;` - Import module with namespace
- [x] `include "path";` - Include module inline
- [x] `module {...}` - Module metadata (parsed, stub implementation)
- [x] `-L path` / `--library-path` - Add library search paths
- [x] `JQ_LIBRARY_PATH` environment variable support
- [x] `~/.jq` auto-loading (file: auto-load definitions, directory: add to search path)
- [x] Namespaced function calls (`module::func`)
- [x] Functions with parameters preserved across modules

**Remaining limitations:**
- `module {...}` metadata is parsed but not used at runtime
- No `$__PROGDIR__` or module-relative paths yet

### Priority 2: Advanced Features ✅

All advanced features have been implemented:

- [x] `label $name | expr` / `break $name` - Non-local control flow

**Note:** Array slicing with steps (`.[::2]`) was removed - it's Python syntax, not jq. Use `[range(0; length; 2) as $i | .[$i]]` instead.

### Priority 3: CLI Enhancements

These are CLI-level features, not expression language:

- [x] `-s` / `--slurp` - Read all inputs into array
- [x] `-R` / `--raw-input` - Read lines as strings
- [x] `-n` / `--null-input` - Don't read input, use null
- [x] `--tab` - Use tabs for indentation
- [x] `--doc N` - Select specific document from multi-doc stream (yq only)
- [ ] `--argjson` / `--slurpfile` / `--rawfile` enhancements

---

## Known Limitations

### Intentionally Not Implemented

1. **SQL-style operators** - Not in standard jq
2. **Multi-precision integers** - Uses Rust's i64/f64
3. **Full jq module library** - Just core builtins
4. **`input` / `inputs` / `input_line_number`** - The succinct data structure approach builds a semi-index per document for efficient repeated queries. Streaming multiple documents within an expression conflicts with this architecture. Multiple input files are better handled at the CLI level, where each file gets its own optimized index. Users needing NDJSON/JSON Lines streaming should use standard `jq`.

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
| 2026-01-19 | Added tojson / fromjson for JSON string conversion (✅ complete)|
| 2026-01-19 | Added YAML metadata functions: tag, anchor, style for yq (✅ partial - tag works fully, anchor/style return defaults)|
| 2026-01-19 | Added kind function for yq - returns node kind: scalar, seq, map (✅ complete)|
| 2026-01-19 | Added key function for yq - returns current key when iterating (✅ complete)|
| 2026-01-19 | Added quoted field access `."key"` and bracket notation `.["key"]` (✅ complete)|
| 2026-01-19 | Added `#` comments in jq expressions (✅ complete)|
| 2026-01-19 | Added `now` builtin for current Unix timestamp (✅ complete)|
| 2026-01-19 | Added `abs` builtin as alias for fabs (✅ complete)|
| 2026-01-19 | Added `builtins` builtin to list all builtin function names (✅ complete)|
| 2026-01-19 | Added `normals` and `finites` type filters for numeric selection (✅ complete)|
| 2026-01-19 | Added `@urid` format for URI/percent decoding (✅ complete)|
| 2026-01-19 | Added `combinations` / `combinations(n)` for Cartesian product (✅ complete)|
| 2026-01-19 | Added `trunc` math function - truncate toward zero (✅ complete)|
| 2026-01-19 | Added `toboolean` type conversion function (✅ complete)|
| 2026-01-19 | Added `skip(n; expr)` iteration control - skip first n outputs (✅ complete)|
| 2026-01-19 | Moved `input`/`inputs`/`input_line_number` to "Won't implement" - conflicts with succinct data structure architecture|
| 2026-01-19 | Verified `$__loc__` already implemented - returns `{file, line}` at source location (✅ complete)|
| 2026-01-19 | Removed `.[::2]` step slicing from TODO - it's Python syntax, not jq|
| 2026-01-20 | Added `label $name | expr` / `break $name` for non-local control flow (✅ complete)|
| 2026-01-20 | Module system fully implemented: import, include, -L, JQ_LIBRARY_PATH, ~/.jq, namespaced calls, parameterized functions (✅ complete)|
| 2026-01-20 | Added YAML cursor metadata access: YamlCursor::anchor(), style(), tag(), kind() methods (✅ complete)|
| 2026-01-20 | Added reverse anchor mapping (bp_pos → anchor_name) to YamlIndex for O(1) anchor lookup|
| 2026-01-20 | Added YamlCursor::alias() and is_alias() methods to match yq's alias function (✅ complete)|
| 2026-01-20 | Updated kind() to return "alias" for alias nodes, matching yq behavior (✅ complete)|
| 2026-01-20 | Added YamlCursor::line() and column() methods for yq position metadata (✅ complete)|
| 2026-01-20 | Added `line` and `column` jq builtins (return 0 in evaluation, full support at cursor level)|
| 2026-01-20 | Added `-s`/`--slurp` CLI option for yq (✅ complete)|
| 2026-01-20 | Added `from_unix`, `to_unix`, `tz(zone)` yq date/time extensions (✅ complete)|
| 2026-01-20 | Added `-R`/`--raw-input` CLI option for yq (✅ complete)|
| 2026-01-20 | Added `--doc N` CLI option for yq document selection (✅ complete)|
