# Plan: jq Expression Support

## Overview

Implement a subset of jq query expressions on top of the existing JSON semi-indexing infrastructure. The goal is to leverage the fast cursor-based navigation and lazy evaluation already in place.

## Scope

### Phase 1: Core Path Expressions (MVP)

Support basic jq path syntax for navigating JSON:

| Expression | Meaning              | Implementation                          |
|------------|----------------------|-----------------------------------------|
| `.`        | Identity (root)      | `index.root(text)`                      |
| `.foo`     | Object field access  | `JsonFields::find("foo")`               |
| `.[0]`     | Array index access   | `JsonElements::get(0)`                  |
| `.foo.bar` | Chained field access | Compose cursor navigation               |
| `.[]`      | Iterate all elements | `JsonElements` / `JsonFields` iterators |
| `.foo[]`   | Field then iterate   | Compose field access + iteration        |
| `.[2:5]`   | Array slice          | Iterate with skip/take                  |

### Phase 2: Filters and Conditionals

| Expression        | Meaning                          |
|-------------------|----------------------------------|
| `select(expr)`    | Filter based on condition        |
| `.foo?`           | Optional field (null if missing) |
| `.foo // default` | Alternative operator             |
| `type`            | Get value type as string         |
| `length`          | Array/string/object length       |

### Phase 3: Operators and Functions

| Expression      | Meaning                |
|-----------------|------------------------|
| `keys`          | Object keys as array   |
| `values`        | Object values as array |
| `has("foo")`    | Check if field exists  |
| `in(obj)`       | Check if key in object |
| `map(expr)`     | Transform each element |
| `first`, `last` | First/last element     |
| `nth(n)`        | Nth element            |

### Phase 4: Advanced (Future)

- Recursive descent `..`
- Pipe operator `|`
- Arithmetic and comparison operators
- String interpolation
- `reduce`, `group_by`, `sort_by`

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      CLI Layer                          │
│  succinctly json query <file> '<expr>'                  │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                    Query Engine                         │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐  │
│  │   Parser    │→ │  Expr AST   │→ │   Evaluator     │  │
│  │ (jq syntax) │  │             │  │ (cursor-based)  │  │
│  └─────────────┘  └─────────────┘  └─────────────────┘  │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│              Existing Infrastructure                    │
│  JsonIndex, JsonCursor, BalancedParens, StandardJson    │
└─────────────────────────────────────────────────────────┘
```

---

## File Structure

```
src/
├── jq/
│   ├── mod.rs          # Module root, public API
│   ├── expr.rs         # Expression AST types
│   ├── parser.rs       # jq expression parser
│   ├── eval.rs         # Expression evaluator
│   └── output.rs       # Result formatting (JSON output)
└── bin/succinctly/
    └── main.rs         # Add 'query' subcommand
```

---

## Implementation Details

### Step 1: Expression AST (`src/jq/expr.rs`)

```rust
/// A jq expression
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Identity: `.`
    Identity,

    /// Field access: `.foo`
    Field(String),

    /// Index access: `.[0]` or `.[-1]`
    Index(i64),

    /// Slice: `.[2:5]` or `.[2:]` or `.[:5]`
    Slice(Option<i64>, Option<i64>),

    /// Iterate: `.[]`
    Iterate,

    /// Optional: `.foo?`
    Optional(Box<Expr>),

    /// Chain: `.foo.bar` or `.foo[0]`
    Pipe(Vec<Expr>),

    /// Try with alternative: `.foo // "default"`
    TryAlt(Box<Expr>, Box<Expr>),
}
```

### Step 2: Parser (`src/jq/parser.rs`)

Hand-written recursive descent parser (no external dependencies):

```rust
pub fn parse(input: &str) -> Result<Expr, ParseError>;

// Grammar (simplified):
// expr     := pipe
// pipe     := term ('.' term)*
// term     := field | index | slice | iterate | optional
// field    := IDENT
// index    := '[' NUMBER ']'
// slice    := '[' NUMBER? ':' NUMBER? ']'
// iterate  := '[' ']'
// optional := term '?'
```

### Step 3: Evaluator (`src/jq/eval.rs`)

```rust
/// Result of evaluating an expression
pub enum QueryResult<'a, W> {
    /// Single value
    One(StandardJson<'a, W>),

    /// Multiple values (from iteration)
    Many(Vec<StandardJson<'a, W>>),

    /// No result (filtered out or missing optional)
    None,

    /// Error during evaluation
    Error(String),
}

/// Evaluate expression against JSON
pub fn eval<'a, W>(
    expr: &Expr,
    cursor: JsonCursor<'a, W>,
) -> QueryResult<'a, W>;
```

Key implementation notes:

1. **Field access** (`.foo`):
   ```rust
   match cursor.value() {
       StandardJson::Object(fields) => {
           match fields.find(name) {
               Some(value) => QueryResult::One(value),
               None => QueryResult::Error("field not found"),
           }
       }
       _ => QueryResult::Error("not an object"),
   }
   ```

2. **Index access** (`.[n]`):
   ```rust
   match cursor.value() {
       StandardJson::Array(elements) => {
           let idx = if n < 0 { /* handle negative */ } else { n as usize };
           match elements.get(idx) {
               Some(value) => QueryResult::One(value),
               None => QueryResult::Error("index out of bounds"),
           }
       }
       _ => QueryResult::Error("not an array"),
   }
   ```

3. **Iteration** (`.[]`):
   ```rust
   match cursor.value() {
       StandardJson::Array(elements) => {
           QueryResult::Many(elements.collect())
       }
       StandardJson::Object(fields) => {
           QueryResult::Many(fields.map(|f| f.value()).collect())
       }
       _ => QueryResult::Error("cannot iterate"),
   }
   ```

4. **Chained expressions** (`.foo.bar`):
   - Evaluate left-to-right
   - For `Many` results, apply next expression to each element

### Step 4: CLI Integration (`src/bin/succinctly/main.rs`)

```rust
#[derive(Debug, Subcommand)]
enum JsonSubcommand {
    Generate(GenerateJson),
    GenerateSuite(GenerateSuite),
    /// Query JSON using jq-like expressions
    Query(QueryJson),
}

