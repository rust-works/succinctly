# YamlIndex

[Knowledge Map](index.md) > YamlIndex

Semi-index for YAML documents. Converts indentation-based structure into balanced parentheses via an "oracle parser", enabling O(1) navigation with lazy value extraction.

## What It Does

YAML's context-sensitive grammar (indentation, multiple scalar styles, anchors/aliases) makes it significantly harder to semi-index than JSON. Succinctly uses a two-phase approach:

1. **Oracle phase** — a sequential parser that tracks indentation and resolves character ambiguity, emitting virtual brackets
2. **Index phase** — the virtual brackets become a [BalancedParens](architecture/balanced-parens.md) encoding, just like JSON

After building, the same cursor API provides O(1) navigation.

## Oracle + Virtual Brackets

YAML has no explicit delimiters. The oracle inserts virtual brackets where indentation changes:

```yaml
users:
  - name: Alice
    age: 30
  - name: Bob
```

Oracle output:
```
{ users: [ { name: Alice, age: 30 }, { name: Bob } ] }
         ^                        ^
         indent → open            dedent → close
```

| Indentation Change | Virtual Bracket                  |
|--------------------|----------------------------------|
| Increase           | Open `(` — entering container    |
| Decrease           | Close `)` — returning to parent  |
| Same level         | Sibling — no bracket             |
| Sequence `-`       | Open item within sequence        |

## Index Components

| Component                 | Purpose                     | Notes                                         |
|---------------------------|-----------------------------|-----------------------------------------------|
| Interest Bits (IB)        | Structural positions        | Same as JSON                                  |
| Balanced Parentheses (BP) | Tree structure              | With `WithSelect` for `at_offset`             |
| Type Bits (TY)            | Distinguish container types | YAML-specific: maps vs sequences vs scalars   |
| Advance Positions         | BP-to-text mapping          | Memory-efficient bitmap (P12)                 |
| End Positions             | Node end boundaries         | For value extraction                          |

## Streaming (P9)

The identity query (`yq '.'`) uses direct YAML-to-JSON streaming without an intermediate DOM:

```
YAML cursor --> JSON output (single pass)
```

This eliminated the `OwnedValue` intermediate representation, yielding a **2.3x speedup** — the largest optimization in the YAML pipeline.

## Optimization Journey

YAML parsing has an extensive documented optimization history (P0-P12, O1-O3):

| Phase | Result          | Technique                                    |
|-------|-----------------|----------------------------------------------|
| P2.5  | +1-17%          | Cached type checking                         |
| P2.7  | +19-25%         | Block scalar SIMD (AVX2 newline scanning)    |
| P4    | +6-17%          | Anchor/alias SIMD                            |
| P9    | +130%           | Direct YAML→JSON streaming                   |
| P10   | correctness     | Type preservation for yq compatibility       |
| P11   | 2.5-5.9x select | BP select1 for yq-locate                     |
| P12   | +20-25%         | Advance index (memory-efficient bp_to_text)  |
| O1    | +3-13%          | Sequential cursor for AdvancePositions       |
| O3    | 4-12x micro     | SIMD escape scanning (NEON)                  |

**Rejected** (with documented reasons): P2.6 (prefetching), P2.8 (threshold tuning), P3 (branchless), P5-P8 (various), all documented in [parsing/yaml.md](parsing/yaml.md).

Key lesson: micro-benchmark wins frequently don't translate to end-to-end gains. Three consecutive optimizations (P2.6, P2.8, P3) showed micro gains but caused real regressions.

## Additional Features

- **Anchors & aliases**: The oracle tracks `&name` anchors and `*name` aliases, storing an anchor-to-position mapping. Aliases are resolved at query time, not during indexing (no automatic expansion).
- **Block scalars**: Literal (`|`) and folded (`>`) block scalars with chomping modifiers (`-`, `+`, default).
- **Multi-document streams**: Multiple YAML documents (`---` separated) are wrapped in an implicit array. Use `--doc N` to select a specific document.

## Depends On

- [BitVec](architecture/bitvec.md) — all bit vectors use rank/select
- [BalancedParens](architecture/balanced-parens.md) — with `WithSelect` generic parameter

## Used By

- [jq Evaluator](jq-evaluator.md) — via `YqSemantics` evaluation mode

## Source & Docs

- Implementation: [src/yaml/](../src/yaml/) (parser.rs, index.rs, advance_positions.rs, end_positions.rs)
- SIMD: [src/yaml/simd/](../src/yaml/simd/) (neon.rs, x86.rs, broadword.rs)
- Parsing doc: [parsing/yaml.md](parsing/yaml.md) (very detailed, includes all optimization history)
- YAML 1.2 compliance: [compliance/yaml/1.2.md](compliance/yaml/1.2.md)
- Benchmark: [benchmarks/yq.md](benchmarks/yq.md)
