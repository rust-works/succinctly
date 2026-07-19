# Wiki Ingestion Log

Tracks updates to the knowledge wiki pages in `docs/`.

## 2026-07-15 — Rust succinct-library evaluation (issue #47)

**Sources ingested:**
- crates.io + GitHub metadata for `succinct`, `vers-vecs`, `fid`, `bio`, `sucds`, `sux` — versions,
  maintenance status, `no_std` support (verified by compiling, not by reading the attribute)
- `src/bits/` — `bitvec.rs`, `rank.rs`, `select.rs`, `popcount.rs`; the ~25% overhead note at `rank.rs:352-354`
- `src/trees/bp.rs`, `src/json/light.rs`, `src/dsv/index.rs` — coupling that a generic crate cannot supply
- New measurements from `bench-compare/benches/succinct_libs.rs` (Apple M5 Max)

**Pages created:**
- [adr-0011.md](adrs/adr-0011.md) — why succinct structures are built in-crate rather than taken from a crate
- [rust-succinct-libs.md](benchmarks/rust-succinct-libs.md) — rank/select vs vers-vecs, sucds, sux

**Pages corrected:**
- [prior-art.md](architecture/prior-art.md), `CLAUDE.md`, `README.md`,
  [hierarchical-structures.md](optimizations/hierarchical-structures.md) — all claimed the rank directory costs
  ~3% space. That is the Poppy *paper's* figure; succinctly's directory costs ~25% by design
  (`src/bits/rank.rs:352-354`), and a full `BitVec` measures 27.5–47.5% resident. Corrected throughout.

## 2026-07-15 — YAML Test Suite conformance (issue #49)

**Sources ingested:**
- [YAML Test Suite](https://github.com/yaml/yaml-test-suite) at tag `data-2022-01-17` — 402 cases, vendored
- `tests/yaml_test_suite.rs` — the previous 5040-line generated harness (replaced)
- `src/yaml/` — parser, error variants, scalar type resolution
- `src/json/validate.rs`, `src/bin/succinctly/json_validate.rs` — the JSON validation precedent being mirrored
- `src/bin/succinctly/yq_runner.rs` — the two YAML→JSON output paths
- Measured runs of `succinctly yq` against all 402 suite cases

**Pages created:**
- [compliance/yaml/limitations.md](compliance/yaml/limitations.md) — measured YAML Test Suite conformance (load 72.4%, reject 11.7%), unsupported features, the two-output-path divergence, and the rejection of the hybrid-parser proposal

**Pages updated:**
- [index.md](index.md) — new "Specification Compliance" section; the knowledge map previously did not link `compliance/` at all
- [README.md](README.md) — compliance section and "Finding What You Need" entry
- [compliance/yaml/1.2.md](compliance/yaml/1.2.md) — corrected two false claims: that `Null` resolves to null (it does not), and that tags are escaped in output (they are a parse error in block context, silently absorbed in flow context); added the missing breadcrumb
- [parsing/README.md](parsing/README.md) — YAML was described as "Feasibility analysis (not implemented)"; it ships
- [parsing/yaml.md](parsing/yaml.md) — marked §8 "Strict mode only" as superseded (never implemented; contradicted by the measured 11.7% rejection rate) and the phased plan's "Not Supported" lists as historical
- [parsing/yaml-index.md](parsing/yaml-index.md) — added a Validation section mirroring `parsing/json-index.md`

**Corrections to record:**
- The repo claimed to run the YAML Test Suite. It ran a hand-picked 253 of 402 cases with all 64 error cases `#[ignore]`d, 54 of the then-failing cases absent, at least one expectation transcribed wrongly (`4Q9F`), and comparisons made against a test-local converter rather than the shipped one.
- `succinctly yq` has two YAML→JSON implementations that disagree on the *value* of 29 suite cases; `-I 0` silently selects between them.

**Not yet covered (gaps to fill in future ingestion):**
- Scalar type resolution is duplicated across 5 sites in `src/`; the `NULL`/`Null`, hex/octal int, and bare `nan`/`inf` divergences from the 1.2 core schema are recorded but not fixed
- `docs/README.md` and `docs/index.md` still state different yq speedup figures for the same comparison

## 2026-07-15 — Environment variable reference (issue #48)

**Sources ingested:**
- `src/json/simd/mod.rs` — `SUCCINCTLY_SVE2` dispatch, cfg gating, NEON/SVE2 trade-off
- `src/bin/succinctly/jq_runner.rs` — `NO_COLOR`, `JQ_COLORS`, `JQ_LIBRARY_PATH`, `HOME`, `SUCCINCTLY_PRESERVE_INPUT`
- `src/bin/succinctly/yq_runner.rs` — color handling
- `src/jq/eval.rs` — `TZ` parsing, `$ENV`/`env`/`env()`/`strenv()` builtins
- `docs/STYLE_GUIDE.md` — STYLE-0003 (document env-switchable dispatch), STYLE-0004
- Behavior of `jq-1.7.1-apple`, probed directly to establish the compatibility target

**Pages created:**
- [environment-variables.md](reference/environment-variables.md) — all 8 environment-variable entries,
  their accepted values, precedence, and caveats

**Findings folded back into the code:**
- `NO_COLOR=""` wrongly disabled color; the convention and jq both require a non-empty value
- `succinctly yq` ignored `NO_COLOR` entirely
- `JQ_COLORS` was unvalidated, interpolating arbitrary text into an escape sequence
- `SUCCINCTLY_SVE2` was re-read from the environment on every index build

**Not yet covered:**
- `docs/index.md` Core Data Structures table is a concept map, so the reference page is
  deliberately not listed there
- Colored-output layout still differs from jq (reset placement, and jq 1.7's `0;90` default for
  `null` versus succinctly's `1;30`) — pre-existing, not part of issue #48

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
- [bitvec.md](architecture/bitvec.md) — BitVec rank/select, Poppy structure, SIMD popcount
- [balanced-parens.md](architecture/balanced-parens.md) — BP tree encoding, RangeMin, generic SelectSupport
- [json-index.md](parsing/json-index.md) — JSON semi-indexing, PFSM, SIMD classification pipeline
- [yaml-index.md](parsing/yaml-index.md) — YAML oracle parser, virtual brackets, P0-P12/O1-O4 optimization history
- [dsv-index.md](parsing/dsv-index.md) — DSV quote handling, prefix XOR, BMI2 toggle
- [jq-evaluator.md](reference/jq-evaluator.md) — jq parser/evaluator, JqSemantics vs YqSemantics, supported syntax
- [simd-strategy.md](optimizations/simd-strategy.md) — Per-module SIMD usage, platform support, lessons learned

**Not yet covered (gaps to fill in future ingestion):**
- Detailed `src/bits/` source analysis (compact_rank.rs, elias_fano.rs)
- Full YAML parsing doc (only first 100 lines ingested; yaml.md is very long)
- `docs/optimizations/` — 10 remaining technique guides
- `docs/getting-started/` and `docs/guides/` — user-facing documentation
- `docs/plan/` — implementation planning documents
- Academic paper PDFs (referenced by URL but not ingested)
- Test suite structure (`tests/`)
