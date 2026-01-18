# Documentation Refactoring Plan (Updated)

**Goal**: Reorganize documentation for improved discoverability, reduce redundancy, and create clear paths for different user types (first-time users, advanced users, contributors, developers).

**Scope**: Committed files outside `.omni-dev/` and `.claude/` directories.

**Exclusions**: CLAUDE.md stays in root (not refactored per user request).

---

## User Decisions Applied

1. ✅ **Planning docs**: Keep in `docs/plan/` if codebase reflects the plan. Delete if obsolete.
   - **Status**: All plans are implemented (dsv/, yaml/, jq/ exist, yq_runner.rs exists)
   - **Action**: Keep all planning docs, move PLAN-JQ.md to docs/plan/

2. ✅ **CLAUDE.md**: Exclude from refactoring - stays in root

3. ✅ **CLI.md**: Include in refactoring - move to docs/guides/

4. ❓ **yaml-build-matrix.md**: Need to determine if it reflects current code
   - Document describes feature flags: `broadword-yaml`, `scalar-yaml`
   - Need to verify these flags still exist in Cargo.toml

5. ✅ **US spelling**: Standardize throughout
   - `optimisations/` → `optimizations/`
   - `favour` → `favor`, `colour` → `color`, etc.

---

## Design Principles

1. **Audience-First Organization**: Separate docs by user journey
2. **Progressive Disclosure**: Simple → Advanced → Expert
3. **Single Source of Truth**: Each topic documented once, linked elsewhere
4. **Clear Entry Points**: README routes to audience-specific starting points
5. **Logical Grouping**: Related docs in same directory
6. **Minimize Root Clutter**: Keep root minimal (CLAUDE.md exception), move details to docs/

---

## Proposed Structure

```
succinctly/
├── README.md                          # Quick start, feature overview, links to guides
├── CHANGELOG.md                       # Version history (unchanged)
├── CLAUDE.md                          # AI guide (unchanged - excluded from refactor)
├── CODE_OF_CONDUCT.md                 # Community standards (unchanged)
├── CONTRIBUTING.md                    # Quick start for contributors → links to docs/
├── LICENSE                            # License file (unchanged)
│
├── .github/
│   └── pull_request_template.md      # PR checklist (update to link CONTRIBUTING)
│
├── docs/
│   ├── README.md                      # 🆕 Documentation index/map
│   │
│   ├── getting-started/               # 🆕 First-time user journey
│   │   ├── README.md                  # Navigation for beginners
│   │   ├── installation.md            # 🆕 Extract from user-guide
│   │   ├── quickstart.md              # 🆕 5-minute tutorial
│   │   └── examples.md                # 🆕 Common use cases
│   │
│   ├── guides/                        # Reorganized user/dev guides
│   │   ├── README.md                  # Guide index
│   │   ├── user.md                    # API usage (streamlined)
│   │   ├── cli.md                     # 📝 Move CLI.md here
│   │   ├── developer.md               # Contributing to codebase (unchanged)
│   │   └── release.md                 # 📝 Move RELEASE.md here
│   │
│   ├── architecture/                  # 🆕 Design & implementation
│   │   ├── README.md                  # Architecture overview
│   │   ├── core-concepts.md           # 🆕 Succinct data structures theory
│   │   ├── bitvec.md                  # 🆕 BitVec design
│   │   ├── balanced-parens.md         # 🆕 Tree encoding
│   │   └── semi-indexing.md           # 🆕 JSON/YAML/DSV approach
│   │
│   ├── parsing/                       # Parser implementation (updated)
│   │   ├── README.md                  # Update to reference consolidated DSV
│   │   ├── json.md                    # JSON parser (unchanged)
│   │   ├── yaml.md                    # YAML parser (unchanged)
│   │   └── dsv.md                     # 📝 Consolidate dsv-performance + dsv-profiling
│   │
│   ├── optimizations/                 # 📝 Rename optimisations → optimizations
│   │   ├── README.md                  # Technique index + decision framework
│   │   ├── quick-reference.md         # 🆕 One-page technique lookup
│   │   ├── bit-manipulation.md        # Updated US spelling
│   │   ├── simd.md                    # Updated US spelling
│   │   ├── lookup-tables.md
│   │   ├── state-machines.md
│   │   ├── cache-memory.md
│   │   ├── hierarchical-structures.md
│   │   ├── branchless.md
│   │   ├── access-patterns.md
│   │   ├── zero-copy.md
│   │   └── parallel-prefix.md
│   │
│   ├── benchmarks/                    # 🆕 Consolidate all benchmark docs
│   │   ├── README.md                  # Benchmark overview + methodology
│   │   ├── jq.md                      # 📝 Move jq-comparison.md
│   │   ├── yq.md                      # 📝 Move yq-comparison.md
│   │   ├── rust-parsers.md            # 📝 Move rust-json-comparison.md
│   │   ├── cross-parser.md            # 📝 Move CROSS-PARSER-BENCHMARKS.md
│   │   └── dsv.md                     # 🆕 Extract from dsv-performance.md
│   │
│   ├── plan/                          # Planning docs (kept - all implemented)
│   │   ├── README.md                  # 🆕 Explain these are implementation plans
│   │   ├── jq-implementation.md       # 📝 Move PLAN-JQ.md here
│   │   ├── dsv-port.md                # Keep (reflects current src/dsv/)
│   │   ├── yq-implementation.md       # Keep (reflects current yq_runner.rs)
│   │   └── yaml-build-matrix.md       # 📝 Move here OR update/delete
│   │
│   └── archive/                       # Historical docs (expanded)
│       ├── README.md                  # 🆕 Explain archive purpose
│       ├── optimizations/             # Historical optimization attempts
│       │   ├── SUMMARY.md             # 📝 Rename OPTIMIZATION-SUMMARY.md
│       │   ├── failed.md              # 📝 Rename failed-optimizations.md
│       │   ├── implemented.md         # 📝 Rename implemented-optimizations.md
│       │   ├── avx512-json.md
│       │   ├── avx512-vpopcntdq.md
│       │   └── ... (other archive files)
│       └── haskell-works/             # Reference implementations (unchanged)
│           └── ...
│
├── bench-compare/
│   └── README.md                      # Rust parser comparison (unchanged)
│
└── data/
    └── bench/
        └── results/                   # 🆕 Add to .gitignore
            ├── .gitignore             # 🆕 Ignore *.md files
            ├── jq-bench.md            # Generated, not committed
            └── dsv-bench.md           # Generated, not committed
```

