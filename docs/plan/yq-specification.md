# Mike Farah's yq Query Language Specification

## Overview

This document specifies the yq query language as implemented by Mike Farah's yq (https://github.com/mikefarah/yq). The language is jq-inspired but has significant differences and YAML-specific extensions.

## 1. Lexical Elements

### 1.1 Identifiers

Identifiers start with a letter or underscore, followed by letters, digits, or underscores:

```
identifier = [a-zA-Z_][a-zA-Z0-9_]*
```

### 1.2 Literals

| Type | Examples | Notes |
|------|----------|-------|
| Integer | `42`, `-17`, `0` | Tag: `!!int` |
| Float | `3.14`, `-2.5`, `1e10` | Tag: `!!float` |
| String | `"hello"`, `'world'` | Double or single quotes |
| Boolean | `true`, `false` | Tag: `!!bool` |
| Null | `null`, `~` | Tag: `!!null` |

### 1.3 Operators (by precedence, lowest to highest)

| Precedence | Operators | Associativity |
|------------|-----------|---------------|
| 1 | `\|` (pipe) | Left |
| 2 | `,` (union) | Left |
| 3 | `//` (alternative) | Left |
| 4 | `or` | Left |
| 5 | `and` | Left |
| 6 | `==`, `!=` | Left |
| 7 | `<`, `<=`, `>`, `>=` | Left |
| 8 | `+`, `-` | Left |
| 9 | `*`, `/`, `%` | Left |
| 10 | `not` (prefix) | Right |

### 1.4 Assignment Operators

| Operator | Meaning |
|----------|---------|
| `=` | Plain assignment (RHS evaluated against pipeline context) |
| `\|=` | Update assignment (RHS evaluated with LHS as context) |
| `+=` | Add and assign |
| `-=` | Subtract and assign |
| `*=` | Multiply/merge and assign |
| `/=` | Divide and assign |

### 1.5 Special Syntax

| Syntax | Meaning |
|--------|---------|
| `.` | Identity (current node) |
| `..` | Recursive descent (values only) |
| `...` | Recursive descent (keys and values) |
| `$var` | Variable reference |
| `@format` | Format function (e.g., `@json`, `@base64`) |

## 2. Path Expressions

### 2.1 Field Access

```yaml
.field           # Simple field access
.field1.field2   # Nested field access
.["field"]       # Bracket notation (for special chars)
.["key.with.dots"]  # Keys with dots
.["key with spaces"] # Keys with spaces
```

### 2.2 Array Access

```yaml
.[0]             # First element (0-indexed)
.[-1]            # Last element
.[2:5]           # Slice from index 2 to 5 (exclusive)
.[:3]            # First 3 elements
.[2:]            # From index 2 to end
.[1:-1]          # Exclude first and last
.[0, 2, 4]       # Multiple specific indices
```

### 2.3 Iteration

```yaml
.[]              # Iterate all elements (array or map values)
.[]?             # Optional iterate (no error on scalar)
.field[]         # Iterate field's elements
```

### 2.4 Optional Access

```yaml
.field?          # Returns null if missing (no error)
.[0]?            # Optional array access
```

### 2.5 Wildcard Patterns

```yaml
.a."*b*"         # Keys containing "b"
.a."prefix*"     # Keys starting with "prefix"
.a."*suffix"     # Keys ending with "suffix"
```

## 3. Operators

### 3.1 Arithmetic Operators

| Operator | Numbers | Strings | Arrays | Maps |
|----------|---------|---------|--------|------|
| `+` | Addition | Concatenation | Concatenation | Shallow merge |
| `-` | Subtraction | N/A | Remove elements | N/A |
| `*` | Multiplication | Repeat | N/A | Deep merge |
| `/` | Division | N/A | N/A | N/A |
| `%` | Modulo | N/A | N/A | N/A |

### 3.2 Comparison Operators

All return boolean. Compare same types only.

| Operator | Meaning |
|----------|---------|
| `==` | Equal (supports wildcards in strings) |
| `!=` | Not equal |
| `>` | Greater than |
| `>=` | Greater than or equal |
| `<` | Less than |
| `<=` | Less than or equal |

**Wildcard equality**: `"hello" == "*llo"` returns `true`

### 3.3 Boolean Operators

| Operator | Meaning |
|----------|---------|
| `and` | Logical AND |
| `or` | Logical OR |
| `not` | Logical NOT |

**Truthiness rules**:
- `null` is falsy
- `false` is falsy
- Everything else is truthy (including empty string, 0)

### 3.4 Merge Operator Flags

The `*` operator accepts flags for merge behavior:

| Flag | Meaning |
|------|---------|
| `+` | Append arrays instead of replacing |
| `d` | Deep merge arrays by index |
| `?` | Only merge existing fields |
| `n` | Only merge new fields |
| `c` | Clobber custom tags |

