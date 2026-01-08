# jq Query Language Implementation Plan

This document outlines a comprehensive plan to implement the full jq query language on top of succinctly's JSON semi-indexing infrastructure.

## Current Implementation Status

### What's Implemented
- `.` - Identity
- `.foo` - Field access
- `.[n]` - Array index (positive and negative)
- `.[n:m]`, `.[n:]`, `.[:m]` - Array slicing
- `.[]` - Iterate all elements
- `.foo?` - Optional access
- `.foo.bar`, `.foo[0].bar` - Chained/piped expressions

### What's NOT Implemented
Everything else from jq 1.8, which is substantial.

---

## Phase 1: Core Operators (Foundation)

**Goal**: Implement fundamental operators that most jq programs use.

### 1.1 Comma Operator (Multiple Outputs)
```jq
.foo, .bar        # Outputs both .foo and .bar
.[0], .[1]        # Multiple indices
```

**Challenge**: Current `QueryResult::Many` handles iteration output, but comma creates parallel outputs from a single input. Need to ensure the evaluation model correctly fans out.

**Implementation**:
- Add `Expr::Comma(Vec<Expr>)` to AST
- Parser: Handle `,` as lowest-precedence binary operator
- Evaluator: For each input, run all comma branches and concatenate outputs

### 1.2 Parentheses (Grouping)
```jq
(.foo + .bar)     # Group for precedence
```

**Implementation**: Parser change only - parentheses affect parsing precedence but don't need an AST node.

### 1.3 Recursive Descent (`..`)
```jq
..                # Recursively descend into all values
.. | .foo?        # Find all "foo" fields at any depth
```

**Challenge**: Must traverse the entire JSON tree. With succinct structures, this means walking the BP tree.

**Implementation**:
- Add `Expr::RecursiveDescent`
- Walk the BP tree using `first_child`, `next_sibling`, yielding all values

---

## Phase 2: Arithmetic & Comparison

**Goal**: Enable computations on JSON values.

### 2.1 Arithmetic Operators
```jq
.a + .b           # Addition (numbers, strings, arrays, objects)
.a - .b           # Subtraction (numbers, arrays, objects)
.a * .b           # Multiplication (numbers, string*number, object merge)
.a / .b           # Division (numbers, string split)
.a % .b           # Modulo (numbers only)
```

**Challenge**: **This is the first major architectural challenge.**

Current design operates on cursors pointing into the original JSON bytes - it's zero-copy. Arithmetic requires:
1. Materializing values from the JSON
2. Performing computation
3. Creating new JSON values

**Options**:
1. **Materialized Value Type**: Create a `JsonValue` enum that can hold owned values
2. **Hybrid Approach**: Keep cursor-based for reads, materialize only when computing
3. **Full Materialization**: Always convert to owned values (loses performance benefit)

**Recommended**: Hybrid approach with a new `Value` type:
```rust
enum Value<'a, W> {
    /// Reference to original JSON (zero-copy)
    Ref(StandardJson<'a, W>),
    /// Materialized/computed value
    Owned(OwnedJson),
}

enum OwnedJson {
    Null,
    Bool(bool),
    Number(f64),  // or separate i64/f64
    String(String),
    Array(Vec<OwnedJson>),
    Object(BTreeMap<String, OwnedJson>),
}
```

### 2.2 Comparison Operators
```jq
.a == .b          # Equality
.a != .b          # Inequality
.a < .b           # Less than
.a <= .b          # Less than or equal
.a > .b           # Greater than
.a >= .b          # Greater than or equal
```

**Implementation**: Compare materialized values using jq's comparison semantics (null < bool < number < string < array < object).

### 2.3 Boolean Operators
```jq
.a and .b         # Logical AND
.a or .b          # Logical OR
not               # Logical NOT
```

**Note**: jq's `and`/`or` are short-circuiting and work on truthiness (false and null are falsy).

---

## Phase 3: Conditionals & Control Flow

### 3.1 If-Then-Else
```jq
if .foo then .bar else .baz end
if .a then .b elif .c then .d else .e end
```

**Implementation**:
- Add `Expr::If { cond: Box<Expr>, then: Box<Expr>, else_: Box<Expr> }`
- Parser: Handle `if`, `then`, `elif`, `else`, `end` keywords

### 3.2 Try-Catch
```jq
try .foo catch "default"
try .foo          # Suppresses errors, outputs nothing on error
```

**Implementation**:
- Add `Expr::Try { expr: Box<Expr>, catch: Option<Box<Expr>> }`
- Evaluator: Catch `QueryResult::Error` and substitute

