# Succinctly CLI Tool

[Home](../../) > [Docs](../) > [Guides](./) > CLI

Command-line utility for working with succinct data structures.

## Installation

Build the CLI tool with the `cli` feature:

```bash
cargo build --release --features cli
```

The binary will be located at `target/release/succinctly`.

### Short Aliases

Install short alias symlinks for interactive use:

```bash
succinctly install-aliases              # creates symlinks next to the binary
succinctly install-aliases --dir ~/bin  # or specify a directory on your PATH
```

This creates `sjq`, `syq`, `sjq-locate`, and `syq-locate` symlinks. With aliases installed, you can use `sjq` instead of `succinctly jq`:

```bash
sjq '.users[].name' input.json        # instead of: succinctly jq '.users[].name' input.json
syq '.spec.containers[]' k8s.yaml     # instead of: succinctly yq '.spec.containers[]' k8s.yaml
```

The binary also recognizes `jq` and `yq` as alias names if you create those symlinks manually, but `install-aliases` does not create them to avoid shadowing system tools.

## Commands

### JSON Generation

Generate synthetic JSON files for benchmarking and testing.

```bash
succinctly json generate <SIZE> [OPTIONS]
```

#### Size Format

Supports case-insensitive size units:
- **Plain numbers**: `1024` (bytes)
- **Bytes**: `100b` or `100B`
- **Kilobytes**: `1kb`, `512KB`, `1Kb`
- **Megabytes**: `1mb`, `10MB`, `100Mb`
- **Gigabytes**: `1gb`, `2GB`, `5Gb`

#### Options

- `-o, --output <FILE>`: Write to file (default: stdout)
- `-p, --pattern <PATTERN>`: JSON pattern to generate (default: `comprehensive`)
- `-s, --seed <SEED>`: Random seed for reproducible generation
- `--pretty`: Pretty print JSON
- `--verify`: Validate generated JSON
- `--depth <N>`: Nesting depth for nested structures (default: 5)
- `--escape-density <F>`: Escape sequence density 0.0-1.0 (default: 0.1)

#### Patterns

**comprehensive** (default)
- Best for benchmarking
- Tests all JSON parsing features:
  - Simple values (booleans, nulls, numbers)
  - String variations with escapes
  - Number formats (integers, decimals, scientific notation)
  - Nested arrays and objects
  - Unicode strings (emoji, multiple scripts)
  - Edge cases (empty structures, long strings, etc.)
- Distributed across 10 feature categories
- Optimized for SIMD, state machine, and BP operation testing

**users**
- Array of user objects
- Realistic structure with IDs, names, emails, ages, scores
- Good for testing realistic workloads

**nested**
- Deeply nested objects
- Tests nesting depth and balanced parentheses operations
- Configurable depth with `--depth`

**arrays**
- Arrays of arrays
- Tests array handling and nesting

**mixed**
- Balanced mix of all JSON types
- Random distribution of strings, numbers, bools, nulls, arrays, objects

**strings**
- String-heavy documents
- Tests string parsing performance
- Long Lorem Ipsum strings

**numbers**
- Number-heavy documents
- Various number formats (integers, decimals, scientific notation)

**literals**
- Boolean and null heavy
- Tests literal parsing (true, false, null)

**unicode**
- Unicode-heavy strings
- Multiple scripts: Chinese, Russian, Arabic, Japanese, Korean, Greek, Hebrew, Hindi
- Emoji and special symbols

**pathological**
- Worst-case for parsing
- Maximum structural character density
- Deeply nested with many structural tokens

### JSON Validation

Validate JSON files strictly according to RFC 8259 with detailed error messages.

```bash
succinctly json validate [OPTIONS] [FILES...]
```

#### Options

- `-q, --quiet`: Quiet mode, exit code only (no output)
- `-C, --color`: Force color output even when not a TTY
- `-M, --no-color`: Disable color output

#### Exit Codes

- `0`: JSON is valid
- `1`: JSON is invalid (validation error)
- `2`: I/O error (file not found, permission denied, etc.)

#### Examples

```bash
# Validate from stdin
echo '{"name": "Alice"}' | succinctly json validate
# Exit code 0 (valid), no output

# Validate a file
succinctly json validate config.json

# Validate multiple files
succinctly json validate file1.json file2.json file3.json

# Quiet mode (exit code only)
succinctly json validate --quiet input.json && echo "Valid"

# Colorized error output
succinctly json validate --color invalid.json
```

