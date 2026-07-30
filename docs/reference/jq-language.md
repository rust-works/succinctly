# jq Query Language Reference

[Home](../../) > [Docs](../) > [Reference](./) > jq Language

## Overview

This document describes the jq query language features implemented in succinctly.
The implementation covers ~95% of jq functionality and is production-ready.

## Implementation Status Summary

| Category              | Status              | Coverage |
|-----------------------|---------------------|----------|
| Core path expressions | Fully implemented   | 100%     |
| Operators             | Fully implemented   | 100%     |
| Type functions        | Fully implemented   | 100%     |
| Array operations      | Fully implemented   | 100%     |
| Object operations     | Fully implemented   | 100%     |
| String functions      | Fully implemented   | 100%     |
| Control flow          | Fully implemented   | 100%     |
| Math functions        | Fully implemented   | 100%     |
| Path operations       | Fully implemented   | 100%     |
| Format strings        | Fully implemented   | 100%     |
| Variable binding      | Fully implemented   | 100%     |
| User functions        | Fully implemented   | 100%     |
| Regex functions       | Fully implemented   | 100%     |
| Module system         | Fully implemented   | 95%      |
| I/O operations        | Won't implement     | N/A      |
| Assignment operators  | Fully implemented   | 100%     |
| Succinctly extensions | Fully implemented   | 100%     |

---

## Fully Implemented Features ✅