#[derive(Debug, Parser)]
struct QueryJson {
    /// Input file (use - for stdin)
    input: PathBuf,

    /// jq expression
    #[arg(short, long, default_value = ".")]
    expr: String,

    /// Output format
    #[arg(short, long, default_value = "json")]
    format: OutputFormat,

    /// Compact output (no pretty-printing)
    #[arg(short, long)]
    compact: bool,

    /// Raw string output (no quotes for strings)
    #[arg(short, long)]
    raw: bool,
}

#[derive(Debug, Clone, ValueEnum)]
enum OutputFormat {
    Json,   // Default JSON output
    Lines,  // One value per line
    Csv,    // CSV for arrays of objects
}
```

Usage:
```bash
# Basic field access
succinctly json query data.json -e '.name'

# Array iteration
succinctly json query data.json -e '.users[].email'

# Chained access
succinctly json query data.json -e '.config.database.host'

# Array slicing
succinctly json query data.json -e '.items[0:10]'
```

### Step 5: Output Formatting (`src/jq/output.rs`)

```rust
pub fn format_result<W>(
    result: &QueryResult<W>,
    format: OutputFormat,
    compact: bool,
    raw: bool,
) -> String;
```

- JSON: Serialize back to JSON (pretty or compact)
- Lines: One value per line (useful for piping)
- Raw: Strings without quotes (useful for shell scripts)

---

## Testing Strategy

### Unit Tests

1. **Parser tests** (`src/jq/parser.rs`):
   - Valid expressions parse correctly
   - Invalid expressions produce helpful errors
   - Edge cases: empty string, whitespace, unicode

2. **Evaluator tests** (`src/jq/eval.rs`):
   - Each expression type against sample JSON
   - Error handling for type mismatches
   - Chained expressions

### Integration Tests

```rust
// tests/jq_tests.rs
#[test]
fn test_field_access() {
    let json = br#"{"name": "Alice", "age": 30}"#;
    let result = query(json, ".name");
    assert_eq!(result, r#""Alice""#);
}

#[test]
fn test_array_iteration() {
    let json = br#"[1, 2, 3]"#;
    let result = query(json, ".[]");
    assert_eq!(result, "1\n2\n3");
}
```

### CLI Golden Tests

Add to `tests/cli_golden_tests.rs`:
- Query command with various expressions
- Error messages for invalid queries
- Different output formats

---

## Performance Considerations

1. **Lazy evaluation**: Don't decode strings until needed for output
2. **Streaming iteration**: Use iterators instead of collecting to Vec where possible
3. **Zero-copy navigation**: Leverage existing cursor-based API
4. **Early termination**: For `first`, `.[0]`, etc., stop after finding result

---

## Estimated Effort

| Component         | Lines of Code | Complexity |
|-------------------|---------------|------------|
| Expression AST    | ~50           | Low        |
| Parser            | ~200          | Medium     |
| Evaluator         | ~300          | Medium     |
| Output formatting | ~100          | Low        |
| CLI integration   | ~50           | Low        |
| Tests             | ~300          | Low        |
| **Total**         | **~1000**     | Medium     |

---

## Open Questions

1. **Error handling**: Should missing fields return null (like jq) or error?
   - Recommendation: Error by default, `?` suffix for optional

2. **Multiple outputs**: How to handle `.[]` producing multiple values?
   - Recommendation: Newline-separated by default, `--slurp` to collect into array

3. **Streaming**: Should we support streaming for very large files?
   - Recommendation: Defer to Phase 2, current design works well for indexed files

4. **Compatibility**: How closely to match jq semantics?
   - Recommendation: Match common cases, document differences

---

## Success Criteria

1. Parse and evaluate basic path expressions (`.foo.bar[0]`)
2. Support iteration with `.[]`
3. CLI command works with files and stdin
4. Performance: Query 10MB JSON in <100ms (after indexing)
5. Helpful error messages for parse/eval failures
