# Style Guide

Conventions for code, documentation, and other project artifacts in the succinctly
project. Each item has a unique ID for easy reference.

Much of the detailed *how* lives in the `.claude/skills/` skills and in
[CONTRIBUTING.md](../CONTRIBUTING.md); this guide is the stable-ID index that lint
suppressions, code review, and those docs can point at unambiguously.

## Tag-based lookup

Before writing or reviewing code, documentation, or other project artifacts, identify
which tags apply to the changes and search this file for those tags. Each rule has a
**Tags** line immediately after its heading.

**Search command:** `grep "Tags:.*<tag>" docs/STYLE_GUIDE.md` returns matching rule headings.

| When you are…                                     | Search for tags                              |
|---------------------------------------------------|----------------------------------------------|
| Adding or reorganizing a module / file            | `module-organization`, `simd`                |
| Adding or changing a SIMD implementation          | `simd`, `unsafe`, `code-style`               |
| Writing `unsafe` (SIMD intrinsics, raw pointers)  | `unsafe`, `simd`                             |
| Suppressing a lint (`#[allow(...)]`)              | `code-style`, `lints`                        |
| Adding platform-gated / reference / bench-only code | `code-style`, `simd`, `lints`              |
| Documenting a public item                         | `documentation`                              |
| Writing or updating tests                         | `testing`                                    |
| Updating benchmarks or benchmark docs             | `benchmarks`, `documentation`                |
| Adding or editing a markdown table                | `documentation`                              |
| Writing commit messages                           | `commits`                                    |
| Touching `no_std` boundaries or `std`-gated code  | `module-organization`                        |
| Reviewing code for style compliance               | All tags relevant to the changed code        |

---

## STYLE-0000: Style guide structure

**Tags:** `meta`

### Situation

A new convention needs to be added to this style guide.

### Guidance

Assign the next sequential ID (currently next is `STYLE-0012`) and include:

1. A **Tags** line immediately after the heading — a comma-separated list of category labels
   from the tag vocabulary below.
2. Three subheadings:
   - **Situation** — when this rule applies
   - **Guidance** — what to do (with examples where helpful)
   - **Motivation** — why this rule exists

**Tag vocabulary** (extend as needed):

| Tag                   | Covers                                                       |
|-----------------------|-------------------------------------------------------------|
| `meta`                | Style guide structure and process                           |
| `module-organization` | File layout, module tree, visibility, `no_std` boundaries   |
| `simd`                | SIMD directory layout, feature detection, platform gating   |
| `unsafe`              | Unsafe code and `// SAFETY:` documentation                  |
| `code-style`          | Imports, constants, function shape, lint hygiene            |
| `lints`               | Lint suppression policy and `Cargo.toml [lints]`            |
| `documentation`       | Doc comments, markdown, benchmark tables                    |
| `testing`             | Test structure, property tests, feature-gated tests         |
| `benchmarks`          | Benchmark running and result documentation                  |
| `commits`             | Commit message format and discipline                        |

A rule may have **multiple tags**. Items are ordered by ID. **Do not** group items under
section headings; use tags for categorisation instead.

### Motivation

Consistent structure makes the guide scannable, and stable IDs allow code review comments,
lint suppressions, and cross-references to point at specific rules unambiguously. Tags
replace section headings so items can stay in strict ID order without being shuffled between
sections when categories overlap.

---

## STYLE-0001: Module and SIMD directory layout

**Tags:** `module-organization`, `simd`

### Situation

Adding a new module, or adding a platform-specific SIMD implementation to an existing
format module.

### Guidance

Use the **named-file layout** (Rust 2018+): a module with submodules lives in a file named
after the module alongside a directory of the same name (`json.rs` + `json/`). Do **not**
introduce new `mod.rs` files for a module *root* — but note that `simd/` subdirectories in
this crate do use a `mod.rs` as their dispatch file (see below).

Each format module that has SIMD acceleration owns a `simd/` subdirectory with **one file
per instruction set**, plus a `mod.rs` that performs runtime dispatch. The instruction-set
files present depend on what the module actually uses; the JSON module is the fullest example:

```
src/json/
├── ...
└── simd/
    ├── mod.rs      # runtime dispatch + feature detection (see STYLE-0003)
    ├── x86.rs      # x86_64 SSE2 kernel + dispatch entry
    ├── avx2.rs     # AVX2 kernel
    ├── neon.rs     # ARM NEON kernel
    └── sve2.rs     # ARM SVE2 kernel
```

Other modules carry the subset they need — e.g. `src/yaml/simd/` adds `broadword.rs` (a
portable SWAR fallback) and omits `avx2.rs`; `src/dsv/simd/` includes `sse2.rs`. Shared,
format-agnostic SIMD helpers live in `src/util/simd/`.

