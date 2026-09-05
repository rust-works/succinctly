# Implementation Plans

[Home](../../) > [Docs](../) > Plan

This directory contains planning documents for major features that have been **implemented**.

These plans are kept for:
- Understanding the design rationale
- Reference for future similar work
- Historical context on implementation decisions

## Active Plans

| Plan                                                                 | Status      | Module                            | Description                                         |
|----------------------------------------------------------------------|-------------|-----------------------------------|-----------------------------------------------------|
| [jq.md](jq.md)                                                       | Implemented | `src/jq/`                         | jq query language for JSON                          |
| [dsv.md](dsv.md)                                                     | Implemented | `src/dsv/`                        | DSV (CSV/TSV) semi-indexing                         |
| [yq.md](yq.md)                                                       | Implemented | `src/yaml/`, `yq_runner.rs`       | yq command for YAML                                 |
| [yq-memory-optimization.md](yq-memory-optimization.md)               | Partial     | `yq_runner.rs`, `eval_generic.rs` | yq memory reduction plan                            |
| [compact-index-investigation.md](compact-index-investigation.md)     | Proposed    | `src/yaml/index.rs`               | Elias-Fano for position arrays                      |
| [m2-benchmark-improvements.md](m2-benchmark-improvements.md)         | Proposed    | `yq_bench.rs`                     | Benchmark M2 streaming path                         |
| [simd-features.md](simd-features.md)                                 | Current     | `src/yaml/simd/`                  | YAML SIMD feature flag matrix                       |
| [cspoppy.md](cspoppy.md)                                             | Implemented | `src/trees/bp.rs`, `src/bits/`    | Combined sampling, BP select                        |
| [jq-lazy-map-select.md](jq-lazy-map-select.md)                       | Partial     | `src/jq/eval_generic.rs`          | Lazy `map`/`select` chains (`LazySeq`)              |
| [jq-path-trackability-deferral.md](jq-path-trackability-deferral.md) | Implemented | `src/jq/eval.rs`                  | Deferred path trackability checks                   |
| [jq-lazy-generator-consumers.md](jq-lazy-generator-consumers.md)     | Partial     | `src/jq/eval.rs`                  | Demand-driven sink for short-circuiting consumers   |
| [jq-duplicate-key-collapse.md](jq-duplicate-key-collapse.md)         | Implemented | `src/jq/`, `jq_runner.rs`         | jq-mode duplicate object key collapse               |
| [jq-generator-argument-fanout.md](jq-generator-argument-fanout.md)   | Implemented | `src/jq/eval.rs`                  | One builtin result per generator-argument output    |
| [jq-range-lazy-bounds.md](jq-range-lazy-bounds.md)                   | Implemented | `src/jq/eval.rs`                  | Lazy `range()` bound-argument resolution            |
| [decode-failure-routing.md](decode-failure-routing.md)               | Proposed    | `src/jq/`, `src/yaml/`            | Decode-failure error routing                        |
| [path-context-arm-reachability.md](path-context-arm-reachability.md) | Current     | `src/jq/eval.rs`                  | Eager path-context evaluator arm reachability audit |

## Archived Plans

| Plan                                                             | Status   | Module              | Description                             |
|------------------------------------------------------------------|----------|---------------------|-----------------------------------------|
| [yaml-index-post-compactrank.md](yaml-index-post-compactrank.md) | Archived | `src/yaml/index.rs` | Index analysis from the CompactRank era |

## Plan Status Legend

- **Implemented**: Feature exists in codebase, plan reflects final design
- **Current**: Document describes current code behavior
- **Archived**: Plan superseded or approach changed

## Using Plans

When implementing similar features:
1. Review relevant plan for design patterns
2. Note any lessons learned sections
3. Check Archived Plans for superseded analyses and rejected approaches

If the codebase diverges from a plan, the plan should be updated or archived.