---

## Refactoring Tasks

### Phase 1: Verify Current State

**Step 1.1: Check yaml-build-matrix.md is current**
```bash
# Check if feature flags mentioned in yaml-build-matrix.md exist
grep -E "(broadword-yaml|scalar-yaml)" Cargo.toml
grep -E "(broadword-yaml|scalar-yaml)" src/yaml/simd/mod.rs
```

**Decision**:
- ✅ If flags exist and work as documented → Move to `docs/plan/yaml-build-matrix.md`
- ❌ If flags are outdated/removed → Delete the file

**Step 1.2: Verify planning docs reflect implementation**
- ✅ PLAN-JQ.md → jq module exists, jq_runner.rs exists
- ✅ dsv-port-plan.md → src/dsv/ exists with all modules
- ✅ yq-implementation-plan.md → src/yaml/ exists, yq_runner.rs exists

**Action**: All plans are implemented. Keep in docs/plan/

### Phase 2: Create New Structure (Non-Breaking)

**Step 2.1: Create new directories**
```bash
mkdir -p docs/getting-started
mkdir -p docs/guides
mkdir -p docs/architecture
mkdir -p docs/benchmarks
mkdir -p docs/archive/optimizations
```

**Step 2.2: Create README/index files**
- `docs/README.md` - Documentation map with audience-specific paths
- `docs/getting-started/README.md` - Beginner navigation
- `docs/guides/README.md` - Guide index
- `docs/architecture/README.md` - Architecture overview
- `docs/benchmarks/README.md` - Benchmark methodology + overview
- `docs/plan/README.md` - Explain these are implementation plans
- `docs/archive/README.md` - Explain archive purpose + notable docs

**Step 2.3: Create new content (extracted/consolidated)**
- `docs/getting-started/installation.md` - Extract from user-guide.md
- `docs/getting-started/quickstart.md` - 5-minute tutorial (new)
- `docs/getting-started/examples.md` - Common patterns (new)
- `docs/optimizations/quick-reference.md` - One-page technique lookup
- `docs/architecture/core-concepts.md` - Succinct data structures primer
- `docs/architecture/bitvec.md` - BitVec design doc
- `docs/architecture/balanced-parens.md` - Tree encoding
- `docs/architecture/semi-indexing.md` - JSON/YAML/DSV approach overview
- `docs/benchmarks/dsv.md` - Extract benchmark numbers from dsv-performance.md

### Phase 3: Filename Rationalization

**Principle**: Remove redundant words from filenames when directory name provides context.

**Step 3.1: Review and simplify filenames**

Current redundancies to fix:
```
docs/plan/
├── jq-implementation-plan.md  → jq.md (directory is "plan")
├── dsv-port-plan.md           → dsv.md ("plan" redundant, "port" redundant)
├── yq-implementation-plan.md  → yq.md
└── yaml-build-matrix.md       → build-matrix.md or simd-features.md

docs/benchmarks/
├── jq-comparison.md           → jq.md ("comparison" implied by directory)
├── yq-comparison.md           → yq.md
├── rust-json-comparison.md    → rust-parsers.md (clearer name)
├── cross-parser.md            → cross-language.md (clearer intent)

docs/parsing/
├── json.md                    → ✓ (already clean)
├── yaml.md                    → ✓ (already clean)
├── dsv.md                     → ✓ (already clean)

docs/guides/
├── cli.md                     # Simplified from cli-guide.md ("guide" redundant)
├── user.md                    # Simplified from user-guide.md (directory is "guides")
├── developer.md               # Simplified from developer-guide.md
├── release.md                 # Simplified from release-guide.md
```

**Filename simplification rules**:
1. Remove directory name from filename (docs/plan/jq-plan.md → docs/plan/jq.md)
2. Remove obvious category words when directory provides context:
   - `*-guide.md` in `guides/` → `*.md`
   - `*-comparison.md` in `benchmarks/` → `*.md`
   - `*-plan.md` in `plan/` → `*.md`
3. Keep descriptive words that aren't redundant:
   - `cross-parser.md` → `cross-language.md` (more specific)
   - `rust-json-comparison.md` → `rust-parsers.md` (clearer)

