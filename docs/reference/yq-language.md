# yq Query Language Reference

[Home](../../) > [Docs](../) > [Reference](./) > yq Language

## Overview

The `yq` command provides YAML querying using jq-compatible syntax. It supports all features from the [jq Language Reference](jq-language.md) plus YAML-specific extensions.

**YAML 1.2 Compliance**: See [YAML 1.2 Compliance](../compliance/yaml/1.2.md) for details on type handling, including the Norway problem, sexagesimal numbers, and boolean recognition.

## Implementation Status Summary

| Category                  | Status            | Coverage |
|---------------------------|-------------------|----------|
| All jq features           | Fully implemented | 100%     |
| YAML metadata functions   | Fully implemented | 100%     |
| yq-specific operators     | Fully implemented | 95%      |
| Date/time operators       | Fully implemented | 95%      |
| Format encoders           | Fully implemented | 90%      |
| Multi-document support    | Fully implemented | 100%     |
| Anchors/aliases           | Fully implemented | 95%      |

"All jq features" means the full jq language is available; a subset of jq's
*builtins* that real yq's own lexer rejects (`paths`, `getpath`, `limit`,
`gsub`/`scan`/`splits`, `leaf_paths`, etc.) is off by default and requires
`--jq-extensions` — see [Gated jq Builtins](#gated-jq-builtins---jq-extensions)
below.

---

## Performance

### Apple M1 Max

| Size   | succinctly          | system yq           | Speedup   |
|--------|---------------------|---------------------|-----------|
| 10KB   | 4.2 ms (2.3 MiB/s)  | 8.4 ms (1.2 MiB/s)  | **2.0x**  |
| 100KB  | 5.5 ms (16.8 MiB/s) | 20.6 ms (4.5 MiB/s) | **3.8x**  |
| 1MB    | 15.3 ms (60.2 MiB/s)| 120.7 ms (7.6 MiB/s)| **7.9x**  |

### AMD Ryzen 9 7950X

| Size   | succinctly            | system yq             | Speedup   |
|--------|-----------------------|-----------------------|-----------|
| 10KB   | 1.63 ms (6.0 MiB/s)   | 64.6 ms (155 KiB/s)   | **40x**   |
| 100KB  | 2.78 ms (33.1 MiB/s)  | 79.6 ms (1.2 MiB/s)   | **29x**   |
| 1MB    | 13.2 ms (69.7 MiB/s)  | 210.5 ms (4.4 MiB/s)  | **16x**   |

---

## jq Compatibility

All features from the [jq Language Reference](jq-language.md) work with yq, including:

- Path expressions (`.foo`, `.[0]`, `.[]`, `..`)
- Operators (arithmetic, comparison, boolean, pipe, comma)
- Array/object operations (`keys`, `values`, `sort`, `group_by`, etc.)
- String functions (`split`, `join`, `test`, `match`, etc.)
- Control flow (`if-then-else`, `try-catch`, `reduce`, `foreach`)
- User-defined functions (`def name: body;`)
- Variable binding (`as $var`)
- Assignment operators (`=`, `|=`, `+=`, etc.)
- Format strings (`@json`, `@csv`, `@base64`, etc.)
- Module system (`import`, `include`)

### Operator precedence differs on `|` vs `,`

The [jq operator-precedence table](jq-language.md#operator-precedence) applies in
yq mode **except for its two loosest levels, which are swapped** (#2420). Real
yq's parser is a shunting-yard operator-precedence parser and its own table
(`pkg/yqlib/operation.go`, v4.53.3) gives `pipeOpType` precedence 30 and
`unionOpType` (`,`) precedence 10 — pipe binds *tighter* than comma, the
opposite of jq's `parser.y`. `succinctly yq` follows yq, `succinctly jq`
follows jq (ADR-0018 rule 2: the mode decides, not the input format):

| Filter               | `succinctly yq` / `yq` | `succinctly jq` / `jq` |
|----------------------|------------------------|------------------------|
| `.a, .b \| . + 10`   | `1` `12`               | `11` `12`              |
| `[.a, .b \| . + 10]` | `[1,12]`               | `[11,12]`              |
| `(.a, .b) \| . + 10` | `11` `12`              | `11` `12`              |
| `.a, (.b \| . + 10)` | `1` `12`               | `1` `12`               |

(on `a: 1`/`b: 2`.) It is a fixed entry in yq's operator table, so it holds at
every nesting depth. Explicit parentheses restore jq's grouping, on either side
of the comma, because they are a boundary the shunting yard cannot see across;
construction brackets do not, since `[...]` scopes evaluation rather than
precedence.

---

## YAML-Specific Features

### Metadata Functions

These functions access YAML-specific metadata not available in JSON:

| Function   | Description                        | Example Output                          |
|------------|------------------------------------|-----------------------------------------|
| `tag`      | YAML type tag                      | `!!str`, `!!int`, `!!map`               |
| `anchor`   | Anchor name for nodes with `&name` | `"myanchor"` or `""`                    |
| `style`    | Scalar/collection style            | `"double"`, `"literal"`, `"flow"`       |
| `kind`     | Node kind                          | `"scalar"`, `"seq"`, `"map"`, `"alias"` |
| `key`      | Current key when iterating         | `"name"` or `0`                         |
| `line`     | 1-based line number                | `5`                                     |
| `column`   | 1-based column number              | `3`                                     |

```bash
# Get line numbers of all items
succinctly yq '.items[] | {name: .name, line: line}' file.yaml

# Find nodes with anchors
succinctly yq '.. | select(anchor != "")' file.yaml

# Get style of scalars
succinctly yq '.description | style' file.yaml
# Output: "literal" (for block scalar |)
```

### Multi-Document Support

| Function           | Description                                   |
|--------------------|-----------------------------------------------|
| `document_index`   | 0-indexed document position in stream         |
| `di`               | Alias for `document_index`                    |
| `split_doc`        | Mark outputs as separate YAML documents       |

```bash
# Select second document only
succinctly yq 'select(di == 1)' multi.yaml

# Filter by document index
succinctly yq 'select(document_index == 0) | .metadata' multi.yaml

# Split array into separate documents
succinctly yq '.items[] | split_doc' file.yaml
# Output:
# item1
# ---
# item2
# ---
# item3
```

### Cross-File Operations

**Real yq surface, not a succinctly invention** — cross-file evaluation is real yq (`eval-all`), filed by [#715](https://github.com/rust-works/succinctly/issues/715) as a feature to implement, not as an extension. It carries an ordinary fidelity obligation under ADR-0018 rule 4, and the deviation noted below is tracked accordingly, not exempted under rule 5.

`--eval-all`/`--ea` combines every document from every input file into one evaluation context (default `eval` mode evaluates each document independently, low memory). `file_index`/`fileIndex`/`fi` returns the 0-indexed origin file position, resolvable only within that combined context.

| Function/Flag        | Description                                                      |
|----------------------|------------------------------------------------------------------|
| `--eval-all`, `--ea` | Combine all documents from all files into one evaluation context |
| `file_index`         | 0-indexed origin file position (within `--eval-all`)             |
| `fileIndex`          | Alias for `file_index` (real yq's own spelling)                  |
| `fi`                 | Short alias for `file_index`                                     |

```bash
# Combine documents across files, count them
succinctly yq --eval-all 'length' f1.yaml f2.yaml

# Select only documents from the first file
succinctly yq --eval-all '.[] | select(file_index == 0)' f1.yaml f2.yaml

# General merge across any number of files
succinctly yq --eval-all 'reduce .[] as $item ({}; . * $item)' f1.yaml f2.yaml f3.yaml

# Merge exactly two files (correct only when each contributes one document)
succinctly yq --eval-all '(.[] | select(file_index == 0)) * (.[] | select(file_index == 1))' f1.yaml f2.yaml
```

**Deviation from real yq**: real yq's `eval-all` treats the combined documents as an implicit node list that most operators broadcast over, so `select(fileIndex == 0) * select(fileIndex == 1)` works with no `.[]`. succinctly's evaluator has one scalar value per evaluation instead, so `.[]` must be explicit — `.[] | select(file_index == 0)`, not the bare `select(fileIndex == 0)`. `file_index`/`key`/`document_index` resolve correctly through `.`/`.[]`/`.field` navigation, comparisons, `select(...)`, `map(...)`, `if/then/else`, `try/catch`, comma, `label`, array literals (`[...]`), and user-defined functions — but not inside object literals (`{...}`) or `any`/`all`, where they fall back to `0` (see [Known Limitations](#known-limitations)). `--eval-all` is incompatible with `--slurp`, `--inplace`, `--raw-input`, `--split-exp`, and `--front-matter`.

### yq-Specific Operators

| Operator      | Description                                  | Example                        |
|---------------|----------------------------------------------|--------------------------------|
| `pick(keys)`  | Select only specified keys                   | `pick(["a", "b"])`             |
| `omit(keys)`  | Remove specified keys (inverse of pick)      | `omit(["temp", "debug"])`      |
| `shuffle`     | Randomly shuffle array elements              | `[1,2,3] \| shuffle`           |
| `pivot`       | Transpose arrays/objects (SQL-style)         | `[[1,2],[3,4]] \| pivot`       |
| `load(file)`  | Load external YAML/JSON file                 | `load("config.yaml")`          |
| `parent`      | Return parent node                           | `.. \| select(.name) \| parent`|
| `parent(n)`   | Return nth parent node                       | `parent(2)`                    |

```bash
# Remove keys from object
echo '{a: 1, b: 2, c: 3}' | succinctly yq 'omit(["a", "c"])'
# Output: {b: 2}

# Transpose array of arrays
echo '[[a, b], [x, y]]' | succinctly yq 'pivot'
# Output: [[a, x], [b, y]]

# Transpose array of objects
echo '[{name: "Alice", age: 30}, {name: "Bob", age: 25}]' | succinctly yq 'pivot'
# Output: {name: ["Alice", "Bob"], age: [30, 25]}

# Load external file
succinctly yq '. + {config: load("defaults.yaml")}' input.yaml

# Get parent of matching node
succinctly yq '.. | select(.name == "target") | parent' file.yaml
```

### Merge-Flag Suffixes on `*`/`*=`

Real yq extends the `*`/`*=` merge operator with combinable flag suffixes.
They go directly after `*` for the plain (non-assign) form, or after `*=`
for the in-place form — never between `*` and `=` (`.a *+= .b` is not
valid; the flags belong after the `=`, e.g. `.a *=+ .b`). Flags combine
freely in any order and duplicates are harmless (`*+d` ≡ `*d+` ≡ `*++dd`).

| Flag | Meaning                                                                                    |
|------|--------------------------------------------------------------------------------------------|
| `+`  | Append arrays instead of replacing them                                                    |
| `?`  | Only update fields/indices that already exist; never create new                            |
| `n`  | Only write fields/indices that don't already exist (or are `null`)                         |
| `d`  | Deep-merge arrays: treat them like objects, merging by index                               |
| `c`  | Clobber custom tags (parsed but a no-op today — no tag data exists to preserve or clobber) |

```bash
printf 'a: [1, 2]\nb: [3, 4]\n' | succinctly yq '.a *=+ .b'
# a: [1, 2, 3, 4]

printf 'a:\n  thing: one\n  cat: frog\nb:\n  missing: two\n  thing: two\n' \
  | succinctly yq '.a *=? .b'
# a: {thing: two, cat: frog}          — ? blocks new key "missing"

printf 'a:\n  thing: one\n  cat: frog\nb:\n  missing: two\n  thing: two\n' \
  | succinctly yq '.a *=n .b'
# a: {thing: one, cat: frog, missing: two}   — n blocks overwriting existing "thing"

# d: deep-merge arrays by index, like objects keyed by index
printf 'a: [{name: fred, age: 12}, {name: bob, age: 32}]\nb: [{name: fred, age: 34}]\n' \
  | succinctly yq '.a *=d .b'
# a: [{name: fred, age: 34}, {name: bob, age: 32}]
```

**Notes and known divergences from real yq:**

- Plain unflagged `*`/`*=` on two arrays replaces the left side wholesale
  with the right — this is yq-only; `succinctly jq` still errors on array
  `*` (real jq has no array-merge concept at all).
- `null` acts as an empty container on either side of a yq-mode merge: a
  null (or absent, since `.a` on a missing key evaluates to `null`) *left*
  operand merges as if it started from `{}`/`[]` — so `.a *=n .b` on
  `a: null` writes the full `.b` in, and `.a *=? .b` on an absent `.a`
  leaves it as `a: {}` (blocked by `?`, not `a: null`). A null *right*
  operand is always a no-op — `.a *= null` (with or without flags) leaves
  `.a` untouched, whatever it is. This only applies to yq mode; jq mode has
  no such exception and errors on every `null`-involving `*` pairing
  (#1175).
- `?`/`n` propagate through every nesting depth: a parent key that already
  exists still gets recursed into so its own new children can be added or
  blocked individually. Combining `?` and `n` is an AND of both gates (net
  effect: only touch a field that already exists and is currently `null`).
- `+`/`d` combined on the same array is a deliberate simplification: real
  yq's own combined behavior here is an undocumented, untested-upstream
  quirk. succinctly makes `+` take clean priority instead (pure append, `d`
  ignored).
- `c` requires custom YAML tag preservation, which succinctly doesn't
  support anywhere yet (the parser rejects custom tags outright) — it's
  parsed for forward compatibility but has no observable effect.

### `sub`/`gsub` in yq mode

Real yq's bare, 2-arg `sub(re; s)` diverges from jq's: it replaces **every**
match, not just the first — jq's `sub` = first match only, `gsub` = all
matches; yq's bare `sub` behaves like jq's `gsub` unconditionally (#1069,
confirmed against yq v4.53.3):

```bash
echo '"aaa"' | succinctly yq 'sub("a"; "X")'
# "XXX" — every match replaced (unlike succinctly jq's identical-syntax "Xaa")

echo '"aaa"' | succinctly yq --jq-extensions 'gsub("a"; "X")'
# "XXX" — same result for a non-zero-width pattern like this one
```

`gsub` (any arity), `scan`, and `splits` are not real yq builtins at all — real yq's own
lexer rejects them outright (confirmed live against yq v4.53.3, #1436). succinctly's `gsub`
support in yq mode is therefore a succinctly-only convenience, gated behind
`--jq-extensions` since #1512 (see [Gated jq Builtins](#gated-jq-builtins---jq-extensions)
below) rather than a verified match against an oracle that has nothing to compare against —
and, as of #1255 below, it's no longer a strict synonym for bare `sub` even internally: a
**zero-width-capable** pattern (`a*`, `x?`, `""`) makes the two diverge, since bare `sub`
picks up #1255's real-yq-verified Go-`regexp` iteration while `gsub` deliberately keeps the
original jq/Oniguruma-style iteration (there being no oracle for `gsub` to match instead):

```bash
echo '"bab"' | succinctly yq 'sub("a*"; "X")'                     # "XbXbX"  (Go-style, matches real yq)
echo '"bab"' | succinctly yq --jq-extensions 'gsub("a*"; "X")'    # "XbXXbX" (jq/Oniguruma-style, unchanged)
```

**Resolved (#1122):** the 3-arg `sub(re; s; flags)` form doesn't fit jq's
model or yq's own bare-`sub` model — real yq never evaluates `replacement`
or `flags` at all, and always does a global replace with the empty string
using only the pattern (near-certainly an upstream Go bug, not a designed
feature). succinctly now reproduces this bug-for-bug per
[ADR-0018](../adrs/adr-0018.md) rule 3:

```bash
echo '"aaa"' | succinctly yq 'sub("a"; "X"; "g")'
# "" — replacement and flags are never read; every match is deleted
```

### Date/Time Extensions

Beyond jq's standard date functions (`now` stays ungated; `gmtime`, `localtime`,
`mktime`, `strftime`, `strptime`, `todate`, `fromdate`, `todateiso8601`, and
`fromdateiso8601` all need `--jq-extensions` -- see
[Gated jq Builtins](#gated-jq-builtins---jq-extensions) for the full list), yq
adds:

| Function        | Description                   | Example                             |
|-----------------|-------------------------------|-------------------------------------|
| `from_unix`     | Unix epoch to ISO 8601 string | `1705766400 \| from_unix`           |
| `to_unix`       | ISO 8601 string to Unix epoch | `"2024-01-20T16:00:00Z" \| to_unix` |
| `tz(zone)`      | Convert timestamp to timezone | `now \| tz("America/New_York")`     |

```bash
# Convert Unix timestamp to datetime
echo '1705766400' | succinctly yq 'from_unix'
# Output: "2024-01-20T16:00:00Z"

# Convert datetime to Unix timestamp
echo '"2024-01-20T16:00:00Z"' | succinctly yq 'to_unix'
# Output: 1705766400

# Convert to specific timezone
echo '1705314600' | succinctly yq 'tz("America/New_York")'
# Output: "2024-01-15T05:30:00-05:00"

echo '1705314600' | succinctly yq 'tz("Asia/Tokyo")'
# Output: "2024-01-15T19:30:00+09:00"
```

**Supported timezone formats:**
- IANA names: `America/New_York`, `Europe/London`, `Asia/Tokyo`
- Abbreviations: `EST`, `PST`, `JST`, `CET`, `UTC`, `GMT`
- Numeric offsets: `+05:30`, `-0800`, `+09`

### Format Encoders

Beyond jq's standard formats (`@json`, `@csv`, `@base64`, etc.), yq adds:

| Format    | Description                        | Example Output           |
|-----------|------------------------------------|--------------------------|
| `@yaml`   | YAML flow-style encoding           | `{a: 1, b: 2}`           |
| `@props`  | Java properties format             | `key = value`            |

```bash
# Encode as YAML string (flow-style)
echo '{a: 1, b: [2, 3]}' | succinctly yq '@yaml'
# Output: "{a: 1, b: [2, 3]}"

# Encode as Java properties
echo '{database: "postgres", port: 5432}' | succinctly yq '@props'
# Output:
# database = postgres
# port = 5432

# Nested objects use dot-notation
echo '{db: {host: "localhost", port: 5432}}' | succinctly yq '@props'
# Output:
# db.host = localhost
# db.port = 5432
```

### Position-Based Navigation (Succinctly Extension)

Same as jq - see [jq Language Reference](jq-language.md#succinctly-extensions):

| Builtin                    | Description                                |
|----------------------------|--------------------------------------------|
| `at_offset(n)`             | Jump to node at byte offset n (0-indexed)  |
| `at_position(line; col)`   | Jump to node at line/column (1-indexed)    |

```bash
# Jump to node at byte offset
succinctly yq 'at_offset(10)' config.yaml

# Jump to node at line 5, column 3
succinctly yq 'at_position(5; 3)' config.yaml
```

### Gated jq Builtins (--jq-extensions)

Real yq's own lexer rejects a chunk of the jq language outright — these
builtins have no yq token at all, confirmed live against yq v4.53.3. Rather
than silently accepting broader syntax than the reference it's meant to
match, `succinctly yq` rejects them too by default (a parse error naming
`--jq-extensions`); passing the flag opts back into the jq-compatible
surface (#1512):

| Builtin                                              | Description                                   |
|------------------------------------------------------|-----------------------------------------------|
| `paths`, `paths(f)`, `leaf_paths`                    | Paths to every node / every leaf node         |
| `getpath(path)`                                      | Value at a path array                         |
| `tostream`, `fromstream(f)`, `truncate_stream(f)`    | Streaming event representation                |
| `IN(s)`, `IN(src; s)`                                | Membership test                               |
| `ltrimstr(s)`                                        | Strip a leading string prefix                 |
| `limit(n; f)`                                        | First `n` outputs of `f`                      |
| `isempty(f)`                                         | True if `f` produces no output                |
| `debug`, `debug(msg)`                                | Print to stderr, pass value through           |
| `infinite`, `isnan`                                  | IEEE 754 infinity literal / NaN test          |
| `nan`, `isinfinite`, `isnormal`, `isfinite`          | IEEE 754 NaN literal / classification (#1885) |
| `gsub(re; s)`, `scan(re)`, `splits(re)`              | Not real yq builtins at any arity (#1436)     |
| `inside(s)`                                          | Membership test, `contains`'s inverse         |
| `startswith(s)`, `endswith(s)`                       | String prefix / suffix test                   |
| `rtrimstr(s)`                                        | Strip a trailing string suffix                |
| `index(s)`, `rindex(s)`, `indices(s)`                | First / last / all match positions            |
| `range(n)`, `range(from; to)`, `range(from; to; by)` | Numeric generator                             |
| `nth(n)`, `nth(n; f)`                                | The `n`th value / output of `f`               |
| `combinations`, `combinations(n)`                    | Cartesian product of an array of arrays       |
| `pow(base; exp)`                                     | Exponentiation                                |
| `bsearch(target)`                                    | Binary search index in a sorted array         |
| `strftime(fmt)`, `strptime(fmt)`                     | Broken-down-time formatting / parsing (#1650) |
| `gmtime`, `localtime`, `mktime`                      | jq's date functions (#1907)                   |
| `todate`, `fromdate`                                 | ISO 8601 shortcuts (#1907)                    |
| `todateiso8601`, `fromdateiso8601`                   | ISO 8601, fixed format (#1907)                |
| `add`                                                | Sum of an array/stream (#1714)                |
| `min_by(f)`, `max_by(f)`                             | Extremum by a key function (#1714)            |
| `implode`                                            | Codepoint array to string (#1714)             |
| `INDEX(idx_expr)`, `INDEX(stream; idx_expr)`         | Build an object keyed by an index expr (#1714)|
| `walk(f)`                                            | Recursively transform every node (#1714)      |
| `floor`, `ceil`, `round`, `sqrt`, `fabs`             | Basic math functions (#1714)                  |
| `abs`, `trunc`                                       | Absolute value / truncate to integer (#1885)  |
| `log`, `log2`, `log10`                               | Logarithms (#1714)                            |
| `exp`, `exp2`, `exp10`                               | Exponentials (#1714)                          |
| `sinh`, `cosh`, `tanh`                               | Hyperbolic trig functions (#1714)             |
| `atan2(y; x)`                                        | Two-argument arctangent (#1714)               |
| `asin`, `acos`, `atan`                               | Inverse trig functions (#1837)                |
| `sin`, `cos`, `tan`                                  | Trig functions (#1837)                        |
| `asinh`, `acosh`, `atanh`                            | Inverse hyperbolic trig functions (#1837)     |
| `skip(n; f)`                                         | Drop the first `n` outputs of `f` (#1882)     |

```bash
echo '{}' | succinctly yq 'paths'
# Error: parse error: ... "paths" is not part of yq's syntax; pass --jq-extensions ...

echo '{"a":1}' | succinctly yq --jq-extensions -o=json 'paths'
# ["a"]
```

Real yq's own extensions (`sub`, `split`, the merge-flag suffixes on `*`/`*=`,
date/time and format encoders above) are unaffected — they're already yq
surface, not part of this gate. `leaf_paths` is a succinctly-only invention
modeled on a jq community recipe (real jq itself rejects it too, see the
[jq Language Reference](jq-language.md#succinctly-extensions)); it's grouped
here because, from `succinctly yq`'s syntax-surface point of view, it's the
same kind of thing as the rest of this table — extra, off by default.

---

## CLI Options

### Input Options

| Flag                  | Description                                      |
|-----------------------|--------------------------------------------------|
| `-n, --null-input`    | Don't read input; use null                       |
| `-p, --input-format`  | Input format: `auto`, `yaml`, `json`             |
| `-s, --slurp`         | Read all inputs into array                       |
| `-R, --raw-input`     | Read lines as strings instead of YAML            |
| `--doc N`             | Select Nth document (0-indexed)                  |
| `--eval-all`, `--ea`  | Combine docs/files into one eval context (below) |
| `--front-matter MODE` | Extract/process YAML front matter (below)        |
| `--jq-extensions`     | Accept jq-only builtins yq's lexer lacks (above) |

### Output Options

| Flag                  | Description                                   |
|-----------------------|-----------------------------------------------|
| `-r, --unwrapScalar`  | Output raw strings without quotes             |
| `-I, --indent N`      | Indent level (0 for compact)                  |
| `-o, --output-format` | Output format: `yaml`, `json`, `auto`         |
| `-i, --inplace`       | Update file in place                          |
| `-0, --nul-output`    | Use NUL separator instead of newline          |
| `--no-doc`            | Omit document separators (`---`)              |
| `--tab`               | Use tabs for indentation (write-only — see [Known Limitations](#known-limitations)) |
| `--split-exp EXPR`    | Split output into one file per result (below) |

### `--front-matter`

**Real yq surface, not a succinctly invention** — same correction as [Cross-File Operations](#cross-file-operations) above; `yq --help` lists `-f, --front-matter`.

Real yq can operate on YAML embedded as front matter inside another file (e.g. Markdown with a `---`-delimited YAML header). `--front-matter extract` evaluates the expression against just the front matter and discards the trailing content; `--front-matter process` re-emits the transformed front matter (re-fenced) followed by the original trailing content, unchanged.

```bash
# Extract: read only the front matter
succinctly yq --front-matter extract '.title' post.md

# Process: rewrite the front matter in place, body untouched
succinctly yq --front-matter process --inplace '.tags += ["new"]' post.md
```

A file without a leading `---` line errors. `--front-matter` is incompatible with `--doc`, `--null-input`, `--raw-input`, `--eval-all`, and an explicit `--input-format json` (front matter is YAML by definition); `--front-matter=process` additionally requires YAML output and is incompatible with `--slurp` (a slurped array can't reattach a body per input file); `--front-matter=extract` is incompatible with `--inplace` (it captures no body to reattach, so `-i` would discard everything after the closing fence). Position builtins (`at_offset`/`at_position`/`line`/`column`) resolve against the extracted YAML block's own coordinates, not the original file (see [Known Limitations](#known-limitations)).

### `--split-exp`

**Real yq surface, not a succinctly invention** — same correction as [Cross-File Operations](#cross-file-operations) above; `yq --help` lists `-s, --split-exp` (the long-only spelling below is a separate, deliberate divergence, not the extension question).

Splits output into one file per result instead of printing to stdout, named by evaluating `EXPR` against that result (`.` is the result; `$index` is its zero-based output index across the whole run; `--arg`/`--argjson` values and `$ARGS` are also available, same as the main filter).

```bash
# One file per array element, named by index
succinctly yq --split-exp '"out_" + ($index|tostring) + ".yml"' '.[]' data.yaml

# Named by a field of the result itself
succinctly yq --split-exp '.name + ".yml"' '.[]' data.yaml
```

**Deliberately long-only**, unlike real yq's `-s`/`--split-exp`: succinctly's `-s` is already `--slurp`. A non-string result errors; a duplicate filename overwrites with a warning. Incompatible with `--slurp`, `--inplace`, and `--front-matter`; `--raw-input` is not yet supported.

### Variables

| Flag                      | Description                          |
|---------------------------|--------------------------------------|
| `--arg NAME VALUE`        | Set $NAME to string VALUE            |
| `--argjson NAME JSON`     | Set $NAME to JSON VALUE              |

---

## Multi-Document Behavior

| Input       | Flag        | Behavior                           |
|-------------|-------------|------------------------------------|
| Single doc  | (none)      | Process document                   |
| Multi doc   | (none)      | Process all documents              |
| Multi doc   | `--slurp`   | All documents as array             |
| Multi doc   | `--doc N`   | Select Nth document only           |
| Multi doc   | `--no-doc`  | No `---` separators in output      |

---

## YamlCursor API (Programmatic Access)

For direct Rust API access, `YamlCursor` provides metadata methods:

```rust
cursor.anchor()   // Option<&str> - anchor name
cursor.alias()    // Option<&str> - referenced anchor for alias nodes
cursor.is_alias() // bool - check if node is an alias
cursor.style()    // &'static str - "double", "single", "literal", "folded", "flow", ""
cursor.tag()      // &'static str - inferred YAML type tag
cursor.kind()     // &'static str - "scalar", "seq", "map", "alias"
cursor.line()     // usize - 1-based line number
cursor.column()   // usize - 1-based column number
```

---

## Known Limitations

See [yq Remaining Work](../plan/yq-remaining.md) for incomplete features.

### Intentionally Not Implemented

1. **XML/TOML input/output** - Out of scope (separate tools)
2. **Comment preservation** - Comments not stored in semi-index
3. **Merge keys** (`<<: *alias`) - Rarely used, complex semantics
4. **Schema validation** - Separate tool concern

### Partial Implementation Notes

1. **`line`/`column` in complex expressions** - Work best with direct cursor access; may return 0 after DOM conversion
2. **Anchor metadata** - Available at cursor level; may be lost after complex jq operations
3. **`file_index`/`key`/`document_index` in complex expressions** - Resolve through `.`/`.[]`/`.field` navigation, comparisons, `select(...)`, `map(...)`, `if/then/else`, `try/catch`, comma, `label`, array literals (`[...]`, since #1302 gave `Expr::Array` its own path-context recursion), and user-defined functions; still return `0` inside object literals (`{...}`) or `any`/`all`
4. **`--slurp`/`--eval-all` output has no comments** - both combine documents through the `OwnedValue` DOM (which carries no comment data) before evaluating, unlike the default per-document path
5. **`--front-matter` position builtins use the extracted block's own coordinates** - `at_offset`, `at_position`, `line`, and `column` resolve against the extracted YAML slice, not the original file; a reported `line`/`column` is offset from the file's real line/column by the front-matter header's length
6. **`--tab` output does not round-trip** - succinctly's own YAML reader (like the wider YAML 1.1/1.2 spec) forbids tab characters in indentation, so any nested `--tab` YAML output cannot be read back by `succinctly yq` itself, or by other spec-strict YAML parsers. Real yq v4.53.3 has no `--tab` flag at all (confirmed live: `unknown flag: --tab`), so this is a succinctly-only extension with no round-trip oracle to satisfy — it is write-only by design (#1684)

---

## Examples

### Kubernetes Manifest Processing

```bash
# Get all container images
succinctly yq '.spec.template.spec.containers[].image' deployment.yaml

# Filter deployments by label
succinctly yq 'select(.metadata.labels.app == "web")' *.yaml

# Update image tag
succinctly yq '.spec.template.spec.containers[0].image = "nginx:1.25"' -i deployment.yaml

# Get all resource requests
succinctly yq '.spec.template.spec.containers[] | {name: .name, cpu: .resources.requests.cpu}' deployment.yaml
```

### GitHub Actions Workflow

```bash
# List all job names
succinctly yq '.jobs | keys' .github/workflows/ci.yml

# Get all uses: actions
succinctly yq '.jobs[].steps[].uses | select(. != null)' .github/workflows/ci.yml

# Find steps with specific action
succinctly yq '.jobs[].steps[] | select(.uses | startswith("actions/checkout"))' .github/workflows/ci.yml
```

### Docker Compose

```bash
# List all service names
succinctly yq '.services | keys' docker-compose.yml

# Get all exposed ports
succinctly yq '.services[].ports[]' docker-compose.yml

# Find services with specific image
succinctly yq '.services | to_entries[] | select(.value.image | contains("postgres"))' docker-compose.yml
```

---

## Changelog

| Date       | Change                                                                  |
|------------|-------------------------------------------------------------------------|
| 2026-01-20 | Initial yq implementation complete                                      |
| 2026-01-20 | Added date/time extensions: `from_unix`, `to_unix`, `tz`                |
| 2026-01-20 | Added `@yaml`, `@props` format encoders                                 |
| 2026-01-20 | Added `load(file)` operator                                             |
| 2026-01-20 | Added `split_doc` operator                                              |
| 2026-01-20 | Added multi-document support with `--doc N`                             |
| 2026-01-24 | Document created from plan/yq.md                                        |
| 2026-08-10 | Added `--front-matter`, `--split-exp`, `--eval-all`/`file_index` (#715) |