### Core Path Expressions
- [x] `.` - Identity
- [x] `.foo` - Field access
- [x] `."key"` - Quoted field access (for special characters like kebab-case)
- [x] `.["key"]` - Bracket notation with string key
- [x] `.[0]` - Array index (positive and negative)
- [x] `.[$k]`, `.[.k]`, `.[1,2]` - Bracket notation with a computed key (#360)
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
- [x] `omit(keys)` - remove specified keys (inverse of pick, yq)
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
- [x] `test(re)` (regex with `regex` feature; substring fallback without)

### Regular Expressions (with `regex` feature, included in `cli`)
- [x] `match(re)` / `match(re; flags)`
- [x] `capture(re)`
- [x] `scan(re)`
- [x] `splits(re)`
- [x] `sub(re; replacement)` / `gsub(re; replacement)`

### Format Strings
- [x] `@text` - Convert to string
- [x] `@json` - JSON encoding
- [x] `@csv` / `@tsv` - Delimited formats
- [x] `@dsv(delimiter)` - Custom delimiter; string fields always quoted (like `@csv`)
- [x] `@base64` / `@base64d`
- [x] `@uri` / `@urid` - Percent encoding / decoding
- [x] `@html` - HTML entity escaping
- [x] `@sh` - Shell quoting
- [x] `@yaml` - YAML flow-style encoding (yq)
- [x] `@props` - Java properties format (yq)

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
- [x] `isempty(expr)` - returns true if expr produces no outputs

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
- [x] `tostream` / `fromstream(f)` / `truncate_stream(f)` — jq's standard streaming
  builtins (#396). Distinct from `tojsonstream`/`fromjsonstream` above, which predate
  these and use a different (non-standard) event shape; both pairs are kept for
  compatibility. `truncate_stream(f)` takes a single argument, not `depth; f` — the
  depth comes from `.`, matching jq's own `def truncate_stream(stream): . as $n | stream | ...`.
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
- [x] `tag` - return YAML type tag (!!str, !!int, !!map, etc.)
- [x] `anchor` - return anchor name for nodes with anchors (`&name`)
- [x] `style` - return scalar/collection style (double, single, literal, folded, flow, or empty)
- [x] `kind` - return node kind (scalar, seq, map, alias)
- [x] `key` - return current key when iterating (string for objects, int for arrays)
- [x] `line` - return 1-based line number of current node
- [x] `column` - return 1-based column number of current node
- [x] `document_index` / `di` - return 0-indexed document position in multi-doc stream
- [x] `shuffle` - randomly shuffle array elements
- [x] `pivot` - transpose arrays/objects
- [x] `split_doc` - mark outputs as separate YAML documents
- [x] `load(file)` - load external YAML/JSON file

**Note on metadata builtins**: The `anchor`, `style`, `line`, and `column` builtins work best with direct cursor access. When YAML is converted to JSON for complex jq operations, some metadata may be lost. Direct YamlCursor methods (`cursor.anchor()`, `cursor.style()`, etc.) always preserve full metadata.

**YamlCursor API** (for programmatic access):
- `cursor.anchor()` → `Option<&str>` - anchor name (e.g., "myanchor" for `&myanchor value`)
- `cursor.alias()` → `Option<&str>` - referenced anchor for alias nodes (e.g., "myanchor" for `*myanchor`)
- `cursor.is_alias()` → `bool` - check if node is an alias
- `cursor.style()` → `&'static str` - style ("double", "single", "literal", "folded", "flow", or "")
- `cursor.tag()` → `&'static str` - inferred YAML type tag
- `cursor.kind()` → `&'static str` - structural kind ("scalar", "seq", "map", "alias")
- `cursor.line()` → `usize` - 1-based line number
- `cursor.column()` → `usize` - 1-based column number

### Module System
- [x] `import "path" as name;` - Import module with namespace
- [x] `include "path";` - Include module definitions into current scope
- [x] `module {...}` - Module metadata (parsed)
- [x] `-L path` / `--library-path` CLI option
- [x] [`JQ_LIBRARY_PATH`](environment-variables.md#jq_library_path) environment variable
- [x] `~/.jq` auto-loading (file or directory)
- [x] `namespace::func` - Namespaced function calls
- [x] Parameterized functions in modules

### Succinctly Extensions
These are succinctly-specific extensions not available in standard jq or yq:

- [x] `at_offset(n)` - Jump to node at byte offset n (0-indexed)
- [x] `at_position(line; col)` - Jump to node at line/column (1-indexed)

These enable IDE integration and programmatic navigation to specific document positions.

---

## Known Limitations

**Note:** Array slicing with steps (`.[::2]`) is intentionally not supported - it's Python syntax, not jq. Use `[range(0; length; 2) as $i | .[$i]]` instead.

**Computed keys in brackets** (#360) accept any expression, but two jq behaviours are not reproduced:

- **Slice bounds must fold to an integer literal.** Both bounds accept the same spellings — `.[1:3]`, `.[(1):3]`, `.[1:(3)]`, `.[1.0:3]` — but nothing that has to be evaluated: `.[$a:$b]` and `.[(1+1):]` are parse errors, where jq accepts them.
- **An array-valued key errors instead of searching.** jq reads `[10,20,30] | .[[20]]` as an indices-of-subarray search returning `[1]`; here it reports `Cannot index array with array`.

Two further divergences are confined to path contexts. `path(.[1.7])` yields `[1]` where jq keeps the unfloored `[1.7]`. A NaN key reads as `null` in both (`[10,20,30] | .[nan]` → `null`), but has no path component here: `path(.[nan])` and `del(.[nan])` report `Cannot set array element at NaN index`, jq's own wording for the assignment case, where jq instead yields the path `[null]` that its own `setpath` rejects (and, for `del`, hangs).

A computed key **after a multi-output path component** — `path(.. | .[.k])`, `path(recurse | .[.k])`, `path(.. | objects | .[.k])` — resolves per branch, same as `path(.[] | .[.k])` (#412: `..` and bare `recurse` share one resolver, `recurse(f)`/`recurse(f; cond)` follow the value path's queue, and the typeof filters (`select`, `objects`, `arrays`, `values`, ...) each got a path-tracking arm). A multi-output component with none of those shapes — an arbitrary generator like `range(3)`, or `getpath` with a computed argument — still reports `Cannot use a computed index after a multi-output path component`, since naming the path components it produces would mean tracking components for a genuinely arbitrary expression.

`recurse(f)` for an `f` that keeps yielding null stops rather than descending, matching `[recurse(f)]` and bounding a walk jq does not bound at all: `recurse(.a?)` over `{"a":null}` reads null from null forever, and jq runs until it cannot allocate. In the other direction, `[recurse(f)]` descends into an array `f` returns where jq stops at the array itself; in a *path* context succinctly stops at the array, i.e. follows jq rather than its own value path.

**Indexing by a variable bound from a generator does not work yet** (#397). `.[$k]` itself is fine, but `keys[] as $k | .[$k]` — the most common way to reach it — binds `$k` to the whole array rather than each element. The bracket syntax is not at fault, and neither is `as`: iterating a *computed* value collapses the stream into a single array before the binding ever happens, so `[keys[]]` is already `[["a","b"]]` rather than `["a","b"]`. Explicit bindings (`--arg k a`, `. as $x | $x[…]`, `def f($k): .[$k]`) and iteration of a value navigated out of the document (`.[]`) are unaffected.

See [jq Remaining Work](../plan/jq-remaining.md) for incomplete CLI and module system features.

### Intentionally Not Implemented

1. **SQL-style operators** - Not in standard jq
2. **Multi-precision integers** - Uses Rust's i64/f64
3. **Full jq module library** - Just core builtins
4. **`input` / `inputs` / `input_line_number`** - The succinct data structure approach builds a semi-index per document for efficient repeated queries. Streaming multiple documents within an expression conflicts with this architecture. Multiple input files are better handled at the CLI level, where each file gets its own optimized index. Users needing NDJSON/JSON Lines streaming should use standard `jq`.

### Partial Implementation Notes

1. **Variable scoping** - May not perfectly match jq edge cases
2. **Error messages** - byte-identical to jq-1.7.1 across the probe corpus in `tests/data/jq-error-probes.tsv`, bar the probes on which succinctly raises no error at all; those and the reverse case (succinctly errors where jq answers) are enumerated in [jq Known Limitations](../compliance/jq/limitations.md)
3. **Numeric overflow** - Uses wrapping arithmetic
4. **`$ENV` as bare object** - Only field access works (`$ENV.VAR`)
5. **Number equality above 2^53** - `1 == 1.0` is `true` as in jq, but a mixed
   integer/float comparison widens the integer to `f64`, while jq 1.7 retains
   the decimal literal. They agree on every value representable exactly as an
   `f64`; beyond that `9007199254740993 == 9007199254740992.0` is `true` here
   and `false` in jq

---

## Testing Strategy

### Compatibility Tests

Run against canonical jq to verify:

```bash
# Compare output by hand
echo '{"a":1}' | jq '.a'
echo '{"a":1}' | succinctly jq '.a'

# Or run the pinned-jq oracle suites (both hermetic; jq only needed to resync)
cargo test --features cli,regex --test jq_golden_tests
cargo test --features cli,regex --test jq_error_message_tests
./scripts/sync-jq-golden.sh --check
./scripts/sync-jq-error-messages.sh --check
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
| 2026-01-20 | Added `split_doc` yq operator for outputting results as separate documents (✅ complete)|
| 2026-01-20 | Fixed `select(di == N)` to work correctly - added Select and Compare to generic evaluator (✅ complete)|
| 2026-01-24 | Document audit: Added `omit(keys)`, `load(file)`, `at_offset(n)`, `at_position(line; col)` to docs|
| 2026-01-24 | Clarified YAML metadata: `alias` is cursor-level API only (not a jq builtin)|
| 2026-01-24 | Updated coverage to 100% for most categories after comprehensive code review|
