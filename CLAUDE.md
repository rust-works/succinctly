# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Skills Reference

Detailed guidance is organized into skills in `.claude/skills/`. Claude will auto-invoke these based on context:

| Skill                  | Triggers                                    | Purpose                                    |
|------------------------|---------------------------------------------|--------------------------------------------|
| **benchmark-docs**     | "benchmark", "update benchmarks"            | Platform-specific benchmark documentation  |
| **markdown-tables**    | "format table", "align table"               | Fixed-width table formatting               |
| **simd-optimization**  | "SIMD", "AVX", "NEON", "vectorization"      | SIMD patterns and learnings                |
| **bit-optimization**   | "rank", "select", "popcount", "lookup table"| Bit-level optimization patterns            |
| **json-semi-indexing** | "JSON index", "semi-index", "cursor"        | JSON parsing implementation details        |
| **yaml-semi-indexing** | "YAML index", "YAML parser", "sequence item"| YAML parsing and debugging patterns        |
| **testing**            | "test", "assert", "coverage", "regression"  | Test quality patterns and anti-patterns    |
| **commit-msg**         | "commit message", "amend commit"            | Conventional commits format                |
| **knowledge-map**      | "knowledge map", "wiki", "update wiki"      | Maintain docs/ knowledge wiki              |
| **skill-writing**      | "create skill", "SKILL.md", "write skill"   | Best practices for writing Claude skills   |

## Knowledge Wiki

A structured knowledge base for this codebase lives in `docs/`. Start at [docs/index.md](docs/index.md) for a concept-oriented map of how the data structures, algorithms, SIMD implementations, and benchmarks relate to each other. The wiki pages cross-link to existing architecture docs, parsing docs, source files, and academic papers.

## Changelog Fragments

