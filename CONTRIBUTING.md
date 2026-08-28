# Contributing to Succinctly

Thank you for your interest in contributing to Succinctly! This document provides guidelines and information to help you get started.

## Code of Conduct

This project adheres to a [Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code.

## Getting Started

### Prerequisites

- Rust 1.70 or later (we use edition 2021)
- Git

### Development Setup

1. Clone the repository:
   ```bash
   git clone https://github.com/rust-works/succinctly.git
   cd succinctly
   ```

2. Build the project:
   ```bash
   cargo build
   ```

3. Run tests:
   ```bash
   cargo test
   ```

4. Run benchmarks:
   ```bash
   cargo bench
   ```

### Building with Features

```bash
# Build with SIMD popcount
cargo build --features simd

# Build with CLI tool
cargo build --features cli

# Run large bitvector tests (requires ~125MB RAM)
cargo test --features large-tests

# Run huge bitvector tests (requires ~625MB RAM)
cargo test --features huge-tests
```

## Making Changes

### Code Style

We use standard Rust formatting and linting. The project's coding, documentation, and
lint conventions are captured with stable rule IDs in
[docs/STYLE_GUIDE.md](docs/STYLE_GUIDE.md) — consult it (and cite the relevant
`STYLE-####` when suppressing a lint) before submitting changes.

```bash
# Format code
cargo fmt

# Run clippy
cargo clippy --all-targets --all-features -- -D warnings
```

All code must:
- Pass `cargo fmt --check`
- Pass `cargo clippy` with no warnings
- Include tests for new functionality
- Include documentation for public APIs

### Testing Requirements

Before submitting a PR, ensure:

```bash
# All tests pass
cargo test

# Tests pass with all feature combinations
cargo test --features simd
cargo test --features serde

# Clippy is clean
cargo clippy --all-targets --all-features -- -D warnings

# Documentation builds without warnings
cargo doc --no-deps
```

### Performance-Sensitive Code

For performance-critical changes:

1. Run benchmarks before and after:
   ```bash
   cargo bench --bench rank_select
   cargo bench --bench json_simd
   ```

2. Include benchmark results in your PR description

3. Consider Amdahl's Law - optimize the bottleneck, not the fast path

4. Test on multiple platforms if SIMD is involved

### Commit Messages

We use [Conventional Commits](https://www.conventionalcommits.org/) format:

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation only
- `style`: Formatting, no code change
- `refactor`: Code change that neither fixes a bug nor adds a feature
- `perf`: Performance improvement
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

**Examples:**
```
feat(json): add PFSM table-driven parser

Implements parallel finite state machine approach from hw-json-simd.
Achieves 880 MiB/s throughput on x86_64 (AMD Zen 4), 40-77% faster than scalar.
```

```
fix(bp): correct find_close for edge case at word boundary

Fixes #42
```

```
perf(popcount): add AVX-512 VPOPCNTDQ implementation

5.2x faster than scalar for large bitvectors.
Requires Intel Ice Lake+ or AMD Zen 4+.
```

## Pull Request Process

1. **Fork and branch**: Create a feature branch from `main`
   ```bash
   git checkout -b feat/my-feature
   ```

2. **Make changes**: Implement your changes with tests

3. **Test locally**: Run the full test suite
   ```bash
   cargo test
   cargo clippy --all-targets --all-features -- -D warnings
   ```

4. **Push and create PR**: Push your branch and open a pull request

5. **Describe your changes**: Include:
   - What the PR does
   - Why it's needed
   - How it was tested
   - Performance impact (if applicable)

6. **Address review feedback**: Make requested changes and push updates

7. **Merge**: Once approved and green, use **"Merge when ready"** to add the PR to
   the merge queue. The queue replays your commits onto the current `main` and
   re-runs CI on that combination before merging, so a PR that was green in
   isolation can't silently break `main` by racing another merge. You don't need
   to update your branch first — the queue does that for you. Merges are done by
   **rebase**: your commits are replayed onto `main` with no merge commit.

### Verifying CI actually ran

`gh pr checks` reporting "no checks reported" is ambiguous: it can mean
"nothing has started yet" *or* "GitHub silently failed to deliver the
trigger event for the current head" (see #531 — force-pushes and a
close/reopen on a PR stopped producing new workflow runs for ~50 minutes
while `main` and other branches ran normally). Don't assume the former.

Don't trust the last SHA that *did* run — check the PR's actual current head:

```bash
gh pr view <PR> --json headRefOid -q .headRefOid
gh api repos/rust-works/succinctly/commits/<head-sha>/check-runs
```

If `total_count` is `0` for the real head SHA while other branches/PRs are
running CI normally, treat it as a possible event-delivery gap rather than
"still queued."

The `Canary` workflow (`.github/workflows/canary.yml`) is the fastest
signal: it completes in seconds and stamps the SHA it ran against in its
Step Summary, so you can tell at a glance which head GitHub most recently
triggered a run for.

Per #531's timeline, a content-identical amend + force-push and a
close/reopen did *not* revive triggering, but a real rebase (new commit
SHAs) did — if you hit this, a rebase is a known workaround.

### Keeping a stacked PR bisectable

CI builds a PR's head commit and the merge queue's replay result — not each
commit in a multi-commit PR individually. A PR that only compiles once every
commit is applied, but not partway through the stack, merges cleanly and
passes every check, yet leaves a broken commit sitting on `main` (#1626: an
intermediate commit left two CLI call sites unadapted after a signature
change; the very next commit in the same PR fixed them as an aside, so
nothing outside that PR ever saw the gap). `git bisect` across that range
then fails to build instead of answering, at exactly the moment a stray
build error is most expensive — the natural reading is "the bug is here."

Before pushing a stacked series, check that every commit in it builds on its
own:

```bash
git rebase --exec 'cargo check --features cli' -i <base>
```

This is cheap (`cargo check` doesn't produce a binary) and catches the class
of gap #1626 found before it reaches `main`, rather than leaving it for a
future bisector to notice and `git bisect skip` past.

## Architecture Guidelines

### Memory Layout

- Prefer contiguous memory layouts for cache efficiency
- Document memory overhead in comments and docs
- Use `#[repr(C)]` when memory layout matters

### SIMD Code

- Always provide a scalar fallback
- Use runtime feature detection (`is_x86_feature_detected!`)
- Test on both x86_64 and aarch64 if possible
- Document the expected speedup

#### SIMD CI coverage

SIMD unit tests self-skip when the host CPU lacks the required feature (an
early `return`, emitting a `SKIPPED: SIMD test` marker via `note_simd_skip`).
A fully-skipped suite would otherwise read as a green pass, so each test job
**asserts its runner actually has the features** (the "Verify SIMD CPU
features" steps in `.github/workflows/ci.yml`) and fails loudly if not.

What each CI runner exercises for real:

| Runner                        | CPU           | Backends asserted in CI                |
|-------------------------------|---------------|----------------------------------------|
| `ubuntu-latest` (Test x86_64) | AMD EPYC 7763 | POPCNT, SSE4.1/4.2, **BMI2**, **AVX2** |
| `ubuntu-24.04-arm` (Test ARM) | Neoverse-N2   | NEON, **SVE2**, **SVE2-BITPERM**       |
| `macos-latest` (Test macOS)   | Apple Silicon | NEON                                   |

Each test job also pins its expected feature set hard via
`SUCCINCTLY_EXPECT_SIMD` (`tests/simd_expectation_tests.rs`): if a runner-fleet
change stops exposing a listed feature, the leg fails instead of the guarded
suites silently self-skipping (#193 x86, #194 ARM/macOS).

Kernel-direct differential tests (`tests/simd_level_tests.rs`,
`tests/dsv_simd_differential_tests.rs`, `src/yaml/simd/x86.rs`) exercise every
kernel *level* on the AVX2 runner, but they cannot catch caller-side dispatch
bugs — e.g. the classify/skip-width accounting (#231), where the parser
consumed a 16-byte SSE2 result as if 32 bytes had been scanned. For that, the
x86 leg **re-runs the whole suite with `SUCCINCTLY_SIMD=sse2`** (the
"SSE2-clamped dispatch" step, #247), forcing the yaml parser through the real
16-byte dispatch path; an in-lib contract test
(`test_succinctly_simd_env_contract`) fails the step if the clamp stops
applying. See
[docs/reference/environment-variables.md](docs/reference/environment-variables.md#succinctly_simd).

**SVE2 (#194).** The Neoverse-N2 runner detects `sve2` and `sve2-bitperm`, so
the always-on SVE2 kernels (DSV quote masking, broadword select via BDEP/BEXT)
run natively on every push. The JSON semi-index SVE2 kernels sit behind the
opt-in `SUCCINCTLY_SVE2=1` dispatch
(`docs/reference/environment-variables.md`) and are exercised by the dedicated
"JSON SVE2 dispatch" step in the ARM job. Apple Silicon (M1-M4) has no
non-streaming SVE/SVE2, so on a Mac every SVE2 suite prints a `SKIPPED` line
and asserts nothing; to validate SVE2 changes locally, run
`scripts/test-sve2-qemu.sh`, which executes the aarch64 suite under
`qemu-aarch64 -cpu max` (SVE2 + BITPERM emulated at the 128-bit vector length
of real Neoverse hardware) inside Docker.

**Not covered by routine CI: AVX-512.** The standard x86 runners usually draw
Zen 3 (EPYC 7763) CPUs, which have no `avx512f`/`avx512vpopcntdq`, so the
AVX-512 paths (`src/util/simd/x86.rs` `avx512f` branch,
`src/bits/popcount.rs` VPOPCNTDQ) self-skip on most runs. The runner fleet is
not CPU-homogeneous — an occasional host does have AVX-512 (the source of the
coverage wobble described below) — but no run is guaranteed one, so treat the
paths as uncovered. Validate changes to them off-CI — on AVX-512 hardware or
under an emulator (e.g. QEMU `qemu-x86_64 -cpu max`) — and note how you
tested in the PR.

Because the AVX-512 gate is a runtime CPU check, not a `#[cfg]`, the ~30
executable lines behind the VPOPCNTDQ dispatch in `src/bits/popcount.rs` are
still *compiled* under `--features cli,simd,regex,serde` and counted in the
coverage denominator — but only *executed* on a runner whose CPU has
`avx512vpopcntdq`. The coverage job is also `ubuntu-latest` with no guaranteed
VPOPCNTDQ (`SUCCINCTLY_EXPECT_SIMD` intentionally omits it — see the note in
[docs/reference/environment-variables.md](docs/reference/environment-variables.md#succinctly_expect_simd)),
so **`src/bits/popcount.rs` reports a non-deterministic x86 coverage number**:
~93% when the run happens to draw a VPOPCNTDQ-capable host, ~66% when it
doesn't — a ±~26pp swing driven purely by which CPU the run landed on, not by
any code change. This is expected, not a regression — and it is now **filtered
out of the PR comment** rather than merely tolerated: `src/bits/popcount.rs` is
listed in [`.omni-dev/coverage.yaml`](.omni-dev/coverage.yaml), which
`omni-dev coverage diff` (>= 0.38.0, pinned in `ci.yml`) reads straight from the
checkout and applies to *both* the baseline and head reports before diffing, so
the file can no longer generate a phantom entry. Note the exclusion is symmetric
and therefore also removes the file from the **`Total:`** the comment reports —
that number is a couple of points off the raw `cargo llvm-cov` total for this
reason. It does not weaken any gate: `fail-under-lines` is computed by
`cargo llvm-cov` itself, independently of `omni-dev`, over the unfiltered set
(and has ~19pp of headroom over the ~74.5% total regardless).

If you are reading a *pre-filter* PR comment, or `popcount.rs` moves by ±26pp on
an otherwise-unrelated PR (e.g. a dependabot bump), that's this — no need to
re-triage it. Tracked in #302 (and omni-dev#1398 for the mechanism); the
kernel's correctness is still validated on capable hardware by the two
host-gated tests in `popcount.rs`, which the exclusion does not affect — it
suppresses *reporting*, not measurement or testing.

### Unsafe Code

- Minimize `unsafe` blocks
- Add `// SAFETY:` comments explaining invariants
- Prefer safe abstractions over raw unsafe code

### `no_std` Compatibility

- Avoid `std` dependencies in core library code
- Use `#[cfg(feature = "std")]` for std-only functionality
- Test with `--no-default-features` to verify `no_std` works

## Releases

See [docs/guides/release.md](docs/guides/release.md) for the release process and checklist. Releases are handled by maintainers.

## Questions?

- Open an issue for questions about contributing
- Check existing issues and PRs for similar work
- Read the [architecture documentation](CLAUDE.md) for design context

## License

By contributing, you agree that your contributions will be licensed under the same terms as the project (MIT OR Apache-2.0).
