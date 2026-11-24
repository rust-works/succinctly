# Release Checklist

[Home](../../) > [Docs](../) > [Guides](./) > Release

This document describes the release process for succinctly.

## Overview

Releases are automated via GitHub Actions. When you push a tag matching `v*`, the release workflow:

1. Packages the crate and builds it from its own files, and stops if it is too large for crates.io (see
   [The published crate](#the-published-crate))
2. Creates a GitHub release
3. Builds binaries for 4 platforms (linux-x64, linux-arm64, macos-arm64, windows)
4. Uploads archives to the GitHub release
5. Publishes the crate to crates.io

## Prerequisites

Before releasing, ensure you have:

- [ ] Write access to the repository
- [ ] `CARGO_REGISTRY_TOKEN` secret configured in GitHub (for crates.io publishing)
- [ ] All CI checks passing on `main`

## Release Process

### 1. Prepare the Release

```bash
# Ensure you're on main and up to date
git checkout main
git pull origin main

# Run the full quality check
./scripts/build.sh

# Run the complete test suite against the shipped feature set --
# NOT --all-features, which builds several non-shipping measurement
# configurations instead (portable-popcount, broadword-yaml, scalar-yaml)
# and would let a shipped-shape-only regression pass silently. See
# CLAUDE.md's Feature Flags section for why.
cargo test --features cli,simd,regex,serde

# Run benchmarks (see docs/guides/benchmarking.md for details)
cargo bench
```

**Note**: For major releases, run full benchmark suite on all platforms. See [benchmarking.md](benchmarking.md#when-to-run-benchmarks) for details.

### 2. Update Version

Update the version in `Cargo.toml`:

```toml
[package]
version = "X.Y.Z"  # Update this
```

Follow [Semantic Versioning](https://semver.org/):
- **MAJOR** (X): Breaking API changes
- **MINOR** (Y): New features, backward compatible
- **PATCH** (Z): Bug fixes, backward compatible

### 3. Update CHANGELOG.md

Changelog entries are per-PR **fragments** in `changelog.d/` (see
[changelog.d/README.md](../../changelog.d/README.md)), not edits to `CHANGELOG.md`;
`scripts/changelog.py` assembles them. Preview, then write the release section:

```bash
python3 scripts/changelog.py check                                # every fragment is well-formed
python3 scripts/changelog.py collect --version X.Y.Z --dry-run    # preview; writes nothing
python3 scripts/changelog.py collect --version X.Y.Z              # rewrite CHANGELOG.md
```

`collect` does the following, so there is nothing to move or reformat by hand:

- renders `## [X.Y.Z] - YYYY-MM-DD` with the sections in Keep a Changelog order (Added,
  Changed, Deprecated, Removed, Performance, Fixed, Security), entries by ascending issue
  number, `+slug` entries last;
- carries whatever is under `## [Unreleased]` into that section **verbatim, after the
  fragment sections**, and leaves an empty `## [Unreleased]` above it. The text that
  predates fragments is therefore released once, in the first release made after the switch;
  after that `[Unreleased]` stays empty and the flow is purely fragments. That first section
  repeats headings (a `### Changed` from the fragments, then the legacy text's own `### Changed`
  blocks): the legacy text is kept byte-for-byte on purpose and already repeats them;
- updates the comparison links at the bottom (`[Unreleased]` and the new `[X.Y.Z]`);
- deletes the consumed fragments (everything in `changelog.d/` except `README.md`).

It refuses, writing and deleting nothing, if a fragment is malformed, the version is not
`X.Y.Z`, the version already has a section, or there is nothing to release. Read the result
(`git diff CHANGELOG.md`) and group or reword entries by user impact if the release needs it.
The release PR is titled `chore(release): ...`, which waives the "PR needs a fragment"
check (it deletes fragments rather than adding one).

The result has this shape:

```markdown
## [Unreleased]

## [X.Y.Z] - YYYY-MM-DD

### Added
- New features...

### Changed
- Modified behaviors...

### Fixed
- Bug fixes...

### Deprecated
- Features to be removed...

### Removed
- Removed features...

### Security
- Security fixes...
```

Update the comparison links at the bottom:

```markdown
[Unreleased]: https://github.com/rust-works/succinctly/compare/vX.Y.Z...HEAD
[X.Y.Z]: https://github.com/rust-works/succinctly/compare/vPREVIOUS...vX.Y.Z
```

### 4. Commit the Release

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md
git add -A changelog.d    # the consumed fragments are deleted
git commit -m "chore(release): prepare vX.Y.Z"
```

### 5. Create and Push the Tag

```bash
# Create an annotated tag
git tag -a vX.Y.Z -m "Release vX.Y.Z"

# Push the commit and tag
git push origin main
git push origin vX.Y.Z
```

### 6. Monitor the Release

1. Go to [Actions](https://github.com/rust-works/succinctly/actions) and watch the Release workflow
2. Verify all build jobs complete successfully
3. Check the [Releases](https://github.com/rust-works/succinctly/releases) page for the new release
4. Verify [crates.io](https://crates.io/crates/succinctly) shows the new version

### 7. Post-Release

- [ ] Verify binary downloads work on each platform
- [ ] Test installation via `cargo install succinctly`
- [ ] Announce the release (if significant)

## Release Artifacts

The release workflow produces:

| Platform    | Archive                          | Contents                               |
|-------------|----------------------------------|----------------------------------------|
| Linux x64   | `succinctly-linux-x64.tar.gz`    | `succinctly`, `LICENSE`, `README.md`   |
| Linux ARM64 | `succinctly-linux-arm64.tar.gz`  | `succinctly`, `LICENSE`, `README.md`   |
| macOS ARM64 | `succinctly-macos-arm64.tar.gz`  | `succinctly`, `LICENSE`, `README.md`   |
| Windows x64 | `succinctly-windows-x64.zip`     | `succinctly.exe`, `LICENSE`, `README.md` |

All Unix binaries are stripped for smaller size.

The stripped Linux x64 and ARM64 binaries have a size budget, `tests/data/binary-size-budget.json` (#7390). CI's
`Perf Regression Guard` legs build this workflow's command, strip a copy and fail when it is more than 5% over its
budget (`scripts/binary_size.py`); an intended increase sets the target's `bytes` to the size the failing run printed,
in the same PR. Take the number from CI's log, not a local build: another toolchain, linker or `strip` moves it by a
percent or two. macOS has no budget, since macOS runners are not spent on the perf guard.

The macOS ARM64 archive is built for macOS 12.0 or newer, not for rustc's default of 11.0 on arm64 (#7321). The
setting is `MACOSX_DEPLOYMENT_TARGET` in `.cargo/config.toml`'s `[env]`, so the archive, a local `cargo build
--release` and the binary `succinctly dev compare jq` times get the same deployment target (their cargo features
differ: [Which binary a run measures](reproducing-the-jq-comparison.md#which-binary-a-run-measures)), and `release.yml`
needs no setting of its own. A 12.0 target gets chained fixups, which remove 22,214 rebases into 103 `__DATA_CONST` pages
from every start (about 0.07 ms of `jq -n 1` on an Apple M4 Pro); the cost is that the archive does not run on
macOS 11. `tests/jq_cli/i7321.rs` reads `minos` and the fixup load command of the built binary.
State the macOS floor in the release notes of the first release that ships this. `cargo install` builds for the
default target and keeps running on macOS 11. Any build run from inside a source checkout (a clone, a source
tarball) reads the file too and gets the 12.0 floor, on x86_64 as well as arm64; export `MACOSX_DEPLOYMENT_TARGET`
to override it.

The Linux x64 and Windows x64 archives are built with `RUSTFLAGS="-C target-feature=+popcnt"` (the `rustflags`
field of their `build-release` matrix entries, #5035), so every per-word `count_ones()` is a `POPCNT` instruction
instead of a 19-instruction broadword sequence. They need an x86-64 CPU with `POPCNT` (Intel Nehalem / AMD K10,
2008, or newer), which Windows 11 24H2 requires of its own machines. `main` checks for the instruction first and,
without it, prints a message naming the requirement and `cargo install succinctly --features cli` (a baseline
build) instead of dying on `Illegal instruction`. State the CPU floor in the release notes of the first release
that ships this. `cargo install` and the library keep the baseline target.

The Linux archives are built on `ubuntu-24.04` and `ubuntu-24.04-arm`, although the x86_64 CI jobs run on
`ubuntu-26.04` since #5926. The reason is the glibc floor: a binary links the glibc of the
image that builds it and runs only on hosts whose glibc has every symbol version it references, so building on a newer
image can lock out hosts that run today's archive. Moving `build-release` to a newer image is a release-policy
decision (name the oldest supported distribution, then check `objdump -T succinctly | grep GLIBC_` on the build), not
a CI cleanup; `scripts/test_ci_runner_pin.py` fails until that is a deliberate edit.

A change to those flags is measured the way #5035 was: the same tree built with and without them, and
`scripts/ab-cli.py` (`--control` first) over `yaml generate-suite` / `json generate-suite` inputs of 1 MB and 10 MB
([simd.md](../optimizations/simd.md#per-word-popcounts-on-baseline-x86_64-5035)).

## The published crate

The crate `cargo publish` uploads ships the library and the CLI only: `include` in `Cargo.toml` is `/src/**/*`,
`/Cargo.toml`, `/LICENSE` and `/README.md`, and cargo adds `Cargo.lock`.
`tests/`, `benches/`, `examples/` and the docs stay in the repository, so the crate's `[[bench]]` targets are dropped
from the packaged manifest (cargo prints `ignoring benchmark` for each).
The tests run from a checkout, not from the downloaded crate: several unit tests `include_str!` fixtures under
`tests/`, which the crate does not ship.
crates.io refuses a `.crate` over 10 MiB (10,485,760 bytes), and this workflow publishes last, after the GitHub
release and its archives exist.
The v0.8.0 crate with `tests/` and `benches/` still in `include` was 10,434,790 bytes, 51 KB under that limit; without
them it is about 6.2 MiB.
`scripts/crate_size.py check --verify --features cli` runs `cargo package --features cli`, which builds the packaged
library and the `succinctly` binary from the crate's own files, and fails when the `.crate` is over 8 MiB.
The binary has `required-features = ["cli"]`, so a packaged build without `cli` (such as the one `cargo publish` runs
by default) compiles only the library and would miss a bad `include_str!` under `src/bin/`.
It runs in CI's `Crate package` job on every code change, in this workflow's `package-check` job before the release is
created, and in the pre-push `crate-size` gate (`--full`).
A file under `src/` may `include_str!` a file outside `include` only inside a `#[cfg(test)]` module; anywhere else the
packaged build fails.
When the crate nears the ceiling, keep what does not need to ship out of `include` before raising `CEILING` in the
script, and never raise it to within 1 MiB of the limit.

## Troubleshooting

### Release workflow failed

1. Check the [Actions logs](https://github.com/rust-works/succinctly/actions) for the specific error
2. Common issues:
   - **Build failure**: Fix the issue, delete the tag, and re-release
   - **crates.io publish failed**: Check `CARGO_REGISTRY_TOKEN` is valid
   - **Asset upload failed**: The release may need manual cleanup

### Deleting a failed release

If a release fails partway through:

```bash
# Delete the local tag
git tag -d vX.Y.Z

# Delete the remote tag
git push origin :refs/tags/vX.Y.Z

# Delete the GitHub release via the web UI if created
```

Then fix the issue and restart from step 4.

### Version already published to crates.io

You cannot republish the same version to crates.io. If you need to fix a release:

1. Increment the patch version (e.g., `0.1.0` -> `0.1.1`)
2. Document the fix in a changelog fragment (`changelog.d/<issue>.fixed.md`)
3. Create a new release

## Pre-release Versions

For alpha, beta, or release candidate versions:

```bash
# In Cargo.toml
version = "0.2.0-alpha.1"
version = "0.2.0-beta.1"
version = "0.2.0-rc.1"

# Tag format
git tag -a v0.2.0-alpha.1 -m "Release v0.2.0-alpha.1"
```

Pre-release versions are published to crates.io but won't be installed by default with `cargo install`.

## MSRV Policy

The Minimum Supported Rust Version (MSRV) is specified in `Cargo.toml`:

```toml
rust-version = "1.89.0"
```

It is the oldest toolchain that checks the shipped CLI feature set (`--features cli,simd,regex,serde`)
with the committed `Cargo.lock`, and CI's `MSRV` job (`.github/workflows/ci.yml`) holds it there: it reads
`rust-version` from `Cargo.toml`, installs that toolchain and runs `cargo check --locked` for the library and
for that feature set on x86_64 Linux. What sets the floor:

- **1.89**: the `simd` feature's AVX-512 VPOPCNTDQ popcount kernel (`src/bits/popcount.rs`) on x86_64, whose
  intrinsics and target features stabilized in 1.89. Without `simd`, or on aarch64, 1.85 builds the CLI too.
- **1.85**: edition-2024 dependencies in `Cargo.lock` (`indexmap`, `hashbrown`, `clap`, `rand`, ...); cargo
  1.84 cannot parse their manifests. This is the floor of the library with default features.

Tests and benches need at least 1.88 for their dev-dependencies (`proptest` 1.12 declares it) and are not
checked at the MSRV.
Clippy's own `msrv` in `clippy.toml` stays at 1.73 until its MSRV-gated suggestions (`is_none_or`,
`is_multiple_of`, `repeat_n`, ...) are applied in a change of their own; it only makes clippy more
conservative.

When bumping MSRV:
- Bump `rust-version`; the `MSRV` job follows it, so nothing else in CI names the version
- This is a **minor** version bump (new feature: support for newer Rust features)
- Document the change in a `changelog.d/<issue>.changed.md` fragment
- A dependency update that raises a crate's `rust-version` above ours fails the `MSRV` job with
  `rustc <ver> is not supported by the following packages`