**Final structure after rationalization**:
```
docs/
├── plan/
│   ├── jq.md              # was: jq-implementation-plan.md
│   ├── dsv.md             # was: dsv-port-plan.md
│   ├── yq.md              # was: yq-implementation-plan.md
│   └── simd-features.md   # was: yaml-build-matrix.md (more descriptive)
│
├── benchmarks/
│   ├── jq.md              # was: jq-comparison.md
│   ├── yq.md              # was: yq-comparison.md
│   ├── rust-parsers.md    # was: rust-json-comparison.md
│   ├── cross-language.md  # was: CROSS-PARSER-BENCHMARKS.md (clearer)
│   └── dsv.md             # new
│
└── guides/
    ├── cli.md             # Simplified from CLI.md (lowercase)
    ├── user.md            # Simplified from user-guide.md
    ├── developer.md       # Simplified from developer-guide.md
    └── release.md         # Simplified from release-guide.md
```

**Benefits**:
- Shorter, clearer paths: `docs/plan/jq.md` vs `docs/plan/jq-implementation-plan.md`
- Less redundancy: directory + filename don't repeat context
- Easier to remember: `docs/guides/cli.md` vs hypothetical `docs/guides/cli-guide.md`
- Consistent: all guides/ files are simple names

### Phase 4: Move & Rename Files

