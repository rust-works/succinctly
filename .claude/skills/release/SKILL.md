# Release Skill

Creates releases for the succinctly crate. Triggered by terms like "release", "publish", "new version".

## Release Process

**IMPORTANT**: Do NOT run `cargo publish` manually. The CI workflow handles crates.io publishing automatically after building binaries for all platforms.

### Steps

1. **Gather ALL changes since last release**
   ```bash
   git describe --tags --abbrev=0  # Get last tag (e.g., v0.4.0)
   git log v0.4.0..HEAD --oneline --no-merges  # ALL commits, not just recent
   ```
   - Review the FULL list - there may be 50+ commits
   - Categorize by: CLI, Performance, Memory, SIMD, Compatibility, Fixes

2. **Run quality checks FIRST**
   ```bash
   ./scripts/build.sh
   cargo test --all-features
   ```

3. **Bump version in Cargo.toml**
   - Follow semver: MAJOR.MINOR.PATCH
   - MAJOR: Breaking API changes
   - MINOR: New features (backward compatible)
   - PATCH: Bug fixes (backward compatible)

4. **Update version snapshot** (ALWAYS required)
   ```bash
   grep -r "0\.4\.0" tests/snapshots/  # Find version in snapshots
   ```
   - Update `tests/snapshots/cli_golden_tests__version.snap`
   - This test WILL fail if you skip this step

5. **Update CHANGELOG.md**
   - Create new version section with date `YYYY-MM-DD`
   - Group changes by user impact (not by commit type):
     - **CLI Enhancements** - user-facing CLI features
     - **Performance** - speed/memory improvements with metrics
     - **SIMD Optimizations** - architecture-specific speedups
     - **Memory Optimizations** - space efficiency improvements
     - **Fixed** - bug fixes
   - Update comparison links at bottom of file

6. **Commit the release**
   ```bash
   git add Cargo.toml Cargo.lock CHANGELOG.md tests/snapshots/
   git commit -m "chore(release): prepare vX.Y.Z"
   ```

7. **Push via PR** (branch protection may block direct push)
   ```bash
   git checkout -b release/vX.Y.Z
   git push -u origin release/vX.Y.Z
   gh pr create --title "chore(release): prepare vX.Y.Z" --body "..."
   ```
   - If direct push to main is allowed, use that instead
   - Wait for PR to be merged before tagging

8. **Create and push tag** (after merge)
   ```bash
   git checkout main && git pull origin main
   git tag -a vX.Y.Z -m "Release vX.Y.Z"
   git push origin vX.Y.Z
   ```

7. **Wait for CI** - The release workflow will:
   - Create GitHub release
   - Build binaries for linux-x64, linux-arm64, macos-arm64, windows
   - Publish to crates.io automatically

8. **Monitor the release**
   - Check Actions tab for workflow status
   - Verify crates.io shows new version
   - Verify GitHub release has all artifacts

## What NOT to Do

- **Never run `cargo publish` manually** - CI handles this
- **Never push a tag before the commit is pushed** - tag must reference pushed commit
- **Never skip updating CHANGELOG.md** - it's part of the release
- **Never look at only recent commits** - always check ALL commits since last tag
- **Never forget version snapshots** - grep for old version across codebase
- **Never push directly to main without checking** - branch protection may require PR

## Troubleshooting

If the "Publish to crates.io" job fails with "already exists":
- This is harmless if you accidentally published manually
- The crate is published, just the CI step failed
- Don't worry about it for this release

If build fails:
1. Delete the tag: `git tag -d vX.Y.Z && git push origin :refs/tags/vX.Y.Z`
2. Fix the issue
3. Create a new release

## Reference

See [docs/guides/release.md](../../../docs/guides/release.md) for full documentation.
