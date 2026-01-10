# Succinctly CLI Tool

Command-line utility for working with succinct data structures.

## Installation

Build the CLI tool with the `cli` feature:

```bash
cargo build --release --features cli
```

The binary will be located at `target/release/succinctly`.

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

### jq Compatibility Mode

By default, succinctly preserves the original number formatting from the input (e.g., `4e4` stays as `4e4`). To get output that exactly matches jq's formatting:

```bash
# Using flag
succinctly jq --jq-compat . input.json

# Using environment variable
SUCCINCTLY_JQ_COMPAT=1 succinctly jq . input.json
```

The `--jq-compat` flag normalizes:
- **Numbers**: `4e4` → `4E+4`, `12e2` → `1.2E+3`, `1e-3` → `0.001`
- **Trailing zeros**: preserved (`0.10` → `0.10`)
- **Escape sequences**: `\u0008` → `\b`, `\u000c` → `\f`

### Input Options

- `-n, --null-input`: Don't read any input; use null as the single input value
- `-R, --raw-input`: Read each line as a string instead of JSON
- `-s, --slurp`: Read all inputs into an array

### Variables

- `--arg NAME VALUE`: Set $NAME to the string VALUE
- `--argjson NAME VALUE`: Set $NAME to the JSON VALUE
- `--slurpfile NAME FILE`: Set $NAME to an array of JSON values from FILE
- `--rawfile NAME FILE`: Set $NAME to the string contents of FILE

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

# jq-compatible output for comparison
succinctly jq --jq-compat . input.json | diff - <(jq . input.json)
```

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