Example: `.a *+? .b` (append arrays, existing fields only)

## 4. Built-in Functions

### 4.1 Type Functions

| Function | Description | Example Output |
|----------|-------------|----------------|
| `kind` | Node kind | `"scalar"`, `"map"`, `"seq"` |
| `tag` | YAML tag | `"!!str"`, `"!!int"`, `"!!map"` |
| `type` | Alias for `tag` | Same as `tag` |
| `is_key` | Is current node a key | `true`, `false` |

### 4.2 Collection Functions

| Function | Description |
|----------|-------------|
| `length` | String length, array/map size |
| `keys` | Array of keys (map) or indices (array) |
| `values` | Array of values |
| `has(key)` | Check if key/index exists |
| `in(obj)` | Check if value is key in object |
| `contains(x)` | Check if contains value/subset |
| `inside(x)` | Inverse of contains |

### 4.3 Array Functions

| Function | Description |
|----------|-------------|
| `first` | First element |
| `sort` | Sort elements |
| `sort_by(exp)` | Sort by expression |
| `reverse` | Reverse order |
| `unique` | Remove duplicates |
| `unique_by(exp)` | Unique by expression |
| `flatten` | Flatten nested arrays |
| `flatten(n)` | Flatten n levels |
| `group_by(exp)` | Group by expression |
| `min` | Minimum value |
| `max` | Maximum value |
| `add` | Sum/concatenate all |
| `any` | Any element true |
| `all` | All elements true |
| `any_c(cond)` | Any satisfies condition |
| `all_c(cond)` | All satisfy condition |
| `shuffle` | Randomize order |
| `nth(n)` | N-th element |

### 4.4 Map Functions

| Function | Description |
|----------|-------------|
| `to_entries` | Convert to `[{key, value}, ...]` |
| `from_entries` | Convert from entries format |
| `with_entries(exp)` | Transform via entries |
| `pick(keys)` | Keep only specified keys |
| `omit(keys)` | Remove specified keys |
| `sort_keys(exp)` | Sort map keys |
| `pivot` | Pivot array of objects |

### 4.5 String Functions

| Function | Description |
|----------|-------------|
| `split(sep)` | Split string to array |
| `join(sep)` | Join array to string |
| `trim` | Remove leading/trailing whitespace |
| `ltrim` | Left trim |
| `rtrim` | Right trim |
| `upcase` | Convert to uppercase |
| `downcase` | Convert to lowercase |
| `capitalize` | Capitalize first letter |
| `test(regex)` | Test regex match (boolean) |
| `match(regex)` | Get match details |
| `capture(regex)` | Get named capture groups |
| `sub(regex; repl)` | Replace first match |
| `sub(regex; repl; "g")` | Replace all matches |

### 4.6 Type Conversion

| Function | Description |
|----------|-------------|
| `to_string` | Convert to string |
| `to_number` | Convert to number |
| `tonumber` | Alias for to_number |
| `tostring` | Alias for to_string |

### 4.7 Path Functions

| Function | Description |
|----------|-------------|
| `path` | Get path to current node |
| `getpath(path)` | Get value at path |
| `setpath(path; val)` | Set value at path |
| `delpaths(paths)` | Delete multiple paths |
| `leaf_paths` | Get all leaf paths |

### 4.8 Navigation Functions

| Function | Description |
|----------|-------------|
| `parent` | Get parent node |
| `parent(n)` | Get n-th ancestor |
| `parents` | Get all ancestors |
| `key` | Get current key |

### 4.9 Format Functions

| Function | Description |
|----------|-------------|
| `@json` | Encode as JSON |
| `@yaml` | Encode as YAML |
| `@base64` | Base64 encode |
| `@base64d` | Base64 decode |
| `@uri` | URI encode |
| `@urid` | URI decode |
| `@csv` | CSV encode (arrays) |
| `@tsv` | TSV encode (arrays) |
| `@sh` | Shell quote |
| `@props` | Properties format |

### 4.10 Control Flow

| Function | Description |
|----------|-------------|
| `select(cond)` | Filter by condition |
| `map(exp)` | Transform array elements |
| `map_values(exp)` | Transform map values |
| `empty` | Produce no output |
| `error(msg)` | Raise error |
| `debug` | Print debug info |

### 4.11 Variable and Reduce

```yaml
# Variable binding
.expr as $var | ...

# Reduce (note: ireduce, not reduce)
.[] as $item ireduce (init; update_expr)
```

### 4.12 Alternative/Default

```yaml
.field // "default"    # Use default if null/false
.a // .b // .c         # Chain of alternatives
```

### 4.13 With Operator

```yaml
with(.path; operations)  # Set context for multiple ops
```

### 4.14 Eval Operator

```yaml
eval(.expr_field)        # Evaluate expression from data
eval(strenv(VAR))        # Evaluate from env var
```

## 5. YAML-Specific Features

### 5.1 Anchor and Alias Operators