### 3.3 Alternative Operator
```jq
.foo // "default"   # If .foo is false/null, use "default"
```

**Note**: This is NOT null-coalescing - it triggers on falsy values (false or null).

### 3.4 Error Raising
```jq
error             # Raise error with null
error("message")  # Raise error with message
```

---

## Phase 4: Core Builtin Functions

### 4.1 Type Functions
```jq
type              # Returns "null", "boolean", "number", "string", "array", "object"
isnull, isboolean, isnumber, isstring, isarray, isobject
isnormal, isinfinite, isnan  # Number classification
infinite, nan     # Special number constants
```

**Implementation**: Straightforward type checks on values.

### 4.2 Length & Keys
```jq
length            # String length, array length, object key count, null=0
utf8bytelength    # Byte length of string
keys              # Object keys as sorted array
keys_unsorted     # Object keys in original order
has("key")        # Check if object has key or array has index
in(obj)           # Check if key exists in object
```

**Challenge**: `keys` needs to extract and sort strings. With cursor-based access, we need to materialize keys.

### 4.3 Selection & Filtering
```jq
select(condition) # Output input only if condition is true
empty             # Output nothing
```

**Implementation**: `select` evaluates condition, outputs input if truthy, otherwise outputs nothing.

### 4.4 Map & Iteration
```jq
map(f)            # Apply f to each element: [.[] | f]
map_values(f)     # Apply f to each object value
```

**Note**: `map(f)` is syntactic sugar for `[.[] | f]`.

### 4.5 Reduction
```jq
add               # Sum array elements (works for numbers, strings, arrays)
any               # True if any element is truthy
all               # True if all elements are truthy
min, max          # Minimum/maximum element
min_by(f), max_by(f)
```

---

## Phase 5: Array & Object Functions

### 5.1 Array Operations
```jq
first             # First element: .[0]
last              # Last element: .[-1]
nth(n)            # Nth element
first(expr), last(expr), nth(n; expr)  # First/last/nth output of expr
range(n), range(a;b), range(a;b;step)  # Generate numbers
reverse           # Reverse array
contains(b)       # True if a contains b (recursive)
inside(b)         # True if a is inside b
flatten           # Flatten nested arrays
flatten(depth)    # Flatten to specific depth
group_by(f)       # Group by key
unique            # Remove duplicates
unique_by(f)      # Remove duplicates by key
sort              # Sort array
sort_by(f)        # Sort by key
```

**Challenge**: Many of these require materializing arrays. `sort_by` requires evaluating an expression for each element.

### 5.2 Object Operations
```jq
to_entries        # {k:v} → [{key:k, value:v}]
from_entries      # [{key:k, value:v}] → {k:v}
with_entries(f)   # to_entries | map(f) | from_entries
paths             # All paths to leaf values
paths(filter)     # Paths to values matching filter
getpath(path)     # Get value at path
setpath(path; v)  # Set value at path
delpaths(paths)   # Delete multiple paths
leaf_paths        # Paths to scalar values
```

**Challenge**: `setpath`/`delpaths` require **mutation semantics**. Since our JSON is read-only, these must create new materialized objects.

### 5.3 Object Construction
```jq
{foo: .bar}       # Object construction
{(.key): .value}  # Dynamic key
{user, name}      # Shorthand: {user: .user, name: .name}
```

**Implementation**:
- Add `Expr::Object(Vec<(KeyExpr, Expr)>)` where `KeyExpr` is either literal or dynamic
- Construct `OwnedJson::Object` at evaluation time

### 5.4 Array Construction
```jq
[.foo, .bar]      # Array construction
[.[] | select(.active)]  # Array from filter
```

**Implementation**: `Expr::Array(Box<Expr>)` - collect all outputs into an array.

---

## Phase 6: String Operations

### 6.1 Basic String Functions
```jq
ascii_downcase, ascii_upcase
ltrimstr(s), rtrimstr(s)
startswith(s), endswith(s)
split(s)          # Split string into array
join(s)           # Join array into string
inside(s)         # String containment
```

### 6.2 String Interpolation
```jq
"Hello \(.name)"  # String interpolation
```

**Implementation**:
- Parser: Handle `\(expr)` inside strings
- Add `Expr::StringInterpolation(Vec<StringPart>)` where `StringPart` is `Literal(String) | Expr(Box<Expr>)`

### 6.3 String Formatting
```jq
@text, @json      # Format as text/JSON
@uri, @csv, @tsv  # URI encode, CSV/TSV format
@base64, @base64d # Base64 encode/decode
@html             # HTML entity escape
@sh               # Shell quote
```