**Every SIMD path must have a scalar (or broadword) fallback** reachable when no supported
instruction set is detected. Feature-gated fallback selection is exposed through the
`broadword-yaml` and `scalar-yaml` Cargo features (see `Cargo.toml`).

### Motivation

One file per instruction set keeps each kernel's `#[target_feature]` scoping local and makes
`git log`, editor tabs, and search unambiguous. Co-locating kernels under a `simd/` directory
with a single `mod.rs` dispatcher means the platform-selection logic lives in exactly one
place per module, and adding a new instruction set is a new file plus one dispatch arm rather
than edits scattered across the module.

---

## STYLE-0002: `unsafe` code and `// SAFETY:` comments

**Tags:** `unsafe`, `simd`

### Situation

Writing `unsafe` — almost always calling a SIMD intrinsic, or a proven-in-bounds slice
access on a hot path.

### Guidance

`unsafe` is **permitted** in this crate (unlike a pure-safe library) because SIMD intrinsics
are `unsafe fn`. Keep it disciplined:

1. **Every `unsafe` block carries a `// SAFETY:` comment** immediately above it, naming the
   invariant that makes it sound. For an intrinsic call that invariant is normally the
   feature-detection guard that gates the call:

   ```rust
   // SAFETY: has_fast_bmi2() verified BMI2 is available on this CPU.
   let mask = unsafe { bmi2::toggle64(quotes) };
   ```

   For a bounds-eliding access, cite the check that established the bound:

   ```rust
   // SAFETY: We verified bounds above.
   let word = unsafe { *self.words.get_unchecked(i) };
   ```

2. **The `unsafe` intrinsic kernel is only ever reached through the feature-detected
   dispatcher** (see STYLE-0003). Never call a `#[target_feature]` function without the
   matching `is_*_feature_detected!` guard on the path to it.

3. **Prefer a safe abstraction** where one exists at no cost — reach for `unsafe` for the
   intrinsic itself and the innermost hot loop, not for surrounding bookkeeping.

