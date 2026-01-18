# Implementation Plans

[Home](/) > [Docs](../) > Plan

This directory contains planning documents for major features that have been **implemented**.

These plans are kept for:
- Understanding the design rationale
- Reference for future similar work
- Historical context on implementation decisions

## Active Plans

| Plan | Status | Module | Description |
|------|--------|--------|-------------|
| [jq.md](jq.md) | Implemented | `src/jq/` | jq query language for JSON |
| [dsv.md](dsv.md) | Implemented | `src/dsv/` | DSV (CSV/TSV) semi-indexing |
| [yq.md](yq.md) | Implemented | `src/yaml/`, `yq_runner.rs` | yq command for YAML |
| [simd-features.md](simd-features.md) | Current | `src/yaml/simd/` | YAML SIMD feature flag matrix |

## Plan Status Legend

- **Implemented**: Feature exists in codebase, plan reflects final design
- **Current**: Document describes current code behavior
- **Archived**: Plan superseded or approach changed

## Using Plans

When implementing similar features:
1. Review relevant plan for design patterns
2. Note any lessons learned sections
3. Check archive for rejected approaches

If the codebase diverges from a plan, the plan should be updated or archived.
