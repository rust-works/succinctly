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

7. **Merge**: Once approved, your PR will be merged

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
early `return`, sometimes emitting a `SKIPPED: SIMD test` marker via
`note_simd_skip`). A fully-skipped suite would otherwise read as a green pass,
so each test job **asserts its runner actually has the features** (the
"Verify SIMD CPU features" steps in `.github/workflows/ci.yml`) and fails
loudly if not.

What each CI runner exercises for real:

| Runner                        | CPU           | Backends asserted in CI                |
|-------------------------------|---------------|----------------------------------------|
| `ubuntu-latest` (Test x86_64) | AMD EPYC 7763 | POPCNT, SSE4.1/4.2, **BMI2**, **AVX2** |
| `ubuntu-24.04-arm` (Test ARM) | Neoverse-N2   | NEON, **SVE2-BITPERM** (BDEP/BEXT)     |
| `macos-latest` (Test macOS)   | Apple Silicon | NEON                                   |

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

**Not covered by routine CI: AVX-512.** The standard x86 runners are Zen 3
(EPYC 7763), which has no `avx512f`/`avx512vpopcntdq`, so the AVX-512 paths
(`src/util/simd/x86.rs` `avx512f` branch, `src/bits/popcount.rs` VPOPCNTDQ)
self-skip there. Validate changes to those paths off-CI — on AVX-512 hardware
or under an emulator (e.g. QEMU `qemu-x86_64 -cpu max`) — and note how you
tested in the PR. (SVE2 *is* covered: the Neoverse-N2 ARM runner provides
`sve2-bitperm`.)

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