The clippy lint `undocumented_unsafe_blocks` enforces item 1; enabling it project-wide is
tracked in the `Cargo.toml [lints]` work (see STYLE-0004 and issue #205). Until then, a
missing `// SAFETY:` comment is a review defect even though it does not fail the build.

### Motivation

Intrinsics are `unsafe` because the caller, not the compiler, guarantees the CPU supports the
instruction. Writing the guarantee down at the call site as `// SAFETY:` turns an implicit
assumption into a reviewable claim, and keeps the "is this feature actually detected?" question
answerable without tracing the whole call graph.

---

## STYLE-0003: Runtime CPU feature detection

**Tags:** `simd`

### Situation

Selecting which SIMD kernel to run for the current CPU.

### Guidance

Detect features at **runtime**, not only at compile time, so a single binary runs optimally on
whatever CPU it lands on. Use the standard macros and dispatch in the module's `simd/mod.rs`:

```rust
#[cfg(target_arch = "x86_64")]
if std::is_x86_feature_detected!("avx2") {
    // SAFETY: guarded by the detection above.
    return unsafe { avx2::scan(input) };
}
```

- Guard x86_64 kernels with `is_x86_feature_detected!` and aarch64 kernels with
  `is_aarch64_feature_detected!`.
- Annotate each intrinsic kernel with the matching `#[target_feature(enable = "...")]`.
- **Cache** the detection result where dispatch is hot rather than re-querying per call.
- Feature detection needs `std`; the `no_std` build (see STYLE-0011) falls back to the portable
  path. Some paths are additionally switchable via environment variable (e.g. `SUCCINCTLY_SVE2`)
  — document any such variable where it is read.

### Motivation

Runtime detection lets one artifact ship the fastest path for each machine instead of forcing a
target-cpu build. Keeping detection in the `mod.rs` dispatcher (rather than sprinkled through
kernels) means STYLE-0002's safety argument reduces to "the guard is right above the call."

---

## STYLE-0004: Lint suppression discipline

**Tags:** `code-style`, `lints`

### Situation

Adding a `#[allow(...)]` (or considering a project-wide lint override).

### Guidance

Every lint suppression must be **traceable to a documented reason**. Two cases:

1. **Per-item suppression** — annotate the specific item and append a citation comment naming
   the STYLE rule that justifies the suppression, plus a one-line specific reason:

   ```rust
   #[allow(clippy::type_complexity)] // STYLE-0004: build-time index tuple; a named struct
                                     // would add indirection to a build-only path.
   fn build_bp_index(...) -> (u64, u64, /* ... */) { ... }
   ```

   When the suppression is an instance of a more specific rule, cite that rule instead — e.g.
   `dead_code` on conditionally-compiled code cites **STYLE-0005**, not STYLE-0004.

2. **Project-wide policy** — lints that should be allowed (or denied/warned) across the whole
   crate belong in a `Cargo.toml [lints]` table with a justification comment per entry, **not**
   in a blanket crate-level `#![allow(...)]` that hides warnings wholesale. Adding that `[lints]`
   table (with `undocumented_unsafe_blocks`, a pedantic-clippy baseline, and an explicit unsafe
   policy) is tracked in issue #205; when it lands, each allow comment there cites the STYLE rule
   that motivates it, exactly as the per-item comments do.

Do not add a bare `#[allow(...)]` with no comment, and do not silence a warning you could fix.

### Motivation

An uncommented `#[allow]` is a decision with no recorded rationale — the next reader cannot tell
whether it is load-bearing or leftover. A STYLE-ID citation turns each suppression into a link to
the policy that permits it, so a reviewer verifies "does this match the cited rule?" instead of
re-deriving the justification. Centralising project-wide decisions in `[lints]` keeps the policy
auditable in one place.

---

## STYLE-0005: Conditionally-compiled, reference, and bench-only code

**Tags:** `code-style`, `simd`, `lints`

### Situation

A `dead_code` warning fires on code that is intentionally not reachable in the current build.

### Guidance

`#[allow(dead_code)]` is **expected and correct** for these recurring cases in this crate. Cite
STYLE-0005 and say which case applies:

| Case | Why it looks dead | Comment |
|------|-------------------|---------|
| **Platform-gated** | Compiled out on the current `target_arch` / feature combo (e.g. a NEON kernel in an x86_64 build) | `// STYLE-0005: platform-gated (aarch64 NEON)` |
| **Reference / fallback impl** | A scalar or broadword implementation kept next to the SIMD one for correctness comparison or as the fallback path | `// STYLE-0005: scalar reference impl kept for correctness` |
| **Test / bench / future-use** | Helpers reachable only from tests or benchmarks, or deliberately kept for imminent future use | `// STYLE-0005: used in tests` |
| **Retained field / complete set** | A struct field kept for symmetry or a deliberately complete constant set (e.g. all jq exit codes) where only some entries are read today | `// STYLE-0005: complete jq exit-code set; not all emitted yet` |

Use a per-item `#[allow(dead_code)]` where possible. A module-level `#![allow(dead_code)]` is
acceptable only for a module that is *entirely* one of these cases (e.g. a bench-helper module) —
give it a `//!` doc line stating so.

Do **not** use `#[allow(dead_code)]` to silence genuinely unused code — delete that instead.

### Motivation

Platform-gated SIMD, paired scalar/SIMD implementations, and deliberately complete API/constant
sets mean a large amount of code is legitimately unreachable in any single build configuration, so
`dead_code` would otherwise be noise. Naming which case applies keeps "expected-dead" distinguishable
from "should be deleted," so the allow does not become a place for real dead code to hide.

---

## STYLE-0006: Doc comments

**Tags:** `documentation`

### Situation

Adding or updating documentation on a module, type, or public function.

### Guidance

- **Module-level docs** — each module file opens with a `//!` summary.
- **Item-level docs** — every public type, field, variant, and method gets `///`.
- **Summary line style** — third-person singular present indicative per
  [RFC 505](https://rust-lang.github.io/rfcs/0505-api-comment-conventions.html)
  (`/// Returns the k-th set bit.`, not `/// Return ...`), ending with a period.
- **Runnable examples** — non-trivial public functions should carry a `# Examples` block. These
  compile and run under `cargo test`, doubling as regression tests. `src/lib.rs` already carries
  such an example for `BitVec`; keep new public entry points to the same bar.

### Motivation

Third-person summaries match the standard library and `rustdoc` output. Doc examples are compiled,
so they cannot silently rot, and they document the intended call shape better than prose.

---

## STYLE-0007: Test structure

**Tags:** `testing`

### Situation

Writing a new test.

### Guidance

- **Unit tests** live in a `#[cfg(test)] mod tests` block at the **end** of the source file, with
  `use super::*;`.
- **Property tests** use `proptest` and live in `tests/` (`tests/property_tests.rs`,
  `tests/bp_properties.rs`); **integration tests** likewise live in `tests/`.
- **Large/expensive tests** are gated behind Cargo features (`large-tests`, `huge-tests`,
  `mmap-tests`) so the default `cargo test` stays fast.
- **Snapshot tests** use `insta` where output stability matters.

For deeper patterns and anti-patterns (e.g. asserting on real behaviour rather than
tautologies), see the [`testing` skill](../.claude/skills/testing/SKILL.md).

### Motivation

The `mod tests` convention gives tests access to private items and keeps them next to the code
they cover. Feature-gating the multi-gigabyte tests keeps the common test run cheap while leaving
the heavy coverage available on demand.

---

## STYLE-0008: Benchmark documentation discipline

**Tags:** `benchmarks`, `documentation`

### Situation

Adding, updating, or documenting benchmark results.

### Guidance

- **Regenerate, don't hand-edit numbers.** Produce results with the documented command
  (`succinctly bench run <bench>`, or the relevant `cargo bench`) and paste the actual output.
- **Run benchmarks sequentially, never concurrently** — benchmarks need exclusive CPU, and
  overlapping runs produce meaningless numbers.
- **Label every result table with the CPU/platform** it was measured on (the existing tables use
  headings like "Apple M1 Max", "ARM Neoverse-V2"). Numbers without a platform are not comparable.
- **Keep the summaries in sync** — the performance tables in [CLAUDE.md](../CLAUDE.md) and the
  detailed pages under [docs/benchmarks/](benchmarks/) must not diverge; update both when a number
  changes, and note the regeneration command beneath the table.

The [`benchmark-docs` skill](../.claude/skills/benchmark-docs/SKILL.md) codifies the per-platform
update procedure.

### Motivation

Benchmark numbers are only trustworthy if they are reproducible and attributed to hardware.
Sequential execution avoids the biggest source of noise; per-platform labelling stops readers
comparing an M4 result against a Neoverse one; keeping CLAUDE.md and `docs/benchmarks/` in lock-step
prevents the headline summary from drifting away from the detail.

---

## STYLE-0009: Fixed-width markdown tables

**Tags:** `documentation`

### Situation

Creating or editing a markdown table in any project doc.

### Guidance

Pad every column so cell borders line up in the raw source — the header, the separator row, and
all body rows share one column width. Prefer this:

```markdown
| Structure    | Overhead |
|--------------|----------|
| BitVec       | 3–4%     |
| BalancedParens | 6%     |
```
→ align to:
```markdown
| Structure      | Overhead |
|----------------|----------|
| BitVec         | 3–4%     |
| BalancedParens | 6%       |
```

The [`markdown-tables` / `format-md-tables` skill](../.claude/skills/markdown-tables/SKILL.md)
automates this — run it after editing a table rather than aligning by hand.

### Motivation

Fixed-width tables are readable in the raw markdown (not just when rendered), so diffs stay legible
and a mis-aligned cell is an obvious signal that a column changed. Automating the alignment keeps it
from being a manual chore.

---

## STYLE-0010: Commit messages

**Tags:** `commits`

### Situation

Writing a commit message.

### Guidance

Use [Conventional Commits](https://www.conventionalcommits.org/): `<type>(<scope>): <description>`
with an optional body and footer. Types in use: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`,
`test`, `chore`. Scope names the touched area (`json`, `bp`, `yaml`, `dsv`, `bench`, …). For
performance commits, put the measured speedup in the body. Full detail and examples are in
[CONTRIBUTING.md](../CONTRIBUTING.md#commit-messages); the
[`commit-msg` skill](../.claude/skills/commit-msg/SKILL.md) analyses a diff and drafts a message.

### Motivation

A consistent, machine-parseable commit format keeps `git log` scannable, lets tooling derive
changelogs and scopes, and makes `git bisect` and history archaeology reliable.

---

## STYLE-0011: `no_std` compatibility

**Tags:** `module-organization`

### Situation

Adding code to the core library (anything outside the CLI / `bin/`).

### Guidance

The core crate is `no_std`. Preserve that:

- The crate declares `#![cfg_attr(not(any(test, feature = "std")), no_std)]` and
  `extern crate alloc;` in `src/lib.rs` — depend on `alloc` (`Vec`, `String`, `Box`), never on
  `std`, in core modules.
- Gate any genuinely `std`-only functionality (notably runtime feature detection, threads, I/O)
  behind `#[cfg(feature = "std")]`, and provide a `no_std` fallback path.
- CLI-only code lives behind the `cli` feature (which pulls in `std`) — heavy `std` dependencies
  belong there, not in the core modules.
- Verify with `cargo build --no-default-features` (and `cargo test` uses `std`, since tests enable
  it via the `cfg_attr` above).

### Motivation

`no_std` support lets succinctly be used in embedded and WASM contexts where `std` is unavailable.
The guarantee is easy to break accidentally with a stray `std::` path, so keeping the `std` surface
behind explicit feature gates — and testing the default-features-off build — makes the boundary
enforceable rather than aspirational.