**Challenge**: These require implementing various encoding algorithms.

---

## Phase 7: Regular Expressions

```jq
test(re)          # Test if regex matches
test(re; flags)
match(re)         # Return match object
match(re; flags)
capture(re)       # Return named captures as object
scan(re)          # Find all matches
split(re)         # Split by regex
sub(re; s)        # Replace first match
gsub(re; s)       # Replace all matches
```

**Major Challenge**: jq uses Oniguruma regex. Options:
1. **Add regex dependency**: Use `regex` crate (adds ~500KB to binary)
2. **Subset implementation**: Implement basic patterns only
3. **Feature flag**: `regex` feature enables these functions

**Recommendation**: Feature-gated `regex` crate dependency.

---

## Phase 8: Advanced Control Flow

### 8.1 Reduce
```jq
reduce .[] as $x (0; . + $x)   # Fold/reduce over values
```

**Implementation**:
- Add `Expr::Reduce { input: Box<Expr>, var: String, init: Box<Expr>, update: Box<Expr> }`
- Requires variable binding support

### 8.2 Foreach
```jq
foreach .[] as $x (0; . + 1)   # Like reduce but outputs intermediate
foreach .[] as $x (0; . + 1; .)  # With extract expression
```

### 8.3 Recursion
```jq
recurse           # Recursively apply .[]
recurse(f)        # Recursively apply f
recurse(f; cond)  # Recurse while condition holds
walk(f)           # Apply f to all values bottom-up
```

### 8.4 Limit & Control
```jq
limit(n; expr)    # Take first n outputs
first(expr)       # First output only
last(expr)        # Last output only (must collect all)
until(cond; update)  # Loop until condition
while(cond; update)  # Loop while condition
repeat(expr)      # Infinite repetition
```

---

## Phase 9: Variables & Definitions

### 9.1 Variable Binding
```jq
.foo as $x | .bar + $x
.foo as $x | .bar as $y | $x + $y
```

**Implementation**:
- Add variable environment to evaluator: `HashMap<String, Value>`
- Add `Expr::As { expr: Box<Expr>, var: String, body: Box<Expr> }`

### 9.2 Destructuring
```jq
. as {name: $n, age: $a} | ...
. as [$first, $second] | ...
```

**Challenge**: Pattern matching with extraction.

### 9.3 Function Definitions
```jq
def double: . * 2;
def addone($x): . + $x;
def f($a; $b): $a + $b;
```

**Implementation**:
- Add `Expr::FuncDef { name: String, params: Vec<String>, body: Box<Expr>, then: Box<Expr> }`
- Function call: `Expr::FuncCall { name: String, args: Vec<Expr> }`
- Evaluator maintains function table

---

## Phase 10: I/O & Special Functions

### 10.1 Input/Output
```jq
input             # Read next input
inputs            # Read all remaining inputs
debug             # Output to stderr
```

**Challenge**: These require runtime context (input stream, stderr handle). Need to add I/O context to evaluator.

### 10.2 Path Expressions
```jq
path(.foo.bar)    # Return path as array: ["foo", "bar"]
getpath(["foo"])  # Get value at path
```

### 10.3 Type Conversions
```jq
tonumber          # Convert string to number
tostring          # Convert to string
tojsonstream      # Convert to stream format
fromjsonstream    # Convert from stream format
```

### 10.4 Date/Time
```jq
now               # Current Unix timestamp
todate            # Number to ISO 8601
fromdate          # ISO 8601 to number
strftime(fmt)     # Format date
strptime(fmt)     # Parse date
```

**Challenge**: Date formatting requires either external dependency or manual implementation.

---

## Architecture Decisions

### Decision 1: Value Representation

```rust
/// A JSON value that can be either a reference or owned
pub enum Value<'a, W = Vec<u64>> {
    /// Zero-copy reference to original JSON
    Ref(StandardJson<'a, W>),
    /// Computed/materialized value
    Owned(OwnedValue),
}

/// Owned JSON value for computed results
#[derive(Debug, Clone, PartialEq)]
pub enum OwnedValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<OwnedValue>),
    Object(indexmap::IndexMap<String, OwnedValue>),
}
```

**Rationale**:
- Preserves zero-copy performance for navigation-only queries
- Enables arithmetic and construction operations
- `IndexMap` preserves insertion order for objects

### Decision 2: Error Handling

jq has complex error semantics:
- Some errors propagate (type errors)
- Some errors are suppressed (`?` operator)
- `try-catch` catches errors