Never edit `CHANGELOG.md` in a PR: every PR touching the top of `[Unreleased]` made any two PRs in flight conflict (#3920). Add `changelog.d/<issue>.<type>.md` instead — `<type>` is `added`, `changed`, `deprecated`, `removed`, `performance`, `fixed` or `security`; `+<slug>` replaces the issue number when there is none; `.<n>` before `.md` is a second entry. The body is the entry exactly as it would have read in `CHANGELOG.md` (`- **bold lead-in** (#N).` plus prose and measured numbers); a follow-up that refreshes an entry edits that same file. `python3 scripts/changelog.py check` validates; a PR with no user-visible effect waives the CI check with a `[no changelog]` line in its body (or the `no-changelog` label). `scripts/changelog.py collect` assembles the release section — see [changelog.d/README.md](changelog.d/README.md) and [docs/guides/release.md](docs/guides/release.md).

## Limitation Pages: One Sentence Per Line

In `docs/compliance/{jq,yq,yaml}/limitations.md`, put each new sentence on its own source line (at
the paragraph's indentation) instead of appending it to an existing line: concurrent PRs that
extend the same paragraph then merge line by line (#5048, STYLE-0014). `python3
scripts/doc_lines.py check` caps prose lines at 1000 characters; `fix` splits overlong ones. A rebase
that conflicts in one of these pages because `fix` split a paragraph upstream is resolved by
`python3 scripts/doc_lines.py resolve <file>` (it splits all three sides, then merges them).

## macOS CI Runs in the Merge Queue Only

The macOS test matrix (`Test (macOS ARM64)`) runs in merge-group runs, not on pull requests: macOS
runners are the one CI resource that queues, and PR runs used most of them (#5699). A PR runs it
only if it touches `src/jq/math.rs`, a `build.rs`, a `Cargo.toml`/`Cargo.lock`, `rust-toolchain*`
or `.github/workflows/**`, or carries the `ci:macos` label (add the label, then push or re-run the
jobs). On any other PR the required check reports success with a notice, so a macOS-only failure
surfaces as a merge-queue ejection instead; label a PR `ci:macos` when it touches something
platform-sensitive that the path list misses.

## Before You Push

`scripts/pre-push.sh` maps the files your branch changed to the CI gates they can fail and runs
those (`--full` adds the lints CI runs: three clippy invocations, rustdoc, `--no-default-features`;
`list` shows the gates). `.claude/settings.json` runs it as a `PreToolUse` hook, so an agent's
`git push` is blocked while it fails -- `--no-verify` does not skip it. `PRE_PUSH_SKIP="gate ..."`
skips named gates (as a prefix of the push, `PRE_PUSH_SKIP=ledger-only git push`, for an agent), and a
skip is always printed.
The hook reads only the command text, so `git -C "$WT" push` is resolved from a `WT=...` assignment earlier in the
same command (`WT=/path; git -C "$WT" push`), as the shell would: repeat the assignment in every call, since a variable
set in an earlier call is gone, and a prefix assignment (`WT=/path git -C "$WT" push`) does not count. A directory it
still cannot resolve, or that is no repository, is refused rather than let through (#5850). The gates are checked against
`.github/workflows/*.yml` by `python3 scripts/test_pre_push.py`: a new CI gate has to be mirrored in
`scripts/pre_push.py` or listed there as deliberately not (#5585).

A fix for a `to-triage` entry in `comparison/jq/divergences/` (one file per entry, #5769) deletes that entry's file in the same
change, or CI's `Ledger drift` job fails on "listed but no longer diverging" (#5583 needed a third
commit for it). `python3 scripts/ledger_refs.py check` names the entry in milliseconds; a branch
that only mentions the issue says `[ledger keeps #N]` on its own line in a commit message.
`succinctly dev compare jq --ledger-only` is the stale-entry half of that CI job in about a minute,
against `/usr/bin/jq` when the pinned 1.7.1 is not built.

## AI Scratch Directory

Use `.ai/scratch/` for temporary files (git-ignored):

```bash
mkdir -p .ai/scratch
```

When manually exercising `jq`/`yq` CLI behavior (trying flags, checking output, `-i`
inplace edits), use a file under `.ai/scratch/`, not a tracked fixture elsewhere in the
repo (e.g. `users.yaml`) — those are committed samples, not scratch space.

## Project Overview

Succinctly is a high-performance Rust library implementing succinct data structures with fast rank and select operations, optimized for both x86_64 (AVX-512 VPOPCNTDQ where available; when `count_ones()` is `POPCNT`: [simd.md](docs/optimizations/simd.md#per-word-popcounts-on-baseline-x86_64-5035)) and ARM (NEON) architectures.

### Semi-Indexing Architecture

Succinctly uses **semi-indexing** rather than traditional DOM parsing. Instead of building a complete in-memory tree, it creates a lightweight structural index (~3-6% overhead) and extracts values lazily. This enables:

- **18-46x less memory** than DOM parsers
- **Fast queries** because only accessed values are materialized
- **Streaming output** without intermediate allocations

**Trade-off**: Semi-indexing performs minimal validation compared to full parsers (jq, yq). The benchmark comparisons to jq/yq reflect both architectural advantages and reduced validation work. See [docs/architecture/semi-indexing.md](docs/architecture/semi-indexing.md) for details.

## Common Commands

### Building and Testing

```bash
cargo build                              # Standard build
cargo build --features simd              # With SIMD popcount
cargo test                               # Run tests
cargo test --features large-tests        # 1GB bitvector tests
cargo bench                              # Run benchmarks
```

### Coverage

CI runs coverage through `.github/workflows/coverage.yml` (x86_64 + ARM64): three test shards per
architecture (`scripts/coverage-shards.py` defines which tests each runs; a new test file lands in the
`rest` shard by itself, #5711), each producing an lcov with `cargo-llvm-cov`, then a report job that
combines them with [`action-works/patchcov-action`](https://github.com/action-works/patchcov-action)'s
thin mode (`patchcov diff`) and posts PR patch coverage as a sticky comment. To reproduce the CI line-coverage number locally,
use the same feature set CI uses:

```bash
# TOTAL line % (matches the CI `test-args`; the fail-under-lines gate is set just below this)
cargo llvm-cov --features cli,simd,regex,serde --workspace --summary-only --fail-under-lines 0

# PR diff / patch coverage (added lines covered + uncovered file:line list), like the CI comment
cargo llvm-cov report --lcov --output-path target/coverage.lcov
patchcov diff --report target/coverage.lcov
```

The two stack-filling `jq_cli_tests` listed in `scripts/deep-recursion-tests.sh` (each fills a 2 GB
native stack; the list also holds the 18 slowest tests of #5494 and four #5554 tests that build a value past the floor) are `#[ignore]`d under `cfg(coverage)`,
so `cargo llvm-cov` reports them as ignored: they cover no line the rest of the suite does not (#3750),
and they still run in the `deep-recursion` CI leg and under a plain `cargo test`.

### CLI Tool

```bash
cargo build --release --features cli

# Install short aliases (sjq, syq, sjq-locate, syq-locate)
./target/release/succinctly install-aliases          # symlinks next to binary
./target/release/succinctly install-aliases --dir ~/bin  # or specify directory

# JSON operations (sjq is an alias for succinctly jq)
sjq '.users[].name' input.json
sjq -r '.users[] | [.name, .age] | @csv' input.json
sjq -r '.users[] | [.name, .age] | @dsv("|")' input.json
sjq --validate '.users[]' input.json  # Strict RFC 8259 validation before processing
sjq-locate input.json --offset 42
sjq-locate input.json --line 5 --column 10

# JSON validation (RFC 8259 strict)
./target/release/succinctly json validate input.json
./target/release/succinctly json validate --quiet input.json  # Exit code only

# YAML operations (syq is an alias for succinctly yq)
syq '.users[].name' config.yaml
syq -o json '.' config.yaml         # Output as JSON
syq '.spec.containers[]' k8s.yaml
syq --doc 0 '.' multi-doc.yaml      # First document only
syq --jq-extensions '[paths]' config.yaml  # opt into jq-only builtins yq's lexer rejects (#1512)
syq-locate config.yaml --offset 42
syq-locate config.yaml --line 5 --column 10

# DSV/CSV operations
sjq --input-dsv ',' '.[] | select(.[0] == "Alice")' data.csv
./target/release/succinctly dsv generate 1mb -p users -o users.csv
./target/release/succinctly dsv generate 1mb -p users --no-header -o users.csv  # without header row

# Data generation
./target/release/succinctly json generate 10mb -o benchmark.json

# Benchmarks (requires: cargo build --release --features bench-runner)
./target/release/succinctly bench run jq_bench
./target/release/succinctly bench run yq_bench
./target/release/succinctly dev bench yq --queries all  # M2 streaming comparison (memory collected by default)
./target/release/succinctly bench run dsv_bench

# Distributed benchmark orchestration across SSH nodes (issue #98)
cp nodes.yaml.example nodes.yaml && $EDITOR nodes.yaml  # gitignored, machine-specific
./target/release/succinctly bench nodes --config nodes.yaml --status
./target/release/succinctly bench sync --config nodes.yaml
./target/release/succinctly bench orchestrate --config nodes.yaml --all --dry-run
./target/release/succinctly bench report --current data/bench/distributed/<run_id> --baseline <prior-run>
```

### Multi-call Aliases

The binary supports multi-call invocation via symlinks. When invoked as `sjq`, `syq`, etc., it dispatches directly to the corresponding subcommand:

| Alias         | Equivalent              | Installed by default |
|---------------|-------------------------|----------------------|
| `sjq`         | `succinctly jq`         | Yes                  |
| `syq`         | `succinctly yq`         | Yes                  |
| `sjq-locate`  | `succinctly jq-locate`  | Yes                  |
| `syq-locate`  | `succinctly yq-locate`  | Yes                  |
| `jq`          | `succinctly jq`         | No (recognized only) |
| `yq`          | `succinctly yq`         | No (recognized only) |

Run `succinctly install-aliases` to create symlinks, or create them manually:

```bash
ln -s $(which succinctly) ~/bin/sjq
```

## Code Architecture

### Module Structure

```
src/
├── lib.rs              # Public API, RankSelect trait
├── bits/               # BitVec, rank/select, popcount
├── trees/              # Balanced parentheses
├── json/               # JSON semi-indexing (PFSM default)
├── yaml/               # YAML semi-indexing (oracle parser)
├── dsv/                # DSV/CSV semi-indexing (BMI2/SIMD)
├── jq/                 # jq query language and evaluator
└── bin/                # CLI tool (jq, yq, jq-locate, yq-locate)
```

### Public API

```rust
use succinctly::bits::BitVec;
use succinctly::trees::BalancedParens;
use succinctly::json::JsonIndex;
use succinctly::yaml::YamlIndex;
use succinctly::dsv::DsvIndex;
use succinctly::jq::{parse, eval};
```

### Core Data Structures

| Structure         | Description                            | Performance (x86_64 Zen 4) |
|-------------------|----------------------------------------|----------------------------|
| **BitVec**        | O(1) rank, sampled select              | ~28-48% overhead           |
| **BalancedParens**| Succinct tree navigation               | ~58% overhead              |
| **JsonIndex**     | JSON semi-indexing with PFSM parser    | ~880 MiB/s                 |
| **YamlIndex**     | YAML semi-indexing with oracle parser  | ~250-400 MiB/s             |
| **DsvIndex**      | DSV semi-indexing with lightweight rank| 85-1676 MiB/s (API)        |

### jq Format Functions and Assignment Operators

The format table (`@csv`, `@tsv`, `@dsv(delim)`, `@json`, `@text`, `@uri`, `@urid`, `@base64`, `@base64d`, `@html`, `@sh`, `@yaml`, `@props`) and the assignment operators (`=`, `|=`, `+=`, `-=`, `*=`, `/=`, `%=`, `//=`, `del()`) are in
[docs/reference/jq-language.md](docs/reference/jq-language.md#format-strings) and its
[assignment section](docs/reference/jq-language.md#assignment-operators), with examples. The rules that are easy to get wrong:

- **`@uri`/`@base64`/`@html` on an array or object (jq mode only):** the container is JSON-encoded first, as real jq does (`[1,2] | @uri` => `"%5B1%2C2%5D"`). `succinctly yq` rejects a container for all three, as real yq does (#1096), and `@html` needs `--jq-extensions` there (#5444).
- **Format-prefixed interpolation (jq mode only):** `@fmt "lit\(E)lit"` formats each `\(E)` slot and leaves the literal text alone; a literal with no interpolation never runs the format. yq's lexer rejects a format followed by a string, and `succinctly yq` matches (#3316).
- **`@text` and `@urid`:** `@text` is `tostring`. `@urid` (URI percent decoding) is new in jq 1.8; `succinctly jq` follows 1.8, where jq 1.7.1 answers `urid is not a valid format` (the `str-format-urid` ledger entry).
- **`@dsv(delim)`:** every string field is double-quoted whatever its content (as `@csv` does), non-strings are bare, null is empty, an inner `"` is doubled, and `@dsv(",")` is identical to `@csv`.
- **`*`/`*=`:** two objects merge recursively in both modes (real jq does too). Two arrays are replaced wholesale in yq mode only; `succinctly jq` errors on array `*`, as real jq does.

### The rule the divergence sections below are exceptions to

**`succinctly jq` follows jq; `succinctly yq` follows yq — and the *mode* decides, never the
input format.** Bug-for-bug fidelity is the default, a reference tool's own inconsistencies
included. Divergence is permitted only where the reference emits output it cannot itself read
back, where matching would corrupt data or discard a write, or where matching would take the
host process down — and every divergence must be recorded in
[docs/compliance/jq/limitations.md](docs/compliance/jq/limitations.md) or
[docs/compliance/yq/limitations.md](docs/compliance/yq/limitations.md). A fork between
candidate behaviours is decided in that order, in both modes: first whether one of those
conditions lets us refuse the reference's behaviour, then which option matches the pinned
reference more closely, and only when neither separates them, which option performs better
or uses less memory (ADR-0018's #2416 amendment). Behavioural rules
therefore belong on `EvalSemantics` (per-mode), not on per-format traits like
`DocumentFields`.

Never state a jq/yq behaviour from memory — capture it from the pinned binary (`/usr/bin/jq`
1.7.1, Homebrew `yq` v4.53.3). That applies to "neither tool has this" as much as to any
other claim: `--front-matter`, `--split-exp` and cross-file evaluation all look like
succinctly extensions and are all real yq features (#715). Calling reference surface an
extension is the costly direction of that mistake, because extensions are exempt from the
divergence rule above. See [docs/adrs/adr-0018.md](docs/adrs/adr-0018.md).

A floating-point *digit* from a math builtin (`sin`, `tan`, `exp`, `pow`, ...) is a claim
about a platform, not just a version: jq calls the platform libm, and Apple's and glibc's
disagree in the last bit on ~40% of ordinary `tan` inputs. succinctly calls the same libm
(`src/jq/math.rs`, #3045), so capture such digits on the platform in question and never pin
one in a hermetic golden without checking the other libm agrees
(`scripts/jq-libm-oracle-sweep.sh`).

### yq Merge-Flag Suffixes on `*`/`*=` (yq mode only)

Real yq extends `*`/`*=` with combinable flag suffixes that control merge semantics. They go directly after `*` for the plain (non-assign) form, or after `*=` for the in-place form — never between `*` and `=` (`.a *+= .b` is not valid; the flags belong after the `=`):

| Flag | Meaning                                                                                    |
|------|--------------------------------------------------------------------------------------------|
| `+`  | Append arrays instead of replacing them                                                    |
| `?`  | Only update fields/indices that already exist; never create new                            |
| `n`  | Only write fields/indices that don't already exist (or are `null`)                         |
| `d`  | Deep-merge arrays: treat them like objects, merging by index                               |
| `c`  | Clobber custom tags (parsed but a no-op today — no tag data exists to preserve or clobber) |

Flags combine freely, in any order (`*+d` and `*d+` are identical):

```bash
printf 'a: [1, 2]\nb: [3, 4]\n' | succinctly yq '.a *=+ .b'   # a: [1, 2, 3, 4]  (append)
printf 'a:\n  x: 1\nb:\n  x: 2\n  y: 3\n' | succinctly yq '.a *=? .b'  # a: {x: 2}        (only-existing)
printf 'a:\n  x: 1\nb:\n  x: 2\n  y: 3\n' | succinctly yq '.a *=n .b'  # a: {x: 1, y: 3}  (only-new)
```

`?`/`n` propagate through every nesting depth: a parent key that already exists still gets recursed into so its own new children can be added or blocked individually — the gate never blocks recursion into a matching nested object/array itself, only the leaf writes within it. Combining `?` and `n` is an AND of both gates (net effect: only touch a field that already exists and is currently `null`).

`+`/`d` combined on the same array is a documented simplification: real yq has a surprising, untested-upstream double-effect here; succinctly makes `+` take clean priority (pure append, `d` ignored) instead.

`null` acts as an empty container on either side of a yq-mode merge (jq mode has no such exception -- every `null`-involving `*` pairing errors there, #1175): a null/absent *left* operand merges as if starting from `{}`/`[]` (`.a *=n .b` on `a: null` writes the full `.b` in; `.a *=? .b` on an absent `.a` leaves `a: {}`, blocked field-by-field rather than staying `null`), and a null *right* operand is always a no-op (`.a *= null` leaves `.a` untouched).

### `sub` divergence (yq mode only) — `gsub`/`scan`/`splits` aren't real yq builtins at all

Real yq's bare, 2-arg `sub(re; s)` replaces *every* match, not just the first — jq's `sub` = first match only, `gsub` = all matches; yq's bare `sub` behaves like jq's `gsub` unconditionally (`"aaa" | sub("a";"X")` => `"XXX"` in yq, `"Xaa"` in jq, confirmed against yq v4.53.3). `gsub` is not a real yq builtin at any arity — its lexer rejects `gsub(...)` outright, same as `scan`/`splits` (confirmed live against yq v4.53.3, #1436) — so there's no jq-model "match" to speak of for succinctly's own `gsub` in yq mode; it's a succinctly extension (ADR-0018 rule 5), gated behind `--jq-extensions` and off by default since #1512, not a verified divergence.

Real yq's 3-arg `sub(re; replacement; flags)` never evaluates `replacement` or `flags` at all — it always performs a global replace-with-empty-string using only the pattern (near-certainly an upstream Go bug reading its replacement from a fixed AST slot that's empty once arity exceeds 2, not a designed feature; confirmed live that an `error(...)` in either position never fires). Per ADR-0018 rule 3, succinctly reproduces this bug-for-bug rather than "fixing" it into jq's model (`"aaa" | sub("a";"X";"g")` => `""` in both yq and succinctly yq mode; flags like `"i"` are silently ignored, and a 4th+ argument is accepted and discarded, matching real yq's own parser leniency — #1122).

### Anchor/alias preservation and its soundness rule (yq mode only)

YAML-target output re-emits `&anchor`/`*alias` on both the cursor-streaming path and the
DOM path taken by a write (`=`, `|=`, `+=`, `del()`) or a DOM-forcing flag (`-P`,
`--arg`) — ADR-0017's mechanism 2, #763. `-o=json` still expands aliases, matching real
yq. A node's mark rides `NodeMeta` in the `CommentTree` side-tree alongside its comment
and style; `enforce_anchor_soundness` (`yq_runner.rs`) then drops any `*name` the
document cannot resolve.

Two routes carry no `CommentTree` at all and so preserve nothing — not anchors, not
comments, not style. Both predate #763 and neither is specific to anchors: a filter
yielding *multiple* results through a comma that is not a plain comma of document nodes
(`.x, (.y | length)`, `.x, .absent`, `(.x, .y) | f`) is made of copies and loses its cursor before
`evaluate_yaml_cursor` can capture one (`.x, .y` itself keeps its nodes, #1361), and
`--eval-all` evaluates through the owned-value route (#4380) -- unless its filter only
navigates or writes a literal (`m2_gate.rs`'s `eval_all_matches_per_document`), when the run
is rerouted through the plain route and keeps all three. `--inplace` used to be a third and no
longer is: since #1349 it keeps comments, style and anchors.

**succinctly never emits YAML it cannot read back — real yq does.** A `*name` is written
only when a matching `&name` exists, is emitted *earlier*, and holds an *equal* value.
Real yq fails all three in ordinary use (both verified against v4.53.3):

```bash
printf 'a: &x 1\nb: *x\n' | yq 'del(.a)'          # b: *x   <- no &x anywhere; yq then
                                                  #            rejects its own output with
                                                  #            "unknown anchor 'x' referenced"
printf 'a: &x 1\nb: *x\n' | succinctly yq 'del(.a)'   # b: 1
```

Real yq's `sort_keys` diverges the same way when sorting would move an alias above its
declaration.

**A write through an alias reaches the shared node (#1351).** yq decides by *path shape*: a
path that ends exactly at an alias (`.b = 5`, `.b |= 5`, `del(.b)`) rebinds that position;
one that passes through it and continues (`.b.p = 9`, `del(.b.p)`, `.b[0] = 9`,
`setpath(["b","p"]; 9)`) mutates the anchor's node, and every position follows. succinctly
keeps its copy model and recovers this in `jq::alias_identity`: for an alias-sensitive write
on an alias-bearing document, the write entry points redirect any resolved path whose proper
prefix is an alias position onto the anchor's path (transitively, and only while the alias
position still holds the anchor's value — a slot rebound earlier in the pipe is written
positionally), then mirror the anchor's new value back into every alias slot that was still
an untouched copy. Shared positions accumulate (`.[] .p += 1` over three positions is 4, as
in yq). Two recorded divergences: yq silently *discards* a value-producing update at an
alias node (`.b += 1`, `.b |= . + {..}`), which succinctly rebinds instead (rule 4(b)); and
with the anchor deleted earlier in the pipe there is nothing to redirect to, so the write
stays positional. The equal-value rule above is then the backstop: wherever the redirect
declines, the values differ and the mark is dropped rather than discarding the write.

### jq Extensions: Position Navigation and `leaf_paths`

Both are succinctly's own, not in jq 1.7.1 or yq; examples are in
[docs/reference/jq-language.md](docs/reference/jq-language.md#succinctly-extensions).

- **`at_offset(n)`, `at_position(line; col)`:** jump to the node at a 0-indexed byte offset, or a 1-indexed line and column (the arguments of `jq-locate --offset` / `--line --column`). An offset or position outside the document, or on no node, is an error; `at_offset(n)?` is empty instead.
- **`leaf_paths`:** not a real jq builtin (`leaf_paths/0 is not defined` in jq 1.7.1 and 1.8.2). It returns the path to every childless node, so it differs from the recipe `paths(scalars)` in two cases: `null` counts as a leaf, and so does an empty `{}`/`[]` (see `collect_leaf_paths` in `src/jq/eval.rs`, #771).
- **In `succinctly yq`** `leaf_paths` is gated behind `--jq-extensions`, like `paths`, `getpath`, `limit`, `gsub`/`scan`/`splits` and the rest of the jq-only surface yq's lexer rejects (#1512; the list is in [docs/reference/yq-language.md](docs/reference/yq-language.md#gated-jq-builtins---jq-extensions)). `succinctly jq` is unaffected.

## Feature Flags

| Feature             | Description                                                                                                   |
|---------------------|---------------------------------------------------------------------------------------------------------------|
| `std`               | Standard library (default, CPU detection)                                                                     |
| `simd`              | Explicit SIMD intrinsics                                                                                      |
| `portable-popcount` | Non-additive: replaces intrinsic popcount with the portable bitwise one                                       |
| `serde`             | Serde serialize/deserialize support                                                                           |
| `regex`             | Regex builtins in jq (included in `cli`)                                                                      |
| `cli`               | CLI tool (jq, yq, locate, generators)                                                                         |
| `local-zone`        | System local zone for jq `localtime`/`strflocaltime` via jiff (in `cli`; not Windows)                         |
| `locale-time`       | Localized `strftime`/`strflocaltime` names via libc `strftime_l` (in `cli`; Apple and Linux libc only; #5754) |
| `bench-runner`      | Unified benchmark runner (bench list/run)                                                                     |
| `large-tests`       | 1GB bitvector tests                                                                                           |
| `huge-tests`        | 5GB bitvector tests                                                                                           |
| `mmap-tests`        | Memory-mapped bitvector tests                                                                                 |
| `broadword-yaml`    | Non-additive: replaces NEON YAML scanning with broadword (SWAR) on ARM64                                      |
| `scalar-yaml`       | Non-additive: disables all SIMD/broadword YAML parsing                                                        |
| `select-stats`      | Adds a counter to five hot `select` scan loops (#40 measurement; never for timing runs)                       |
| `share-stats`       | Record every copy-on-write that copied, with its call site (#2999 clone audit)                                |

### `--all-features` is not a superset build (#3006)

`portable-popcount`, `broadword-yaml` and `scalar-yaml` are **non-additive**: each *replaces* the default with a slower or measurement-only build instead of adding to it. `cargo build --all-features` turns on all three, so that binary is a non-shipping build with no diagnostic saying so. (`select-stats` and `share-stats` are additive instrumentation, not part of this set, but must also never be on for a timing or shipped build.)

- **Clippy cannot see everything in one invocation.** `scalar-yaml` compiles out the YAML SIMD backend of the building architecture (`src/yaml/simd/x86.rs` on x86_64; `neon.rs` and `broadword.rs` on ARM64; `broadword.rs` never compiles on x86_64 at all), and `portable-popcount` together with `simd` compiles out the AVX-512 popcount path (`src/bits/popcount.rs`), so `cargo clippy --all-targets --all-features` lints none of them (#185, #388). CI's x86_64 `Clippy` job therefore runs twice, once with `--all-features` and once with `--features std,simd,serde,cli,regex,bench-runner,large-tests,mmap-tests`, which excludes both; the ARM64 leg runs a third time with `broadword-yaml` added, because the YAML backend selectors are cfg-exclusive (the commands are commented above the `clippy-all-variants` matrix entry in `.github/workflows/ci.yml`).
- **One source of truth for the lists:** `scripts/lib/clippy-variants.sh` (#3354), sourced by `ci.yml`, `scripts/build.sh` and `scripts/test.sh`. Change that file, not prose. For a change touching `src/yaml/simd/*.rs` or `src/bits/popcount.rs`, run every invocation `ci.yml` runs for your architecture; [docs/guides/developer.md](docs/guides/developer.md)'s Linting section lists them. Rustdoc runs for the same reason with the same lists: twice on x86_64, and on ARM64 with the core and the `broadword-yaml` list (#5740, #5908).
- **Pins gated out of the most-run build.** An exact-size or exact-behaviour pin of the *shipped* shape has to be `#[cfg(not(feature = "…"))]`-gated so `cargo test --all-features` stays green, which leaves the pin absent in the build most likely to be run to "check everything". [docs/guides/release.md](docs/guides/release.md)'s checklist runs the shipped feature set for this reason. An A/B holdout feature is one instance: [docs/guides/benchmarking.md](docs/guides/benchmarking.md#ab-benchmarking-method) says to retire the switch once its measurement is recorded (`unshared-containers`, #5841).
- **What "covers everything" means:** the CI legs on the **default** feature set (`cli,simd,regex,serde` for tests) plus the non-`--all-features` clippy invocations above, not `--all-features`. The bare `--all-features` clippy line under CI/CD below is only part of that.

The footgun is documented, not removed: that needs a `--cfg` conversion, which changes `RUSTFLAGS` and so invalidates every build cache, and needs its own measurement. A new non-additive feature needs the same write-up here as well as a line in the table above.

## Testing Strategy

- **Unit tests**: In each module's `#[cfg(test)] mod tests`
- **Property tests**: `tests/property_tests.rs`, `tests/bp_properties.rs`
- **Integration tests**: `tests/json_indexing_tests.rs`, `tests/simd_level_tests.rs`
- **New yq/jq CLI tests**: never append to the end of `tests/yq_cli_tests.rs` or
  `tests/jq_cli_tests.rs` (every PR that did conflicted with every other, #4329). A new test for
  issue `N` goes in `tests/yq_cli/iN.rs` (`tests/jq_cli/iN.rs`) and nowhere else: `mod.rs` declares
  every file in its directory with `automod::dir!`, so there is no `mod iN;` list to edit and
  two PRs never conflict on one (GitHub's merge queue ignores `merge=union`, #5697). The module
  reaches the binary's helpers through `crate::` (`use crate::run_yq_stdin;`). It is still the
  same test binary. `cargo fmt` cannot follow a macro-generated `mod`, so
  `scripts/fmt-per-issue.sh` checks those files (`--fix` formats them); an incremental build
  does not see a *new* file until `tests/yq_cli/mod.rs` is touched. See the header of
  `tests/yq_cli/mod.rs`.

## `no_std` Support

The library is `no_std` compatible:
- Uses `#![cfg_attr(not(test), no_std)]`
- Depends on `alloc` for `Vec<u64>` storage

## CI/CD

```bash
cargo clippy --all-targets --all-features -- -D warnings
cargo test
./scripts/build.sh
```

The `--all-features` clippy line above is only one of the invocations CI
actually runs — see the "`--all-features` is not a superset build" section
under Feature Flags above for the rest and why they're all needed.

## Key Documentation

| Document                                                          | Purpose                              |
|-------------------------------------------------------------------|--------------------------------------|
| [ARCHITECTURE.md](ARCHITECTURE.md)                                | Top-level architecture entry point   |
| [docs/STYLE_GUIDE.md](docs/STYLE_GUIDE.md)                        | Tagged, stable-ID coding conventions |
| [docs/adrs/README.md](docs/adrs/README.md)                        | Architecture Decision Records (why)  |
| [docs/guides/release.md](docs/guides/release.md)                  | Release process and checklist        |
| [docs/optimizations/](docs/optimizations/)                        | Optimization techniques reference    |
| [docs/benchmarks/jq.md](docs/benchmarks/jq.md)                    | JSON jq benchmark results            |
| [docs/benchmarks/yq.md](docs/benchmarks/yq.md)                    | YAML yq benchmark results            |
| [docs/benchmarks/dsv.md](docs/benchmarks/dsv.md)                  | DSV input performance benchmarks     |
| [docs/adrs/adr-0011.md](docs/adrs/adr-0011.md)                    | Why rank/select is built in-crate    |

## Performance Summary

`comprehensive` pattern, identity (`.`); the M1 Max and Neoverse figures predate the #5336 re-run (jq 1.6 / system jq):

- **jq**: 1.6-4.2x faster at 1 MB across five platforms (4.2x on an Apple M4 Pro, 2.8x on an AMD Ryzen 9 7950X) and
  about even at 10 KB, where the M4 Pro is 0.8x and every query is 0.7-0.9x at 10 KB and below. `INDEX(.id)` is
  0.64-0.76x on the 7950X at every size from 100 KB to 100 MB (#5306).
- **yq**: 1.6-24x faster in 3-54% of yq's memory. On Apple silicon the speedup grows with input size (8.9x on an M1
  Max and 8.5x on an M4 Pro at 100 MB); on the 7950X it falls from 24x at 1 KB to 11-12x at 10-100 MB. M2 streaming (`.[0]`
  on a 100 MB file, M1 Max) is 13x faster than yq in 254 MB against its 5 GB.

The per-platform tables, query mixes and measurement dates are in [docs/benchmarks/jq.md](docs/benchmarks/jq.md) and
[docs/benchmarks/yq.md](docs/benchmarks/yq.md); quote figures from there, naming the chip. Regenerate with
`succinctly bench run jq_bench`, `succinctly bench run yq_bench` (includes memory) and
`succinctly dev bench yq --queries all` (M2 streaming).

### Optimization Techniques

For detailed documentation on optimization techniques used in this project, see [docs/optimizations/](docs/optimizations/):

| Category | Document | Key Techniques |
|----------|----------|----------------|
| Bit-level | [bit-manipulation.md](docs/optimizations/bit-manipulation.md) | Popcount, CTZ, PDEP/PEXT |
| SIMD | [simd.md](docs/optimizations/simd.md) | AVX2, AVX-512, NEON |
| Memory | [cache-memory.md](docs/optimizations/cache-memory.md) | Alignment, prefetching |
| Data structures | [hierarchical-structures.md](docs/optimizations/hierarchical-structures.md) | Rank/select indices |
| Parsing | [state-machines.md](docs/optimizations/state-machines.md) | PFSM, lookup tables |

**Rules from the lessons log** (code comments cite the first two by name; the full list, with the measurements
behind each, is [docs/optimizations/lessons.md](docs/optimizations/lessons.md#key-insights)):
- Duplicated predicates diverge silently — one definition, plus a test that the call sites agree (#106: three copies of one predicate, one of them quadratic)
- Instruction counts and wall-clock can disagree in sign, and differently per architecture: #2878's SIMD string scan read +2.2% Ir on x86_64 and -8.2% on ARM64 in the perf guard, and timed -6.5% on a 7950X and +10.7% on an M4 Pro. A guard row moving is a prompt to time it, in either direction
- Micro-benchmark wins ≠ real-world improvements: P2.6, P2.8 and P3 each won a micro-benchmark and regressed end to end
- Derive, don't store, what the text already encodes; check where data physically lives before proposing to tag it (#106)
- A build profile is part of the measurement *and* of the shipped binary: `[profile.release]` pins one codegen unit plus fat LTO (#2603), and code placement still moves `Ir`-flat x86 rows by up to 15% (#3100, [docs/guides/benchmarking.md](docs/guides/benchmarking.md))

### Benchmarking Discipline

**Read [docs/guides/benchmarking.md § A/B Benchmarking Method](docs/guides/benchmarking.md#ab-benchmarking-method) before measuring a before/after change.** The naive method reported a 16x win as a regression (#106). The rules that matter most:

- **Interleave the two binaries within each repetition.** Running all of A then all of B made an improved binary measure up to **2x slower** on every workload — the second half starts thermally loaded. Not fixable with more reps; min and median can agree with each other and both be wrong.
- **Process-spawn A/B needs inputs ≥ 1 MB.** Startup is ~4-6 ms, the entire runtime at 1kb/10kb/100kb — which are in `dev bench yq`'s defaults.
- **Report the scaling curve over 2-3 sizes**, not one ratio. Growth with input size is the signature of an algorithmic fix and corroborates the claimed mechanism.
- **Gate on output identity before believing any timing**, and confirm both still match `jq`/`yq`.
- **Memory-bound effects do not port across architectures** — measure ARM *and* x86_64, and name the chip in every table. #106 was 6.1x on M4 Pro, 16.4x on Zen 4.
- **Verify the box is idle; macOS load average lies** (read 1.4 on a machine using 2.4% CPU — it counts uninterruptible-wait threads). Never benchmark a laptop on battery.
- **A benchmark cannot measure a shape it does not generate** — "all neutral" is not evidence. Add the generator pattern first.
- **A share of runtime is not a comparison.** Name a baseline commit that predates the change being judged — not a sibling from the same series (#1514: "cut the probe from 10.4% to 3.9%" shipped inside a 2-3x regression).
- **Measure a precheck where the guarded code is already fastest.** It is charged to every input including the ones it cannot help, so the workload with least work to save is where its cost shows (#1514: the same detector read +14% on record-shaped input and +141% on one wide object).
- **Attribute the cost before you A/B it.** A perf issue's named culprit is a hypothesis — #1213 and #1301 both named the wrong one. When the issue blames quantity A and you suspect B, build inputs holding A *provably constant* while varying B; if the time moves, A is not the term. #1301 blamed the resolved-path count, so holding it at 80,000 while widening the fan-out from 2 to 512 branches settled it in one table (890ms → 95ms). Unlike a profile, which only samples the window it runs in, that table has no escape hatch — and fitting its model predicts the post-fix floor before you change a line.

### Optimization Log

The accepted and rejected YAML optimizations (P2.5 to P12-A, O1 to O8) and the UTF-8 validation experiments (#134)
are in [docs/optimizations/lessons.md](docs/optimizations/lessons.md), each linking to its full write-up; the
earlier bit-level and JSON work is in [docs/optimizations/history.md](docs/optimizations/history.md). Read the
rejected entries before proposing a similar change. Each of these was rejected:

- software prefetch (P2.6), SIMD threshold tuning (P2.8) and lookup-table character classification (P3), each of
  which regressed end to end, and AVX-512 for YAML (P8) or JSON, which was slower than AVX2
- a BMI2 quote index for YAML (P6: backslash escapes make its quotes context-sensitive), and a flow-collection SIMD
  fast path (P5, before implementation: real flow collections are shorter than the 32-byte break-even)
- a Höhrmann DFA for UTF-8 validation (lost on 9 of 11 patterns: its loop-carried state retires one byte per chain;
  whole-sequence validation retires three or four)
