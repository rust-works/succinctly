# Implementation Plans

[Home](../../) > [Docs](../) > Plan

Design documents written before or alongside a piece of work. A plan whose work is finished is
archived **in place**: it keeps its path, so the links into it from source comments, tests,
scripts, ADRs and error messages keep resolving, and it gains a status line at the top saying
what it became and where the current reference lives. A plan is only deleted when nothing links
to it and it records nothing that is not recorded elsewhere (#5847).

## Live Plans

These still track open work.

| Plan                                                   | Status      | Module                            | Description                                        |
|--------------------------------------------------------|-------------|-----------------------------------|----------------------------------------------------|
| [cspoppy.md](cspoppy.md)                               | Implemented | `src/trees/bp.rs`, `src/bits/`    | Combined sampling, BP select; F1 (#596) still open |
| [yq-memory-optimization.md](yq-memory-optimization.md) | Partial     | `yq_runner.rs`, `eval_generic.rs` | yq memory plan; M2/M3 done, M1/M4-M6 not landed    |
| [jq-remaining.md](jq-remaining.md)                     | Open        | `src/jq/`, CLI                    | jq CLI and module-system gaps                      |
| [yq-remaining.md](yq-remaining.md)                     | Open        | `src/yaml/`, `yq_runner.rs`       | yq operators, encoders and flags not yet supported |

## Archived Plans

The work is finished, superseded or rejected. Each is kept as the design record: the rationale,
the alternatives that were tried, and measurements that may not be recorded anywhere else.

| Plan                                                                           | Status      | Module                             | Description                                                    |
|--------------------------------------------------------------------------------|-------------|------------------------------------|----------------------------------------------------------------|
| [jq.md](jq.md)                                                                 | Implemented | `src/jq/`                          | Original jq query language plan                                |
| [yq.md](yq.md)                                                                 | Implemented | `src/yaml/`, `yq_runner.rs`        | Original yq command plan; #685/#757 lazy-output designs        |
| [dsv.md](dsv.md)                                                               | Implemented | `src/dsv/`                         | Original DSV (CSV/TSV) port plan; hw-dsv comparison            |
| [jq-lazy-map-select.md](jq-lazy-map-select.md)                                 | Implemented | `src/jq/eval_generic.rs`           | Lazy `map`/`select` chains (`LazySeq`)                         |
| [jq-path-trackability-deferral.md](jq-path-trackability-deferral.md)           | Implemented | `src/jq/eval.rs`                   | Deferred path trackability checks                              |
| [jq-lazy-generator-consumers.md](jq-lazy-generator-consumers.md)               | Implemented | `src/jq/eval.rs`                   | Demand-driven sink for short-circuiting consumers              |
| [jq-duplicate-key-collapse.md](jq-duplicate-key-collapse.md)                   | Implemented | `src/jq/`, `jq_runner.rs`          | jq-mode duplicate object key collapse                          |
| [jq-generator-argument-fanout.md](jq-generator-argument-fanout.md)             | Implemented | `src/jq/eval.rs`                   | One builtin result per generator-argument output               |
| [jq-range-lazy-bounds.md](jq-range-lazy-bounds.md)                             | Implemented | `src/jq/eval.rs`                   | Lazy `range()` bound-argument resolution                       |
| [jq-bind-origin-frame.md](jq-bind-origin-frame.md)                             | Implemented | `src/jq/eval.rs`, `src/jq/expr.rs` | Node identity for a navigated `$var` in path position          |
| [jq-path-mode-interpreter-spike.md](jq-path-mode-interpreter-spike.md)         | Implemented | `src/jq/path_mode.rs`              | Path-mode interpreter go/no-go report (#5223)                  |
| [decode-failure-routing.md](decode-failure-routing.md)                         | Implemented | `src/jq/`, `src/yaml/`             | Decode-failure error routing                                   |
| [issue-3025-spike.md](issue-3025-spike.md)                                     | Implemented | `src/jq/`                          | Lossless reindex bridge spike (#3025)                          |
| [issue-3025-results.md](issue-3025-results.md)                                 | Implemented | `src/jq/`                          | Lossless reindex bridge results (#3025)                        |
| [m2-benchmark-improvements.md](m2-benchmark-improvements.md)                   | Implemented | `yq_bench.rs`                      | Benchmark the M2 streaming path                                |
| [sve2-optimizations.md](sve2-optimizations.md)                                 | Implemented | `src/util/simd/`, `src/json/simd/` | SVE2 and NEON optimization survey                              |
| [jq-alt-retry-stash-audit.md](jq-alt-retry-stash-audit.md)                     | Snapshot    | `src/jq/`                          | `?//` retry stash-slot audit (#3517)                           |
| [path-context-arm-reachability.md](path-context-arm-reachability.md)           | Superseded  | `src/jq/eval.rs`                   | Arm audit of the eager path-context evaluator (deleted, #2559) |
| [jq-path-register-producer-contract.md](jq-path-register-producer-contract.md) | Superseded  | `src/jq/eval.rs`                   | Inferred path register; superseded by ADR-0026                 |
| [yaml-index-post-compactrank.md](yaml-index-post-compactrank.md)               | Superseded  | `src/yaml/index.rs`                | Index analysis from the CompactRank era                        |
| [compact-index-investigation.md](compact-index-investigation.md)               | Rejected    | `src/yaml/index.rs`                | Elias-Fano for position arrays (#58)                           |

## Plan Status Legend

- **Open**: Lists work that has not been done
- **Partial**: Some phases landed; the rest are not known to have landed
- **Implemented**: The work landed; the plan is the design record, not a description of the current code
- **Snapshot**: A dated audit; its verdicts describe the code at the commit it names
- **Superseded**: The code or approach it describes was replaced
- **Rejected**: The approach was measured and not adopted

## Using Plans

When implementing similar features:
1. Review relevant plan for design patterns
2. Note any lessons learned sections
3. Check Archived Plans for superseded analyses and rejected approaches

If the codebase diverges from a live plan, update the plan. When the work it tracks is finished,
move its row to Archived Plans and add a status line at the top of the file instead of moving or
deleting it.
