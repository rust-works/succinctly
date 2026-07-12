# Architecture

This is the top-level entry point for understanding how succinctly is built. It gives a
one-page mental model and then hands off to the deeper documentation under [`docs/`](docs/).

## What succinctly is

Succinctly is a high-performance Rust library of **succinct data structures** — bit vectors
and trees that store data close to its information-theoretic minimum while still supporting
fast queries. On top of that foundation it provides **semi-indexing** parsers for JSON, YAML,
and DSV/CSV, and a `jq`-style query engine over them.

## The core idea: semi-indexing

Rather than building a full in-memory DOM, succinctly builds a lightweight structural index
(~3–6% overhead) and materialises values lazily. The pipeline is:

1. **Scan** — a SIMD-accelerated pass finds structural characters (braces, brackets, quotes,
   newlines).
2. **Encode** — those become a [balanced-parentheses](docs/architecture/balanced-parens.md)
   bit vector plus "interest bit" vectors.
3. **Navigate** — O(1) tree operations (parent/child/sibling, subtree skip) run as rank/select
   over the bit vector.
4. **Extract** — a value is parsed only when a query actually reads it.

This is what buys the large memory and speed wins over DOM parsers (jq/yq/serde_json). See
[docs/architecture/semi-indexing.md](docs/architecture/semi-indexing.md) for the full design.

## How the pieces layer

```
BitVec (rank/select)
   └─ BalancedParens (succinct trees)
        ├─ JsonIndex ┐
        ├─ YamlIndex ┼─ jq / yq evaluator (generic over a Document + cursor)
        └─ DsvIndex  ┘
```

Each format module carries a `simd/` subdirectory selected at runtime: x86_64 uses
AVX2 / BMI2 / SSE4.2, ARM64 uses NEON / SVE2, and a broadword/scalar path is the fallback
when no accelerated instruction set is detected.

## Module map

```
src/
├── lib.rs      # public API, RankSelect trait, no_std crate attrs
├── bits/       # BitVec: rank/select, popcount
├── trees/      # BalancedParens: succinct tree navigation
├── json/       # JSON semi-indexing (+ json/simd/)
├── yaml/       # YAML semi-indexing (+ yaml/simd/)
├── dsv/        # DSV/CSV semi-indexing (+ dsv/simd/)
├── jq/         # jq query language and evaluator
├── util/       # shared SIMD helpers, broadword (+ util/simd/)
└── bin/        # CLI tool (jq, yq, jq-locate, yq-locate, bench runner)
```

## Start here

For anything beyond this overview, follow the canonical docs — this page deliberately does not
duplicate their detail:

| Go to | For |
|-------|-----|
| [docs/index.md](docs/index.md) | The concept-oriented **knowledge map** — how every structure, algorithm, and paper relates |
| [docs/architecture/](docs/architecture/) | Deep design docs: BitVec, balanced parens, semi-indexing, prior art |
| [docs/parsing/](docs/parsing/) | Per-format parser internals (JSON PFSM, YAML oracle, DSV) |
| [docs/optimizations/](docs/optimizations/) | Optimization techniques and the accept/reject history (incl. why AVX-512 was dropped) |
| [docs/benchmarks/](docs/benchmarks/) | Per-platform benchmark results |
| [docs/STYLE_GUIDE.md](docs/STYLE_GUIDE.md) | Tagged, stable-ID coding & docs conventions |
| [CLAUDE.md](CLAUDE.md) | Full module map, commands, feature flags, performance summary |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to build, test, and submit changes |