| Function | Description |
|----------|-------------|
| `anchor` | Get anchor name |
| `anchor = "name"` | Set anchor name |
| `alias` | Get alias target |
| `alias = "name"` | Set alias target |
| `explode(exp)` | Dereference all aliases |

### 5.2 Comment Operators

| Function | Description |
|----------|-------------|
| `head_comment` | Comment above node |
| `line_comment` | Inline comment |
| `foot_comment` | Comment below node |

Set comments: `.a line_comment = "comment"`

### 5.3 Style Operators

| Style | Description |
|-------|-------------|
| `""` | Default |
| `"double"` | Double quoted |
| `"single"` | Single quoted |
| `"literal"` | Block literal (`\|`) |
| `"folded"` | Block folded (`>`) |
| `"flow"` | Flow/inline |
| `"tagged"` | Show type tag |

Usage: `.a style = "double"` or `.a \| style`

### 5.4 Tag Operators

```yaml
.a tag = "!!str"         # Set tag
.a | tag                 # Get tag
```

### 5.5 Document Operators

| Function | Description |
|----------|-------------|
| `document_index` | Index in multi-doc stream |
| `split_doc` | Split results to documents |

### 5.6 File Operators

| Function | Description |
|----------|-------------|
| `filename` | Current file path |
| `file_index` | Index in file list |
| `load(path)` | Load YAML file |
| `load_str(path)` | Load as string |
| `load_base64(path)` | Load and decode base64 |

### 5.7 Environment Variables

| Function | Description |
|----------|-------------|
| `env(VAR)` | Get env var (parsed as YAML) |
| `strenv(VAR)` | Get env var as string |
| `envsubst` | Substitute `${VAR}` in strings |
| `envsubst(flags)` | With validation flags |

## 6. Constructors

### 6.1 Array Construction

```yaml
[]                       # Empty array
[.a, .b, .c]             # Array from expressions
[.items[]]               # Collect iteration
```

### 6.2 Object Construction

```yaml
{}                       # Empty object
{"key": .value}          # Static key
{(.key): .value}         # Dynamic key
{name, age}              # Shorthand for {name: .name, age: .age}
```

## 7. Differences from jq

| Feature | jq | yq |
|---------|----|----|
| Reduce | `reduce .[] as $x (init; ...)` | `.[] as $x ireduce (init; ...)` |
| Conditionals | `if-then-else-end` | Not supported (use select) |
| Try-catch | `try-catch` | Not supported |
| Recursion | `recurse` | `..` (recursive descent) |
| Import | `import` | `load()` |
| Define functions | `def f: ...;` | Not supported |
| Optional object index | `.foo?` | Supported |
| Comments | `#` | Not in expressions |

## 8. Output Formats

| Flag | Format |
|------|--------|
| `-o=yaml` | YAML (default) |
| `-o=json` | JSON |
| `-o=props` | Java properties |
| `-o=csv` | CSV |
| `-o=tsv` | TSV |
| `-o=xml` | XML |

| Flag | Effect |
|------|--------|
| `-r` | Raw string output (no quotes) |
| `-I=N` | Set indentation (0 for compact) |
| `-P` | Pretty print |

## 9. Grammar Summary (EBNF-like)

```ebnf
program     = expression

expression  = pipe_expr

pipe_expr   = union_expr ("|" union_expr)*

union_expr  = alt_expr ("," alt_expr)*

alt_expr    = or_expr ("//" or_expr)*

or_expr     = and_expr ("or" and_expr)*

and_expr    = compare_expr ("and" compare_expr)*

compare_expr = add_expr (("==" | "!=" | "<" | "<=" | ">" | ">=") add_expr)?

add_expr    = mul_expr (("+" | "-") mul_expr)*

mul_expr    = unary_expr (("*" | "/" | "%") unary_expr)*

unary_expr  = "not"? postfix_expr

postfix_expr = primary_expr suffix*

suffix      = "." identifier
            | "." "[" expression "]"
            | "[" index_expr "]"
            | "?"
            | "(" arguments ")"

primary_expr = "."
             | ".."
             | "..."
             | identifier
             | "$" identifier
             | literal
             | "[" array_elements "]"
             | "{" object_elements "}"
             | "(" expression ")"

index_expr  = expression
            | expression ":" expression?
            | ":" expression

literal     = number | string | "true" | "false" | "null"
```

## 10. Implementation Notes

### 10.1 Evaluation Model

- Expressions produce zero or more results
- Multiple results are processed independently through pipes
- The `.` always refers to the current context value
- Variables are immutable bindings

### 10.2 Type Coercion

- Arithmetic operations require compatible types
- String comparison uses bytecode ordering
- Null propagates through most operations

### 10.3 Error Handling

- Missing fields return `null` (unless using non-optional access)
- Type mismatches raise errors
- Optional operators (`?`) suppress errors
