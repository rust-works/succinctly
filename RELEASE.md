# Release Checklist

This document describes the release process for succinctly.

## Overview

Releases are automated via GitHub Actions. When you push a tag matching `v*`, the release workflow:

1. Creates a GitHub release
2. Builds binaries for 5 platforms (linux-x64, linux-arm64, macos-arm64, macos-x64, windows)
3. Uploads archives to the GitHub release
4. Publishes the crate to crates.io

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

# Run the complete test suite
cargo test --all-features
```

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

Move items from `[Unreleased]` to a new version section:

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

| Platform | Archive | Contents |
|----------|---------|----------|
| Linux x64 | `succinctly-linux-x64.tar.gz` | `succinctly`, `LICENSE`, `README.md` |
| Linux ARM64 | `succinctly-linux-arm64.tar.gz` | `succinctly`, `LICENSE`, `README.md` |
| macOS ARM64 | `succinctly-macos-arm64.tar.gz` | `succinctly`, `LICENSE`, `README.md` |
| macOS x64 | `succinctly-macos-x64.tar.gz` | `succinctly`, `LICENSE`, `README.md` |
| Windows x64 | `succinctly-windows-x64.zip` | `succinctly.exe`, `LICENSE`, `README.md` |

All Unix binaries are stripped for smaller size.

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
2. Document the fix in CHANGELOG.md
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
rust-version = "1.73.0"
```

When bumping MSRV:
- This is a **minor** version bump (new feature: support for newer Rust features)
- Document the change in CHANGELOG.md under "Changed"
- Update CI workflows if they test older versions