**Step 4.1: Root → docs/guides/**
```bash
git mv CLI.md docs/guides/cli.md
git mv RELEASE.md docs/guides/release.md
```

**Step 3.2: Root → docs/plan/**
```bash
git mv PLAN-JQ.md docs/plan/jq-implementation.md
```

**Step 3.3: Benchmark consolidation → docs/benchmarks/**
```bash
git mv docs/jq-comparison.md docs/benchmarks/jq.md
git mv docs/yq-comparison.md docs/benchmarks/yq.md
git mv docs/rust-json-comparison.md docs/benchmarks/rust-parsers.md
git mv docs/CROSS-PARSER-BENCHMARKS.md docs/benchmarks/cross-parser.md
```

**Step 3.4: Archive reorganization**
```bash
git mv docs/archive/OPTIMIZATION-SUMMARY.md docs/archive/optimizations/SUMMARY.md
git mv docs/archive/failed-optimizations.md docs/archive/optimizations/failed.md
git mv docs/archive/implemented-optimizations.md docs/archive/optimizations/implemented.md
git mv docs/archive/avx512-json-results.md docs/archive/optimizations/avx512-json-results.md
git mv docs/archive/avx512-vpopcntdq-results.md docs/archive/optimizations/avx512-vpopcntdq-results.md
git mv docs/archive/between-avx2-and-avx512.md docs/archive/optimizations/between-avx2-and-avx512.md
git mv docs/archive/optimization-opportunities.md docs/archive/optimizations/opportunities.md
git mv docs/archive/performance-analysis.md docs/archive/optimizations/performance-analysis.md
git mv docs/archive/performance-outcomes-summary.md docs/archive/optimizations/performance-outcomes-summary.md
git mv docs/archive/recommended-optimizations.md docs/archive/optimizations/recommended.md
```

**Step 3.5: Rename optimisations → optimizations (US spelling)**
```bash
git mv docs/optimisations docs/optimizations
```

**Step 4.6: yaml-build-matrix.md (after verification)**
```bash
# If still relevant (rename to be more descriptive):
git mv docs/yaml-build-matrix.md docs/plan/simd-features.md

# If obsolete:
git rm docs/yaml-build-matrix.md
```

**Step 4.7: Rename user-guide.md for clarity**
```bash
# "user-guide" is ambiguous (CLI users? API users?)
# "api" is clearer for library users
git mv docs/user-guide.md docs/guides/api.md

# Alternative: keep as "user.md" (simpler)
# git mv docs/user-guide.md docs/guides/user.md
```

### Phase 5: Consolidate & Deduplicate

**Step 4.1: Merge DSV documentation**

Create comprehensive `docs/parsing/dsv.md`:
- Section 1: Parser architecture (from current dsv.md)
- Section 2: Implementation details (from current dsv.md)
- Section 3: Performance characteristics (from dsv-performance.md)
- Section 4: Profiling analysis (from dsv-profiling-analysis.md)
- Link to `docs/benchmarks/dsv.md` for benchmark numbers

Create `docs/benchmarks/dsv.md`:
- Extract just the benchmark tables from dsv-performance.md
- Consolidate with any benchmark data from dsv-profiling-analysis.md

Delete redundant files:
```bash
git rm docs/dsv-performance.md
git rm docs/dsv-profiling-analysis.md
```

**Step 4.2: Streamline CLI documentation**
- Remove CLI section from `docs/guides/user.md` (lines 383-461)
- Ensure `docs/guides/cli.md` is comprehensive
- Add cross-reference at old location: `See [CLI Guide](cli.md)`

**Step 4.3: US spelling pass**

Find and replace throughout all docs:
```bash
# In docs/ (excluding .claude/, .omni-dev/)
grep -r "optimisation" docs/ | grep -v ".git" | grep -v "archive"
# Replace: optimisation → optimization, optimisations → optimizations
# Replace: colour → color, favour → favor, behaviour → behavior
# Replace: centre → center, metre → meter, analyse → analyze
```

Update file contents to use US spelling consistently.

### Phase 5: Update Cross-References

**Step 5.1: Update README.md**

Add "Documentation" section before "Contributing". Example structure (paths are relative from repo root):
```markdown
## Documentation

Choose your path:

- 🚀 **New to succinctly?** → Getting Started (`docs/getting-started/`)
- 📖 **Using the library?** → User Guide (`docs/guides/user.md`)
- 💻 **Using the CLI?** → CLI Guide (`docs/guides/cli.md`)
- 🤝 **Contributing?** → `CONTRIBUTING.md`
- ⚡ **Performance tuning?** → Optimization Techniques (`docs/optimizations/`)
- 🏗️ **Understanding internals?** → Architecture (`docs/architecture/`)
- 📊 **Benchmarks?** → Performance Comparisons (`docs/benchmarks/`)
- 🗺️ **Full documentation map** → `docs/`

For AI-assisted development, see `CLAUDE.md`.
```

**Step 5.2: Update CONTRIBUTING.md**

Add links at the top (after title):
```markdown
# Contributing to Succinctly

Thank you for your interest! This guide provides the essentials. For deeper technical details:

- [Developer Guide](docs/guides/developer-guide.md) - Codebase architecture and development workflow
- [Release Guide](docs/guides/release-guide.md) - Release process (for maintainers)
- [Architecture Docs](docs/architecture/) - Design decisions and core concepts
```

**Step 5.3: Update all internal links**

Search and replace across all .md files:
```bash
# CLI.md references
](CLI.md) → ](docs/guides/cli.md)
](../CLI.md) → ](../guides/cli.md)
](../../CLI.md) → ](../../guides/cli.md)

# RELEASE.md references
](RELEASE.md) → ](docs/guides/release.md)
](../RELEASE.md) → ](../guides/release.md)

# PLAN-JQ.md references
](PLAN-JQ.md) → ](docs/plan/jq-implementation.md)
](../PLAN-JQ.md) → ](../plan/jq-implementation.md)

# Optimisations → Optimizations
](docs/optimisations/) → ](docs/optimizations/)
](optimisations/) → ](optimizations/)
](../optimisations/) → ](../optimizations/)

# Benchmark references
](docs/jq-comparison.md) → ](docs/benchmarks/jq.md)
](docs/yq-comparison.md) → ](docs/benchmarks/yq.md)
](docs/rust-json-comparison.md) → ](docs/benchmarks/rust-parsers.md)
](docs/CROSS-PARSER-BENCHMARKS.md) → ](docs/benchmarks/cross-parser.md)

# DSV references
](docs/dsv-performance.md) → ](docs/parsing/dsv.md)
](docs/dsv-profiling-analysis.md) → ](docs/parsing/dsv.md)
```

**Step 5.4: Update .github/pull_request_template.md**
```markdown
<!-- Add at top after title -->
See [CONTRIBUTING.md](../CONTRIBUTING.md) and [Developer Guide](../docs/guides/developer-guide.md) for contribution guidelines.
```

### Phase 6: Improve Navigation

**Step 6.1: Create docs/README.md (Documentation Map)**

See detailed content in appendix below.

**Step 6.2: Create docs/getting-started/README.md**

Beginner-friendly navigation:
- Prerequisites check (Rust version, cargo)
- Quick install (`cargo add succinctly`)
- "Hello World" example
- Next steps (user guide, CLI guide, examples)

**Step 6.3: Create docs/optimizations/quick-reference.md**

Single-page reference table (example format using relative paths within `docs/optimizations/`):
| Technique | When to Use | Speedup | Document |
|-----------|-------------|---------|----------|
| Cumulative Index | Random access to sorted data | 627x | `hierarchical-structures.md#cumulative-index` |
| RangeMin Index | Tree navigation | 40x | `hierarchical-structures.md#rangemin` |
| ... | ... | ... | ... |

**Path convention**: Within `docs/optimizations/`, use relative links to sibling files (e.g., `[hierarchical-structures.md](hierarchical-structures.md)`).

**Step 6.4: Create docs/plan/README.md**

```markdown
# Implementation Plans

This directory contains planning documents for major features that have been **implemented**.

These plans are kept for:
- Understanding the design rationale
- Reference for future similar work
- Historical context on implementation decisions

## Active Plans

| Plan | Status | Module | Description |
|------|--------|--------|-------------|
| [jq-implementation.md](jq-implementation.md) | ✅ Implemented | `src/jq/` | jq query language for JSON |
| [dsv-port.md](dsv-port.md) | ✅ Implemented | `src/dsv/` | DSV (CSV/TSV) semi-indexing |
| [yq-implementation.md](yq-implementation.md) | ✅ Implemented | `src/yaml/`, `yq_runner.rs` | yq command for YAML |
| [yaml-build-matrix.md](yaml-build-matrix.md) | ✅ Current | `src/yaml/simd/` | YAML SIMD feature flag matrix |

If the codebase diverges from a plan, the plan should be updated or archived.
```

**Step 6.5: Create docs/archive/README.md**

```markdown
# Documentation Archive

This directory preserves historical documentation that provides context for past decisions.

## What's Here

### Optimization History
The `optimizations/` subdirectory contains the complete history of optimization attempts:
- **SUMMARY.md** - Comprehensive optimization timeline
- **failed.md** - Failed optimization attempts with analysis
- **implemented.md** - Successfully implemented optimizations
- **Performance analysis** - Historical benchmark results

### Haskell Reference Implementations
The `haskell-works/` subdirectory contains notes on the Haskell libraries that inspired this project:
- hw-json, hw-json-simd
- hw-dsv
- hw-rankselect, hw-balancedparens

### Other Historical Docs
- Migration notes
- Feature planning (completed features)
- Implementation explorations

## Why Keep Archives?

Archives prevent repeated mistakes by documenting:
- Why certain approaches don't work
- Performance characteristics of rejected implementations
- Evolution of the codebase architecture

Notable archived insights:
- AVX-512 is slower than AVX2 for memory-bound workloads
- Simpler data structures often outperform complex ones (DSV lightweight index)
- Micro-benchmarks can be misleading (YAML P2.8, P3, P5-P8)
```

### Phase 7: Quality Improvements

**Step 7.1: Standardize headers**

All docs should have:
```markdown
# Title

Brief description (1-2 sentences).

## Table of Contents (for docs >200 lines)

- [Section 1](#section-1)
- [Section 2](#section-2)

## Content starts here...
```

**Step 7.2: Add breadcrumbs**

Top of each doc (except root READMEs):
```markdown
[Home](/) > [Docs](/docs) > [Section](/docs/section) > Current Page
```

**Step 7.3: Add "See Also" sections**

Bottom of related docs, before any appendices:
```markdown
## See Also

- [Related Doc 1](../path/to/doc.md) - Brief description
- [Related Doc 2](path/to/doc.md) - Brief description
```

**Step 7.4: Improve cross-references**

Use descriptive link text:
```markdown
<!-- Good -->
See the [SIMD optimization guide](../optimizations/simd.md) for details.

<!-- Bad -->
See [here](../optimizations/simd.md) for details.
```

### Phase 9: Accuracy Verification

**Critical**: All documentation claims must be verified against actual code.

**Step 8.1: Verify API examples compile**

For each doc with code examples:
```bash
# Extract code snippets from markdown
# Test they compile with current API

# High priority files:
# - docs/guides/user-guide.md (BitVec, BalancedParens, JsonIndex examples)
# - docs/getting-started/quickstart.md (new file)
# - docs/getting-started/examples.md (new file)
# - README.md (Quick Start examples)
```

**Verification checklist per API example**:
- [ ] Imports are correct (`use succinctly::...`)
- [ ] Function signatures match current code
- [ ] Method names haven't changed
- [ ] Return types are correct
- [ ] Example compiles with `rustc --test`

**Step 8.2: Verify CLI commands work**

Test every command mentioned in `docs/guides/cli.md`:
```bash
# Build CLI
cargo build --release --features cli

# Test each command from docs:
./target/release/succinctly json generate 10kb -o test.json
./target/release/succinctly jq '.name' test.json
./target/release/succinctly yq '.name' test.yaml
./target/release/succinctly jq-locate test.json --offset 42
# ... etc for all commands in docs/guides/cli.md
```

**Step 8.3: Verify module structure claims**

Compare docs/guides/developer-guide.md with actual src/:
```bash
# Check module structure matches
tree src/ | diff - <(grep "src/" docs/guides/developer-guide.md | extract_tree)

# Verify all mentioned modules exist:
# - src/bits/
# - src/trees/
# - src/json/
# - src/yaml/
# - src/dsv/
# - src/jq/
```

**Step 8.4: Verify feature flags**

Check yaml-build-matrix.md against Cargo.toml and code:
```bash
# Flags mentioned in yaml-build-matrix.md:
grep -E "(broadword-yaml|scalar-yaml)" Cargo.toml
grep -E "(broadword-yaml|scalar-yaml)" src/yaml/simd/mod.rs

# If flags don't exist or don't work as documented:
#   → Update yaml-build-matrix.md or delete if obsolete
```

**Step 8.5: Verify parsing architecture claims**

For each file in docs/parsing/:
- [ ] json.md: Check PFSM table-driven parser exists
- [ ] yaml.md: Verify P0-P10 claims against actual code
- [ ] dsv.md: Check quote-aware indexing implementation

**Step 8.6: Check benchmark reproduction**

Verify all `cargo bench` commands work:
```bash
# From benchmark docs, test each command:
cargo bench --bench jq_comparison
cargo bench --bench yq_comparison
cargo bench --bench yaml_bench
cargo bench --bench dsv_bench
# ... etc
```

**Step 8.7: Update outdated performance numbers**

For all benchmark tables:
- Add date run (e.g., "as of 2026-01-18")
- Add platform (e.g., "Apple M1 Max" or "AMD Ryzen 9 7950X")
- Verify numbers are current or clearly labeled as historical

**Step 8.8: Fix incomplete/TODO sections**

Search for:
```bash
grep -r "TODO\|TBD\|FIXME\|XXX" docs/ --exclude-dir=archive
```

Elaborate or remove placeholders:
- Incomplete sections → Add content or remove
- TODO markers → Implement or remove
- Thin sections → Add detail and examples

**Step 8.9: Verify cross-references**

All internal links must point to existing files/sections:
```bash
# Extract all markdown links
grep -r "](.*\.md" docs/ | extract_links

# Check each link:
# - File exists
# - Section anchor exists (if using #anchor)
# - Path is correct (relative from source doc)
```

**Step 8.10: Elaborate thin documentation**

Minimum standards for each doc type:
- **User guide sections**: At least 1 code example per API
- **Architecture docs**: Explanation + diagram/example
- **Optimization docs**: Technique + usage + performance impact
- **Benchmark docs**: Methodology + numbers + reproduction steps

### Phase 10: .gitignore Updates

**Step 9.1: Ignore generated benchmark files**
```bash
cat > data/bench/results/.gitignore << 'EOF'
# Generated benchmark output files
*.md
*.jsonl
EOF
```

**Step 9.2: Update root .gitignore**

Add comment:
```gitignore
# Benchmark results (generated by CLI tool)
data/bench/results/*.md
data/bench/results/*.jsonl
```

---

## Appendix A: docs/README.md Full Content

```markdown
# Succinctly Documentation

Welcome to the succinctly documentation! This page helps you find what you need.

## 🎯 Quick Links by Audience

### 🚀 First-Time Users
**Start here**: [Getting Started Guide](getting-started/)

Learn the basics in 5 minutes:
- [Installation](getting-started/installation.md)
- [Quickstart Tutorial](getting-started/quickstart.md)
- [Common Examples](getting-started/examples.md)

### 📖 Library Users
**Using succinctly in your Rust project**:
- User Guide (`guides/user.md`) - Comprehensive API reference with examples
- CLI Guide (`guides/cli.md`) - Command-line tool reference

### 🤝 Contributors
**Want to contribute?**
- [CONTRIBUTING.md](../CONTRIBUTING.md) - Start here
- [Developer Guide](guides/developer-guide.md) - Codebase architecture and workflow
- [Release Guide](guides/release-guide.md) - Release process (for maintainers)

### ⚡ Performance Engineers
**Optimizing performance**:
- [Optimization Techniques](optimizations/) - 11 comprehensive guides
- [Quick Reference](optimizations/quick-reference.md) - One-page technique lookup table
- [Benchmarks](benchmarks/) - Performance comparisons vs other tools

### 🏗️ Researchers & Deep Divers
**Understanding internals**:
- [Architecture](architecture/) - Design decisions and core concepts
- [Parsing Implementation](parsing/) - JSON/YAML/DSV parser internals
- [Implementation Plans](plan/) - Feature planning documents
- [Archive](archive/) - Historical context and failed experiments

### 🤖 AI-Assisted Development
- [CLAUDE.md](../CLAUDE.md) - Comprehensive guide for AI assistants

---

## 📚 Documentation Structure

### [getting-started/](getting-started/)
Quick tutorials for new users. Start here if you've never used succinctly.

### [guides/](guides/)
Practical how-to documentation:
- API usage (`user.md`)
- CLI tool (`cli.md`)
- Development (`developer.md`)
- Releases (`release.md`)

### [architecture/](architecture/)
Design documentation:
- Core concepts (BitVec, BalancedParens, semi-indexing)
- Module structure
- Implementation decisions

### [parsing/](parsing/)
Parser implementation details:
- JSON semi-indexing
- YAML parser with P0-P10 optimizations
- DSV (CSV/TSV) parsing

### [optimizations/](optimizations/)
Performance optimization techniques:
- 11 comprehensive technique guides
- Decision framework
- Successes AND failures documented

### [benchmarks/](benchmarks/)
Performance comparisons:
- vs jq (JSON queries)
- vs yq (YAML queries)
- vs Rust JSON parsers (serde_json, sonic-rs, simd-json)
- Cross-language parser comparisons
- DSV performance

### [plan/](plan/)
Implementation plans for major features (all implemented).

### [archive/](archive/)
Historical documentation:
- Optimization history (successes and failures)
- Haskell reference implementations
- Migration notes

---

## 🔍 Finding What You Need

**I want to...**

- ✅ **Install and try succinctly** → [getting-started/](getting-started/)
- ✅ **Use BitVec or BalancedParens** → [guides/user-guide.md](guides/user-guide.md)
- ✅ **Query JSON files** → `guides/cli.md#jq-command`
- ✅ **Query YAML files** → `guides/cli.md#yq-command`
- ✅ **Understand how JSON indexing works** → [parsing/json.md](parsing/json.md)
- ✅ **See YAML optimization journey** → [parsing/yaml.md](parsing/yaml.md)
- ✅ **Learn SIMD techniques** → [optimizations/simd.md](optimizations/simd.md)
- ✅ **Compare performance** → [benchmarks/](benchmarks/)
- ✅ **Contribute code** → [CONTRIBUTING.md](../CONTRIBUTING.md) + [guides/developer-guide.md](guides/developer-guide.md)
- ✅ **Release a new version** → [guides/release-guide.md](guides/release-guide.md)
- ✅ **Understand why AVX-512 was rejected** → [archive/optimizations/](archive/optimizations/)

---

## 📝 Contributing to Documentation

Found a typo or want to improve docs? See [CONTRIBUTING.md](../CONTRIBUTING.md).

Documentation follows these conventions:
- US spelling (optimize, not optimise)
- Breadcrumbs at top of nested docs
- Links use descriptive text (not "click here")
- Code examples are tested and runnable
```

---

## Success Criteria

✅ **Discoverability**
- Clear entry points for 4 audiences (first-time users, library users, contributors, performance engineers)
- README.md has "Documentation" section routing to audience paths
- docs/README.md provides comprehensive map with "I want to..." section

✅ **Organization**
- Root has ≤7 .md files (README, CLAUDE, CHANGELOG, CODE_OF_CONDUCT, CONTRIBUTING, LICENSE)
- Logical grouping: getting-started/, guides/, architecture/, benchmarks/, optimizations/, parsing/, plan/
- Archive clearly separated with README explaining purpose

✅ **Reduced Redundancy**
- CLI documented once (`cli.md`), linked from user guide
- DSV consolidated: parsing/dsv.md (implementation), benchmarks/dsv.md (numbers)
- Planning docs in docs/plan/ (all implemented)
- Benchmark docs in docs/benchmarks/ (6 files, clear purpose)

✅ **Navigation**
- Every directory has README.md
- Breadcrumbs at top of docs
- "See also" links between related docs
- All cross-references updated

✅ **Quality**
- No broken links (verify with link checker)
- Consistent formatting (headers, breadcrumbs)
- US spelling throughout
- Generated files in .gitignore
- PR template references CONTRIBUTING.md

✅ **US Spelling**
- optimisations → optimizations
- colour → color, favour → favor, behaviour → behavior
- centre → center, analyse → analyze

---

## Migration Checklist

### Pre-Refactor
- [ ] Back up current docs (git tag docs-pre-refactor)
- [ ] Audit all internal links (grep for `](.*\.md)`)
- [ ] Identify all external references (blog posts, issues linking to docs)

### Phase 1: Verify Current State
- [ ] Check yaml-build-matrix.md feature flags exist in Cargo.toml
- [ ] Verify broadword-yaml, scalar-yaml flags in src/yaml/simd/mod.rs
- [ ] Decision: Keep in docs/plan/ or delete

### Phase 2: Create New Structure
- [ ] mkdir -p docs/{getting-started,guides,architecture,benchmarks,archive/optimizations}
- [ ] Create docs/README.md (documentation map)
- [ ] Create docs/getting-started/README.md
- [ ] Create docs/getting-started/installation.md (extract from user-guide)
- [ ] Create docs/getting-started/quickstart.md (new 5-min tutorial)
- [ ] Create docs/getting-started/examples.md (common patterns)
- [ ] Create docs/guides/README.md
- [ ] Create docs/architecture/README.md
- [ ] Create docs/architecture/core-concepts.md (new)
- [ ] Create docs/architecture/bitvec.md (new)
- [ ] Create docs/architecture/balanced-parens.md (new)
- [ ] Create docs/architecture/semi-indexing.md (new)
- [ ] Create docs/benchmarks/README.md
- [ ] Create docs/benchmarks/dsv.md (extract from dsv-performance)
- [ ] Create docs/optimizations/quick-reference.md (new)
- [ ] Create docs/plan/README.md
- [ ] Create docs/archive/README.md

### Phase 3: Move & Rename
- [ ] git mv CLI.md docs/guides/cli.md
- [ ] git mv RELEASE.md docs/guides/release.md
- [ ] git mv PLAN-JQ.md docs/plan/jq-implementation.md
- [ ] git mv docs/jq-comparison.md docs/benchmarks/jq.md
- [ ] git mv docs/yq-comparison.md docs/benchmarks/yq.md
- [ ] git mv docs/rust-json-comparison.md docs/benchmarks/rust-parsers.md
- [ ] git mv docs/CROSS-PARSER-BENCHMARKS.md docs/benchmarks/cross-parser.md
- [ ] git mv docs/optimisations docs/optimizations
- [ ] git mv docs/archive/OPTIMIZATION-SUMMARY.md docs/archive/optimizations/SUMMARY.md
- [ ] git mv docs/archive/failed-optimizations.md docs/archive/optimizations/failed.md
- [ ] git mv docs/archive/implemented-optimizations.md docs/archive/optimizations/implemented.md
- [ ] git mv docs/archive/avx512-*.md docs/archive/optimizations/
- [ ] git mv docs/archive/between-avx2-and-avx512.md docs/archive/optimizations/
- [ ] git mv docs/archive/optimization-opportunities.md docs/archive/optimizations/opportunities.md
- [ ] git mv docs/archive/performance-*.md docs/archive/optimizations/
- [ ] git mv docs/archive/recommended-optimizations.md docs/archive/optimizations/recommended.md
- [ ] Decision on yaml-build-matrix.md (move to plan/ or delete)

### Phase 4: Consolidate
- [ ] Merge dsv-performance.md + dsv-profiling-analysis.md → parsing/dsv.md
- [ ] Create benchmarks/dsv.md with just numbers
- [ ] git rm docs/dsv-performance.md
- [ ] git rm docs/dsv-profiling-analysis.md
- [ ] Remove CLI section from user-guide.md (lines 383-461)
- [ ] Add cross-reference to cli.md

### Phase 6: US Spelling Pass
- [ ] Find/replace: optimisation → optimization (all .md files in docs/)
- [ ] Find/replace: optimisations → optimizations
- [ ] Find/replace: colour → color
- [ ] Find/replace: favour → favor
- [ ] Find/replace: behaviour → behavior
- [ ] Find/replace: centre → center
- [ ] Find/replace: analyse → analyze
- [ ] Review: honour → honor, labour → labor

### Phase 6: Update Links
- [ ] Update README.md (add Documentation section)
- [ ] Update CONTRIBUTING.md (link to developer-guide, release-guide)
- [ ] Update .github/pull_request_template.md
- [ ] Find/replace: `](CLI.md)` → `](docs/guides/cli.md)`
- [ ] Find/replace: `](RELEASE.md)` → `](docs/guides/release.md)`
- [ ] Find/replace: `](PLAN-JQ.md)` → `](docs/plan/jq-implementation.md)`
- [ ] Find/replace: `](docs/optimisations/)` → `](docs/optimizations/)`
- [ ] Find/replace: `](docs/jq-comparison.md)` → `](docs/benchmarks/jq.md)`
- [ ] Find/replace: `](docs/yq-comparison.md)` → `](docs/benchmarks/yq.md)`
- [ ] Find/replace: `](docs/rust-json-comparison.md)` → `](docs/benchmarks/rust-parsers.md)`
- [ ] Find/replace: `](docs/CROSS-PARSER-BENCHMARKS.md)` → `](docs/benchmarks/cross-parser.md)`
- [ ] Find/replace: `](docs/dsv-performance.md)` → `](docs/parsing/dsv.md)`
- [ ] Find/replace: `](docs/dsv-profiling-analysis.md)` → `](docs/parsing/dsv.md)`
- [ ] Update relative paths in moved files

### Phase 7: Navigation
- [ ] Add breadcrumbs to all nested docs
- [ ] Add "See also" sections between related docs
- [ ] Verify all directories have README.md
- [ ] Standardize headers (Title, description, TOC for >200 lines)

### Phase 8: Quality
- [ ] Add .gitignore to data/bench/results/
- [ ] Update root .gitignore with comment
- [ ] Run link checker (all docs)
- [ ] Test navigation from README
- [ ] Verify 4 audience journeys work
- [ ] Build rustdoc (check for broken links)

### Phase 11: Broken Link Checking

**Critical**: All links must be validated before final commit.

**Step 11.1: Install link checker**
```bash
# Install markdown-link-check (npm)
npm install -g markdown-link-check

# Or use lychee (Rust, faster)
cargo install lychee
```

**Step 11.2: Check all documentation links**
```bash
# Option 1: markdown-link-check
find docs -name "*.md" -exec markdown-link-check {} \;
markdown-link-check README.md
markdown-link-check CONTRIBUTING.md
markdown-link-check CLAUDE.md

# Option 2: lychee (faster, better output)
lychee --offline docs/**/*.md README.md CONTRIBUTING.md CLAUDE.md

