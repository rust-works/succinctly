# jq Evaluator

[Home](../../) > [Docs](../) > [Reference](./) > jq Evaluator

A jq-compatible query language implementation that works with succinctly's semi-indexed documents, supporting both JSON and YAML inputs.

## What It Does

The jq module provides `parse()` and `eval()` functions that execute jq expressions against semi-indexed documents without building a DOM:

```rust
use succinctly::jq::{parse, eval, JqSemantics, QueryResult};
use succinctly::json::{JsonIndex, StandardJson};

let json = br#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#;
let index = JsonIndex::build(json);
let cursor = index.root(json);

let expr = parse(".users[].name").unwrap();
let result = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
```

## Architecture

```mermaid
graph TD
    Q[Query string] --> P[Parser\nsrc/jq/parser.rs]
    P --> AST[Expr AST\nsrc/jq/expr.rs]
    AST --> E[Evaluator\nsrc/jq/eval.rs, eval_generic.rs]
    E --> JQ[JqSemantics\nJSON input]
    E --> YQ[YqSemantics\nYAML input, type preservation]
    JQ --> D[Document trait\ncursor-based navigation]
    YQ --> D
    D --> R[QueryResult::One / QueryResult::Many]
```

### Generic over Document

The evaluator is generic over a `Document` trait, which is implemented by both `JsonIndex` and `YamlIndex` cursors. This means the same jq expression AST works for both JSON and YAML inputs.

### Evaluation Semantics

| Mode           | Used By      | Behavior                                            |
|----------------|--------------|-----------------------------------------------------|
| `JqSemantics`  | `sjq` (JSON) | Standard jq behavior                                |
| `YqSemantics`  | `syq` (YAML) | Preserves YAML types (quoted strings stay strings)  |

### Owned values and long number literals

A document number keeps its source token in `OwnedValue::NumberLiteral`. The internal
reindex bridge now writes that token verbatim at every length, so a filter sees the
same decimal value whether it reads the original cursor or a materialized value.
Computed floats retain their separate internal token, and NaN retains its sentinel.

The evaluator runs a bounded set of operations directly on owned state: entries
conversion, literal numeric updates to an existing object field, and their
supported pipes. Other expressions still use the lossless reindex bridge. Loop
results can be projected through the sink before array collection, so
`[while(.i < N; .i += 1) | .i]` stores only projected integers. An array
requesting all complete states still stores those states.

See [the #3025 spike](../plan/issue-3025-spike.md) for the 200,000-digit
cost and the [implementation results](../plan/issue-3025-results.md).

### Streaming

For large outputs, the evaluator supports streaming via `StreamableValue`, writing results incrementally without buffering the entire output. The YAML identity query (`yq '.'`) uses direct cursor-to-JSON streaming (P9 optimization).

## Supported Syntax

The implementation covers a substantial subset of jq:

- **Navigation**: `.field`, `.[n]`, `.[]`, `.[n:m]`, `..` (recursive descent)
- **Construction**: `[expr]` (arrays), `{key: expr}` (objects)
- **Operators**: arithmetic (`+`, `-`, `*`, `/`, `%`), comparison, boolean (`and`, `or`, `not`), alternative (`//`)
- **Assignment**: `=`, `|=`, `+=`, `-=`, `*=`, `/=`, `%=`, `//=`, `del()`
- **Control flow**: `if-then-else-end`, `try-catch`, pipes (`|`), comma (multiple outputs)
- **Builtins**: `length`, `keys`, `values`, `has()`, `in()`, `map()`, `select()`, `empty`, `range()`, `limit()`, `first()`, `last()`, `type`, `tostring`, `tonumber`, `ascii_downcase`, `ascii_upcase`, `split()`, `join()`, `test()`, `match()`, `capture()`, `gsub()`, `sub()`, `ltrimstr()`, `rtrimstr()`, `startswith()`, `endswith()`, `contains()`, `inside()`, `indices()`, `sort_by()`, `group_by()`, `unique_by()`, `min_by()`, `max_by()`, `flatten`, `transpose`, `to_entries`, `from_entries`, `with_entries`, `paths`, `getpath`, `setpath`, `delpaths`, `leaf_paths`, `env`, `input`, `inputs`, `debug`, `stderr`, `halt`, `halt_error`, `def-;` (function definitions), `label-break`, `foreach`, `reduce`, `$__loc__`, `@format` strings
- **Format functions**: `@csv`, `@tsv`, `@dsv(delim)`, `@json`, `@text`, `@uri`, `@base64`, `@base64d`, `@html`, `@sh`, `@yaml`, `@props`
- **Position navigation** (succinctly extension): `at_offset(n)`, `at_position(line; col)`

## Depends On

- [JsonIndex](../parsing/json-index.md) — cursor API for JSON documents
- [YamlIndex](../parsing/yaml-index.md) — cursor API for YAML documents
- [DsvIndex](../parsing/dsv-index.md) — via `--input-dsv` flag

## Source & Docs

- Implementation: [src/jq/](../../src/jq/) (parser.rs, expr.rs, eval.rs, eval_generic.rs, value.rs, stream.rs, lazy.rs, document.rs)
- CLI integration: [src/bin/succinctly/jq_runner.rs](../../src/bin/succinctly/jq_runner.rs), [src/bin/succinctly/yq_runner.rs](../../src/bin/succinctly/yq_runner.rs)
- CLI guide: [guides/cli.md](../guides/cli.md)