#### Error Output

When JSON is invalid, the validator shows detailed error messages with:
- Error type and description
- File location (filename:line:column)
- Code snippet with visual error indicator

```
error: expected string key, found '}'
  --> input.json:3:15
     |
   3 |   "name": "Alice",}
     |                   ^
```

#### Validation Rules (RFC 8259)

The validator enforces strict RFC 8259 compliance:

**Numbers:**
- No leading `+` sign (`+1` is invalid)
- No leading zeros (`007` is invalid, but `0` and `0.5` are valid)
- Decimal point must have digits on both sides (`1.` and `.5` are invalid)
- Exponent must have at least one digit (`1e` is invalid)

**Strings:**
- Control characters (U+0000 through U+001F) must be escaped
- Valid escapes: `\"`, `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t`, `\uXXXX`
- Surrogate pairs must be properly paired (high surrogate D800-DBFF followed by low surrogate DC00-DFFF)

**Structure:**
- No trailing commas in objects or arrays
- Object keys must be strings
- Only one root value allowed (no trailing content)

**Whitespace:**
- Only space (0x20), tab (0x09), newline (0x0A), and carriage return (0x0D) allowed outside strings

---

### YAML Validation

`succinctly` is a non-validating YAML loader by design (see
[YAML limitations](../compliance/yaml/limitations.md)). `yaml validate` is the **opt-in**
strict counterpart, mirroring `json validate`: a separate pass, run before indexing, that
rejects invalid YAML. The default indexing path (`syq`, `YamlIndex::build`) is unaffected
and still accepts the same input unless you ask for validation.

```bash
succinctly yaml validate [OPTIONS] [FILES...]
```

#### Options

- `-q`, `--quiet`: Exit code only, no diagnostic output
- `-C`, `--color`: Force color output even when not a TTY
- `-M`, `--no-color`: Disable color output

#### Exit Codes

- `0`: All inputs are valid
- `1`: At least one input is invalid
- `2`: I/O error (file not found, permission denied, etc.)

#### Examples

```bash
# Validate stdin
echo 'a: b: c' | succinctly yaml validate

# Validate a file (rustc-style caret diagnostics)
succinctly yaml validate config.yaml

# Exit code only, for scripts
succinctly yaml validate --quiet config.yaml && echo "Valid"

# Validate before querying, in one step (bails before producing output)
syq --validate '.users[]' config.yaml
```

#### Validation Rules (YAML)

The validator rejects the classes of malformed YAML below (the YAML Test Suite's `lax:*`
cases). It is deliberately not a full grammar checker: anything it does not recognize as
invalid, it accepts.

- **Indentation & tabs** — indentation that matches no open block level; tabs used where
  indentation is expected (`\t` before a mapping key or between block indicators).
- **Structure & mappings** — a second root node after a block collection; a compact nested
  mapping key (`a: b: c`); an inline block sequence after `:` (`key: - a`).
- **Scalars & quoting** — invalid double-quoted escapes (`"\."`); trailing content after a
  quoted scalar; a multi-line scalar used as an implicit key; invalid block-scalar headers
  (`|0`, `|10`, `> text`).
- **Flow collections** — leading/doubled commas, unbalanced or unclosed brackets, bare `-`
  items, document markers inside a flow collection.