# Check only internal links (skip external URLs)
lychee --offline --exclude-external docs/ *.md
```

**Step 11.3: Fix broken links**

For each broken link found:
1. Determine if target exists at old or new location
2. Update link to correct path
3. Verify relative paths are correct
4. Re-run link checker to confirm fix

**Common broken link patterns to check**:
```bash
# Links to moved files
grep -r "](CLI.md)" docs/
grep -r "](RELEASE.md)" docs/
grep -r "](PLAN-JQ.md)" docs/
grep -r "](docs/optimisations/" docs/
grep -r "](docs/jq-comparison.md)" docs/
grep -r "](docs/yq-comparison.md)" docs/

# Links with incorrect relative paths
grep -r "](\.\./\.\./\.\./docs/" docs/  # Too many ../
grep -r "](docs/docs/" docs/             # Duplicate path component
```

**Step 11.4: Validate anchor links**
```bash
# Check section anchors exist
# Extract all #anchor links
grep -roh "](#[^)]*)" docs/ | sort -u

# Verify each anchor:
# - Section heading exists in target file
# - Anchor text matches heading (lowercase, hyphens)
```

**Step 11.5: Check external links (optional)**
```bash
# Check external URLs are reachable
lychee --max-redirects 5 docs/**/*.md

# Note: Some external links may be intentionally dead (archive.org, etc.)
# Focus on fixing broken internal links first
```

### Phase 12: Final Validation & Commit
- [ ] Run link checker on all .md files
- [ ] Fix all broken internal links
- [ ] Verify 4 audience journeys work (click through paths)
- [ ] Test all code examples compile (spot check)
- [ ] Build rustdoc: `cargo doc --no-deps` (check for warnings)
- [ ] git commit -m "docs: reorganize documentation structure"
- [ ] Add entry to CHANGELOG.md under [Unreleased]
- [ ] Update CLAUDE.md if it references moved files

---

## Timeline Estimate

- **Phase 1** (Verify): 30 minutes
- **Phase 2** (Create): 5-6 hours
- **Phase 3** (Filename Review): 1 hour
- **Phase 4** (Move): 1-2 hours
- **Phase 5** (Consolidate): 3-4 hours
- **Phase 6** (US Spelling): 1-2 hours
- **Phase 7** (Links): 2-3 hours
- **Phase 8** (Navigation): 2-3 hours
- **Phase 9** (Accuracy Verification): 4-6 hours
- **Phase 10** (Quality): 2-3 hours
- **Phase 11** (Broken Links): 1-2 hours
- **Phase 12** (Final Validation): 1-2 hours

**Total**: 24-35 hours of focused work (includes filename rationalization, accuracy verification, and link checking)

---

## Risk Mitigation

### Broken External Links
**Risk**: GitHub issues, blog posts, external docs may link to old paths
**Mitigation**:
- Use `git mv` (preserves history, GitHub auto-redirects)
- Document old → new mappings in archive/README.md
- Add note to CHANGELOG.md about refactor

### Search Engine Impact
**Risk**: Google results may point to old URLs
**Mitigation**:
- `git mv` preserves history (GitHub redirects)
- Update docs.rs links if package published

### Contributor Confusion
**Risk**: Contributors may reference old docs in PRs
**Mitigation**:
- Update PR template to reference new paths
- Add note to CONTRIBUTING.md about refactor date
- Document in docs/archive/README.md

### Link Rot
**Risk**: Updating all links may introduce errors
**Mitigation**:
- Use automated link checker before/after
- Test representative paths manually
- Careful find/replace with review

---

## Post-Refactor Tasks

1. Update .claude/skills/ to reference new paths
2. Update .omni-dev/ to reference new paths (if they reference moved docs)
3. Add note to CHANGELOG.md about documentation refactor
4. Consider announcement in README about refactor date (for external refs)
5. Verify docs.rs build still works (if published)
