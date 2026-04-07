# Wiki Ingestion Log

Tracks updates to the knowledge wiki pages in `docs/`.

## 2026-04-07 — Initial wiki creation

**Sources ingested:**
- `docs/architecture/` — all 6 files (README, core-concepts, bitvec, balanced-parens, semi-indexing, prior-art)
- `docs/parsing/` — json.md, yaml.md (first 100 lines), dsv.md
- `docs/optimizations/simd.md` — first 100 lines
- `src/lib.rs`, `src/jq/mod.rs` — public API and module structure
- `CLAUDE.md` — optimization history, CLI reference, feature flags
- Repository exploration — full file tree of docs/ and src/

**Pages created:**
- [index.md](index.md) — Knowledge map entry point with concept graph, paper references, cross-links
- [bitvec.md](bitvec.md) — BitVec rank/select, Poppy structure, SIMD popcount
- [balanced-parens.md](balanced-parens.md) — BP tree encoding, RangeMin, generic SelectSupport
- [json-index.md](json-index.md) — JSON semi-indexing, PFSM, SIMD classification pipeline
- [yaml-index.md](yaml-index.md) — YAML oracle parser, virtual brackets, P0-P12/O1-O3 optimization history
- [dsv-index.md](dsv-index.md) — DSV quote handling, prefix XOR, BMI2 toggle
- [jq-evaluator.md](jq-evaluator.md) — jq parser/evaluator, JqSemantics vs YqSemantics, supported syntax
- [simd-strategy.md](simd-strategy.md) — Per-module SIMD usage, platform support, lessons learned

**Not yet covered (gaps to fill in future ingestion):**
- Detailed `src/bits/` source analysis (compact_rank.rs, elias_fano.rs)
- Full YAML parsing doc (only first 100 lines ingested; yaml.md is very long)
- `docs/optimizations/` — 10 remaining technique guides
- `docs/getting-started/` and `docs/guides/` — user-facing documentation
- `docs/plan/` — implementation planning documents
- Academic paper PDFs (referenced by URL but not ingested)
- Test suite structure (`tests/`)