```rust
pub enum EvalResult<'a, W> {
    /// Successful values
    Values(Vec<Value<'a, W>>),
    /// Empty (no output, not an error)
    Empty,
    /// Error that can be caught
    Error(JqError),
}
```

### Decision 3: Variable Scoping

```rust
struct EvalContext<'a, W> {
    /// Variable bindings (lexically scoped)
    vars: HashMap<String, Value<'a, W>>,
    /// User-defined functions
    funcs: HashMap<String, FuncDef>,
    /// I/O context for input/debug
    io: Option<&'a mut dyn IoContext>,
}
```

---

## Implementation Challenges Summary

### High Priority Challenges

1. **Value Materialization**: Converting between zero-copy cursors and owned values is the biggest architectural change. Must be efficient for the common case (navigation-only queries shouldn't pay the cost).

2. **Multiple Outputs**: jq's model where every expression can produce 0, 1, or many outputs complicates evaluation. The comma operator and iteration both produce multiple outputs that must flow through the pipeline.

3. **Variable Binding**: Adding lexical scoping changes the evaluator from stateless to stateful.

### Medium Priority Challenges

4. **Object/Array Construction**: Building new JSON values requires materialization.

5. **Regex Support**: Adds significant dependency, should be feature-gated.

6. **User-Defined Functions**: Requires function table and recursion support.

### Lower Priority Challenges

7. **Date/Time**: Can be feature-gated or deferred.

8. **Format Strings (@uri, @csv, etc.)**: Straightforward but tedious.

9. **Module System**: jq supports `import` and `include` - likely out of scope.

---

## Recommended Implementation Order

### Sprint 1: Foundation
1. Value type (hybrid cursor/owned)
2. Comma operator
3. Parentheses
4. Array/Object construction `[...]`, `{...}`

### Sprint 2: Computation
5. Arithmetic operators (+, -, *, /, %)
6. Comparison operators (==, !=, <, <=, >, >=)
7. Boolean operators (and, or, not)
8. Alternative operator (//)

### Sprint 3: Control Flow
9. If-then-else
10. Try-catch
11. select(), empty

### Sprint 4: Core Builtins
12. type, length, keys
13. map, add, first, last
14. sort, reverse, unique
15. split, join

### Sprint 5: Variables
16. Variable binding (as)
17. reduce
18. foreach

### Sprint 6: Advanced
19. User-defined functions
20. Recursive descent (..)
21. walk, recurse

### Sprint 7: Optional Features
22. Regex (feature-gated)
23. Date/time (feature-gated)
24. Format strings

---

## Testing Strategy

1. **Unit tests per operator**: Each new operator gets targeted tests
2. **jq compatibility tests**: Run same queries through jq and succinctly, compare outputs
3. **Property tests**: Random expression generation with proptest
4. **Performance regression**: Ensure navigation-only queries stay fast

---

## Compatibility Notes

### Intentionally NOT implementing:
- `$ENV` - Environment variable access
- `@sh`, `@base64d` - Security-sensitive operations
- `input`, `inputs` - Streaming input (different execution model)
- `import`, `include` - Module system
- SQL-like operators (`INDEX`, `IN`, `GROUP BY`)

### Differences from jq:
- Numbers: jq uses IEEE 754 doubles; we might want i64 for integers
- Streaming: jq processes line-by-line; we process whole documents
- Error messages: Will differ but should be equally informative

---

## Estimated Scope

| Phase | Features              | Complexity | Lines of Code (est.) |
|-------|-----------------------|------------|----------------------|
| 1     | Core operators        | Medium     | ~500                 |
| 2     | Arithmetic/Comparison | Medium     | ~400                 |
| 3     | Control flow          | Medium     | ~300                 |
| 4     | Core builtins         | High       | ~800                 |
| 5     | Array/Object ops      | High       | ~600                 |
| 6     | Strings               | Medium     | ~400                 |
| 7     | Regex                 | Medium     | ~200 + dependency    |
| 8     | Advanced control      | High       | ~500                 |
| 9     | Variables/Functions   | High       | ~600                 |
| 10    | I/O & Special         | Low        | ~300                 |

**Total**: ~4,600 lines of Rust code for full implementation.

---

## Conclusion

Implementing the full jq language is a substantial undertaking. The current implementation covers ~10% of jq's functionality (navigation operations). The key architectural decision is the hybrid value representation that preserves zero-copy performance while enabling computation.

The recommended approach is incremental: each sprint adds useful functionality that can be released and tested independently. Sprint 1-4 would provide a "jq-lite" that covers most common use cases. Sprints 5-10 add advanced features for power users.