- **Anchors & aliases** — an anchor immediately followed by a block indicator with no value
  between them. A `*` that is scalar content rather than a node (`a: rm *.tmp`) is untouched.
  (An anchor or tag immediately before an alias, `&a *b` / `!!str *b`, and an alias naming an
  out-of-scope anchor, `a: *nope`, are both rejected by the *default* loader itself — see
  [YAML Limitations](../compliance/yaml/limitations.md#three-exceptions-an-alias-with-no-usable-target-or-an-illegal-decoration-is-rejected) —
  so `--validate` adds nothing beyond the default for either.)
- **Comments** — a `#` not separated from the preceding token by whitespace; a comment
  interrupting a multi-line plain scalar.
- **Documents & directives** — content after a `...` marker; a `%YAML` directive with no
  following `---`, malformed, or duplicated; a document marker inside an open quoted scalar.

---

## Examples

### Basic Generation

```bash
# Generate 1MB to stdout
succinctly json generate 1mb

# Generate 10MB to file
succinctly json generate 10mb -o benchmark.json

# Generate with verification
succinctly json generate 100kb --verify -o test.json
```

### Pattern Selection

```bash
# Comprehensive benchmarking pattern
succinctly json generate 10mb --pattern comprehensive -o bench.json

# Pathological worst-case
succinctly json generate 1mb --pattern pathological -o worst-case.json

# Unicode testing
succinctly json generate 1mb --pattern unicode -o unicode-test.json

# Realistic user data
succinctly json generate 5mb --pattern users -o users.json
```

### Reproducible Generation

```bash
# Generate with seed for reproducibility
succinctly json generate 100mb --seed 42 -o reproducible.json

# Same seed produces identical output
succinctly json generate 100mb --seed 42 -o reproducible2.json
diff reproducible.json reproducible2.json  # No differences
```

### Configuration

```bash
# Control nesting depth
succinctly json generate 10mb --depth 10 -o deep.json

# Control escape sequence density (30% of strings have escapes)
succinctly json generate 10mb --escape-density 0.3 -o escaped.json

# Pretty printed JSON
succinctly json generate 10kb --pretty -o pretty.json
```

### Piping and Benchmarking

```bash
# Generate and pipe to parser
succinctly json generate 10mb | your-json-parser

# Generate and immediately benchmark
succinctly json generate 100mb -o benchmark.json --verify
time cat benchmark.json | your-parser
```

## Benchmarking Recommendations

For comprehensive JSON parser benchmarking:

1. **Use the comprehensive pattern** (default):
   ```bash
   succinctly json generate 100mb -o benchmark.json
   ```

2. **Test multiple sizes**:
   ```bash
   for size in 1kb 16kb 128kb 1mb 16mb 128mb; do
       succinctly json generate $size -o bench-$size.json
   done
   ```

3. **Test different patterns**:
   ```bash
   for pattern in comprehensive users nested pathological unicode; do
       succinctly json generate 10mb --pattern $pattern -o bench-$pattern.json
   done
   ```

4. **Use reproducible seeds** for consistent benchmarks:
   ```bash
   succinctly json generate 100mb --seed 42 -o benchmark.json
   ```

## Output Format

The comprehensive pattern generates JSON with the following structure:

```json
{
  "metadata": {...},
  "simple_values": {...},      // 10% - bools, nulls, simple numbers
  "strings": [...],            // 15% - various string patterns
  "numbers": [...],            // 10% - number format variations
  "arrays": {...},             // 15% - nested and flat arrays
  "nested": {...},             // 15% - deep object nesting
  "data": [...],               // 20% - realistic records
  "unicode": [...],            // 10% - unicode strings
  "edge_cases": {...}          // 5%  - parser edge cases
}
```

Each section tests specific parsing features:
- **State transitions**: InJson ↔ InString ↔ InEscape ↔ InValue
- **Structural characters**: `{}[],:` (BP open/close operations)
- **Escape sequences**: `\"`, `\\`, `\n`, `\t`, `\r`, `\b`, `\f`
- **Number parsing**: integers, decimals, scientific notation
- **Unicode**: UTF-8 multibyte sequences
- **Nesting depth**: Balanced parentheses find_close operations

## yq Command

Query YAML files using a yq-compatible interface (mikefarah/yq). Implements YAML 1.2 Core Schema - see [YAML 1.2 Compliance](../compliance/yaml/1.2.md) for details on type handling.

```bash
succinctly yq [OPTIONS] <FILTER> [FILES...]
```

### Basic Usage

```bash
# Identity filter (pretty-print YAML)
succinctly yq . input.yaml

# Field access
succinctly yq '.name' input.yaml

# Array indexing
succinctly yq '.[0]' input.yaml
succinctly yq '.users[0].name' input.yaml

# Iterate all elements
succinctly yq '.[]' input.yaml
succinctly yq '.users[]' input.yaml
```

### Output Options

- `-o, --output-format <FORMAT>`: Output format: `yaml` (default), `json`, `auto`
- `-I, --indent <N>`: Indent level (0 for compact output, default: 2)
- `-r, --unwrapScalar`: Output raw strings without quotes (default for YAML)
- `-j, --join-output`: Like -r but no newline after each output. **`-j` collides with real yq's own `-j`** — real yq's `-j` is a deprecated alias for `--tojson` (forces JSON output, unrelated to line-joining), and `--join-output` doesn't exist in real yq at all (confirmed against the pinned v4.53.3 binary; see [yq Limitations § Open divergences](../compliance/yq/limitations.md#open-divergences-bugs-not-decisions)). Unaffected in jq mode below, where `-j`/`--join-output` does match real jq
- `-0, --nul-output`: Use NUL char to separate values instead of newline
- `-S, --sort-keys`: Sort keys of each object on output
- `-C, --colors`: Force colorized output
- `-M, --no-colors`: Disable colorized output
- `-N, --no-doc`: Don't print document separators (`---`)
- `-P, --prettyPrint`: Pretty print, expand flow styles to block style
- `--tab`: Use tabs for indentation (write-only — see [yq Language Reference § Known Limitations](../reference/yq-language.md#known-limitations))
- `-a, --ascii-output`: Output ASCII only, escaping non-ASCII as `\uXXXX`. **Not a real yq flag** — v4.53.3 rejects it as an unknown flag; this is a succinctly extension borrowed from jq's own `-a` (ADR-0018 rule 5), so it carries no fidelity obligation to yq. JSON output only: `-o yaml --ascii-output` emits raw UTF-8, since YAML has no `\uXXXX` escape convention to borrow. Escaping happens at the output sink (`AsciiEscapeWriter`), so the flag keeps the streaming path and its duplicate mapping keys; the routes that materialize a DOM anyway (`-P` without `-I0`, `--arg`/`--argjson`, `-r`/`-j`/`-0`, `--eval-all`, `--split-exp`) still collapse duplicates and carry the DOM emitter's own divergences (#1700, #1982)
- `--split-exp <EXPR>`: Split output into one file per result, named by evaluating `EXPR` against it (`.` is the result, `$index` its 0-based output index; `--arg`/`--argjson` values and `$ARGS` are also available, same as the main filter). Suppresses stdout. Long-only, unlike real yq's `-s`/`--split-exp` — succinctly's `-s` is already `--slurp` (#715). Incompatible with `--slurp`, `--inplace`, `--front-matter`; `--raw-input` not yet supported

### Input Options

- `-n, --null-input`: Don't read any input; use null as the single input value
- `-R, --raw-input`: Read each line as a string instead of parsing as YAML/JSON
- `-s, --slurp`: Read all inputs into an array and use it as the single input value
- `-p, --input-format <FORMAT>`: Input format: `auto` (default), `yaml`, `json`
- `--validate`: Validate YAML strictly (opt-in) before processing; reports line:column errors and bails without producing output (see [YAML Validation](#yaml-validation))
- `-i, --inplace`: Update the file in place
- `--doc <N>`: Select specific document by 0-based index from multi-document stream
- `--eval-all`, `--ea`: Combine all documents from all files into one evaluation context, exposing `file_index`/`fileIndex`/`fi` for cross-file merges (e.g. `.[] | select(file_index == 0)`). Requires explicit `.[]` iteration, unlike real yq's bare `select(fileIndex == 0)` — see [yq Language Reference](../reference/yq-language.md#cross-file-operations) for the full deviation (#715). Combines documents through the `OwnedValue` DOM, same as `--slurp`, so output carries no comments regardless of the input. Incompatible with `--slurp`, `--inplace`, `--raw-input`, `--split-exp`, `--front-matter`
- `--front-matter <MODE>`: Treat input as text with a `---`-fenced YAML front matter header (e.g. Markdown). `extract` evaluates only the front matter, discarding the body; `process` re-emits the transformed front matter followed by the untouched body. Incompatible with `--doc`, `--null-input`, `--raw-input`, `--eval-all`, an explicit `--input-format json` (front matter is YAML by definition); `process` mode also requires YAML output and is incompatible with `--slurp`; `extract` mode is incompatible with `--inplace` (it captures no body to reattach, so `-i` would discard everything after the closing fence). Position builtins (`at_offset`, `at_position`, `line`, `column`) resolve against the extracted YAML block's own coordinates, not the original file — a `line`/`column` reported under `--front-matter` does not match the file's line/column

### Variables

- `--arg NAME VALUE`: Set $NAME to the string VALUE
- `--argjson NAME VALUE`: Set $NAME to the JSON VALUE

### Exit Status

- `-e, --exit-status`: Set exit status based on output (0 if last output != false/null)

| Code | Meaning                                                              |
|------|----------------------------------------------------------------------|
| 0    | Success                                                              |
| 1    | Uncaught evaluation error, or with `-e`, no truthy result            |
| 3    | Compile error (`--validate` failure)                                 |

An uncaught evaluation error prints `Error: <message>` to stderr and exits 1,
matching mikefarah/yq. The diagnostic never goes to stdout, so the erroring
document produces no output on its own — though in a multi-document stream,
earlier documents that succeeded still reach stdout:

```bash
succinctly yq 'error("boom")' config.yaml   # stderr: Error: boom
echo $?                                     # 1
```

Note this differs from the `jq` subcommand, which follows jq's convention of
exit code 5 and a `jq: error (at <file>:<line>)` diagnostic. Each subcommand
matches its own upstream.

### Examples

```bash
# YAML to JSON conversion
succinctly yq -o json . config.yaml

# Compact JSON output (use -I 0)
succinctly yq -o json -I 0 . config.yaml

# Raw string output
succinctly yq -r '.name' user.yaml

# JSON input with YAML output
succinctly yq -p json . input.json

# Update file in place
succinctly yq -i '.version = "2.0"' config.yaml

# NUL-separated output (for xargs -0)
succinctly yq -0 '.items[]' list.yaml | xargs -0 process

# Multiple files
succinctly yq '.version' config1.yaml config2.yaml

# Pipe from stdin
cat data.yaml | succinctly yq '.items[]'

# Multi-document YAML
succinctly yq '.' multi-doc.yaml

# Combine documents across files, merge by file origin
succinctly yq --eval-all '(.[] | select(file_index == 0)) * (.[] | select(file_index == 1))' base.yaml override.yaml

# Extract/rewrite YAML front matter in a Markdown file
succinctly yq --front-matter extract '.title' post.md
succinctly yq --front-matter process --inplace '.tags += ["new"]' post.md

# Split each array element into its own file
succinctly yq --split-exp '.name + ".yml"' '.[]' users.yaml
```

### Differences from jq

The `yq` subcommand uses yq-compatible flags (mikefarah/yq v4):

| Feature | jq | yq (succinctly yq) |
|---------|----|--------------------|
| Compact output | `-c` | `-I 0` |
| Raw output | `-r` | `-r` (--unwrapScalar) |
| Default output | JSON | YAML |
| Input format | JSON only | Auto-detect (YAML/JSON) |
| Inplace edit | N/A | `-i` |
| NUL separator | N/A | `-0` |

---

## jq Command

Query JSON files using a jq-compatible interface.

```bash
succinctly jq [OPTIONS] <FILTER> [FILES...]
```

### Basic Usage

```bash
# Identity filter (pretty-print)
succinctly jq . input.json

# Field access
succinctly jq '.name' input.json

# Array indexing
succinctly jq '.[0]' input.json
succinctly jq '.users[0].name' input.json

# Iterate all elements
succinctly jq '.[]' input.json
succinctly jq '.users[]' input.json
```

### Output Options

- `-c, --compact-output`: Compact output (no pretty printing)
- `-r, --raw-output`: Output raw strings without quotes
- `-j, --join-output`: Like -r but no newline after each output
- `-S, --sort-keys`: Sort keys of each object on output
- `-C, --color-output`: Colorize output
- `-M, --monochrome-output`: Disable colorized output
- `--tab`: Use tabs for indentation
- `--indent <N>`: Use N spaces for indentation (max 7)

### Output Formatting

By default, succinctly formats output exactly like jq does:
- **Scientific notation**: `4e4` → `4E+4`, `12e2` → `1.2E+3`, `1e-3` → `0.001`
- **Trailing zeros**: preserved (`0.10` → `0.10`)
- **Escape sequences**: `\b` and `\f` output as escape sequences (not `\u0008`)

To preserve the original formatting from the input (e.g., keep `4e4` as `4e4`):

```bash
# Using flag
succinctly jq --preserve-input . input.json

# Using environment variable
SUCCINCTLY_PRESERVE_INPUT=1 succinctly jq . input.json
```

See [Environment Variables](../reference/environment-variables.md) for the accepted values.

`--preserve-input` governs how a value is *spelled* on output, not what the value is: it
echoes `4e4` as written, and likewise keeps every occurrence of a duplicated object key,
while the evaluator still follows jq (`length` on `{"a":1,"a":2}` is `1` either way). See
[jq Limitations](../compliance/jq/limitations.md#duplicate-object-keys-collapse-except-under---preserve-input).

### Input Options

- `-n, --null-input`: Don't read any input; use null as the single input value
- `-R, --raw-input`: Read each line as a string instead of JSON
- `-s, --slurp`: Read all inputs into an array
- `--input-dsv <DELIMITER>`: Read input as DSV (delimiter-separated values); each row becomes a JSON array of strings
- `--validate`: Validate JSON strictly according to RFC 8259 before processing; reports detailed validation errors with line:column positions

### Variables

- `--arg NAME VALUE`: Set $NAME to the string VALUE
- `--argjson NAME VALUE`: Set $NAME to the JSON VALUE
- `--slurpfile NAME FILE`: Set $NAME to an array of JSON values from FILE
- `--rawfile NAME FILE`: Set $NAME to the string contents of FILE

### Exit Status

- `-e, --exit-status`: Set exit status from the last output (see the table below)

| Code | Meaning                                                              |
|------|----------------------------------------------------------------------|
| 0    | Success                                                              |
| 1    | With `-e`, the last output was `false` or `null`                     |
| 3    | Compile error (`--validate` failure)                                 |
| 4    | With `-e`, no output was produced                                    |
| 5    | Uncaught evaluation error                                            |

An uncaught evaluation error prints a diagnostic to stderr and exits 5, so a
failed filter is distinguishable from a successful one under `set -e`, `&&`, or
an explicit `$?` check:

```bash
echo '{"x":1}' | succinctly jq 'error("boom")'
# stderr: jq: error (at <stdin>:1): boom
echo $?   # 5
```

The diagnostic names the file and the line the input value ended on (`<stdin>`
when reading a pipe, `<unknown>` under `-n`), and flags a raised payload that is
not a string, as jq does:

```bash
echo '{"x":1}' | succinctly jq 'error({"a":1})'
# stderr: jq: error (at <stdin>:1) (not a string): {"a":1}
```

Codes 5 and `-e`'s 1/4 answer different questions and are not interchangeable:
5 means the filter *failed*, while `-e`'s codes describe a filter that succeeded
with a falsy or empty result. An error therefore outranks `-e`. A caught error
(`try`/`catch`) is not a failure and exits 0.

One divergence from jq is deliberate: jq's exit code reflects only the *last*
input, so an error on any earlier input exits 0. `succinctly jq` exits 5 if any
input raised, which is the point of having the code at all.

### Examples

```bash
# Compact output
succinctly jq -c '.users[]' data.json

# Raw string output
succinctly jq -r '.name' user.json

# Sort keys for diff-friendly output
succinctly jq -S . config.json

# Multiple files
succinctly jq '.version' package1.json package2.json

# Pipe from stdin
cat data.json | succinctly jq '.items[]'

# Output matches jq by default
succinctly jq . input.json | diff - <(jq . input.json)

# Validate JSON before processing (RFC 8259 strict)
succinctly jq --validate '.users[]' input.json
```

### Assignment Operators

The jq command supports assignment operators for modifying JSON in-place:

```bash
# Simple assignment
echo '{"a": 1}' | succinctly jq '.a = 42'
# Output: {"a": 42}

# Update assignment (applies filter to current value)
echo '{"x": 5}' | succinctly jq '.x |= . * 2'
# Output: {"x": 10}

# Compound assignment (+=, -=, *=, /=, %=)
echo '{"count": 10}' | succinctly jq '.count += 5'
# Output: {"count": 15}

echo '{"value": 100}' | succinctly jq '.value -= 25'
# Output: {"value": 75}

# Alternative assignment (sets only if null/false)
echo '{"a": null}' | succinctly jq '.a //= "default"'
# Output: {"a": "default"}

echo '{"a": "existing"}' | succinctly jq '.a //= "default"'
# Output: {"a": "existing"}  (unchanged)

# Delete field or array element
echo '{"a": 1, "b": 2}' | succinctly jq 'del(.a)'
# Output: {"b": 2}

echo '[1, 2, 3]' | succinctly jq 'del(.[1])'
# Output: [1, 3]

# Update all array elements
echo '[1, 2, 3]' | succinctly jq '.[] |= . * 2'
# Output: [2, 4, 6]

# Chained assignments
echo '{"x": 0, "y": 0}' | succinctly jq '.x = 10 | .y = 20'
# Output: {"x": 10, "y": 20}

# Nested assignment
echo '{"user": {"name": "Alice", "age": 30}}' | succinctly jq '.user.age += 1'
# Output: {"user": {"name": "Alice", "age": 31}}
```

| Operator | Syntax             | Description                                         |
|----------|--------------------|-----------------------------------------------------|
| `=`      | `.path = value`    | Simple assignment                                   |
| `\|=`    | `.path \|= filter` | Update assignment (apply filter to current value)   |
| `+=`     | `.path += value`   | Add to current value                                |
| `-=`     | `.path -= value`   | Subtract from current value                         |
| `*=`     | `.path *= value`   | Multiply current value                              |
| `/=`     | `.path /= value`   | Divide current value                                |
| `%=`     | `.path %= value`   | Modulo of current value                             |
| `//=`    | `.path //= value`  | Set only if current value is null or false          |
| `del()`  | `del(.path)`       | Delete field or array element                       |

## jq-locate Command

Find the jq expression for a position in a JSON file. Useful for editor integration and debugging.

```bash
succinctly jq-locate <FILE> [OPTIONS]
```

### Options

- `--offset <OFFSET>`: Byte offset in file (0-indexed)
- `--line <LINE>`: Line number (1-indexed)
- `--column <COLUMN>`: Column number (1-indexed, byte offset within line)
- `--format <FORMAT>`: Output format: `expression` (default) or `json`

### Examples

```bash
# Find expression by byte offset
succinctly jq-locate input.json --offset 42
# Output: .users[0].name

# Find expression by line/column
succinctly jq-locate input.json --line 5 --column 10
# Output: .config.version

# Detailed JSON output
succinctly jq-locate input.json --offset 42 --format json
# Output: {"expression":".users[0].name","type":"string","start":38,"end":52}
```

---

## yq-locate Command

Find the yq expression for a position in a YAML file. Useful for editor integration and debugging.

```bash
succinctly yq-locate <FILE> [OPTIONS]
```

### Options

- `--offset <OFFSET>`: Byte offset in file (0-indexed)
- `--line <LINE>`: Line number (1-indexed)
- `--column <COLUMN>`: Column number (1-indexed, byte offset within line)
- `--format <FORMAT>`: Output format: `expression` (default) or `json`

### Examples

```bash
# Find expression by byte offset
succinctly yq-locate config.yaml --offset 42
# Output: .users[0].name

# Find expression by line/column
succinctly yq-locate config.yaml --line 5 --column 10
# Output: .spec.containers[0]

# Detailed JSON output
succinctly yq-locate config.yaml --offset 42 --format json
# Output: {"expression":".users[0].name","type":"string","start":38,"end":52}
```

Unlike `succinctly yq`/`yq --validate`/`at_offset`, which all reject a YAML document
containing invalid UTF-8 at the input boundary, `yq-locate` deliberately keeps answering on
one: warns to stderr and continues, rather than refusing outright, for the byte in question
(#1627). Asking "what's at byte N" of a file a parser just rejected is the normal case for
a diagnostic tool, not an edge case. This doesn't cover every invalid byte in the document,
though: if the byte falls inside a *mapping key* on the path to the requested offset, the
path expression still can't be built at all (a separate, pre-existing limitation in how
`yq-locate` renders keys, not fixed by #1627) and the command still fails — see #1759.

---

## Environment Variables

A command-line flag always beats an environment variable.

| Variable                    | Applies to | Purpose                                        |
|-----------------------------|------------|------------------------------------------------|
| `SUCCINCTLY_PRESERVE_INPUT` | `jq`       | Keep the input's number and escape formatting  |
| `NO_COLOR`                  | `jq`, `yq` | Disable colored output                         |
| `JQ_COLORS`                 | `jq`       | Customize syntax highlighting colors           |
| `JQ_LIBRARY_PATH`           | `jq`       | Directories to search for modules              |
| `HOME`                      | `jq`       | Locates `~/.jq`, which is loaded automatically |
| `TZ`                        | `jq`, `yq` | Timezone for the date builtins                 |

Queries can read any variable via `env`, `$ENV`, `env(NAME)` and `strenv(NAME)`.

See [Environment Variables](../reference/environment-variables.md) for accepted values, precedence,
and the library-only `SUCCINCTLY_SIMD` and `SUCCINCTLY_SVE2`.

---

## Development

Run tests for the CLI:

```bash
cargo test --features cli --bin succinctly
```

Build in debug mode:

```bash
cargo build --features cli
./target/debug/succinctly json generate 1kb
```
