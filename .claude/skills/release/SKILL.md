# Release Skill

Creates releases for the succinctly crate. Triggered by terms like "release", "publish", "new version".

## Release Process

**IMPORTANT**: Do NOT run `cargo publish` manually. The CI workflow handles crates.io publishing automatically after building binaries for all platforms.

### Steps

1. **Update CHANGELOG.md**
   - Move `[Unreleased]` items to new version section
   - Add date in format `YYYY-MM-DD`
   - Update comparison links at bottom of file

2. **Bump version in Cargo.toml**
   - Follow semver: MAJOR.MINOR.PATCH
   - MAJOR: Breaking API changes
   - MINOR: New features (backward compatible)
   - PATCH: Bug fixes (backward compatible)

3. **Update version snapshot** (if test fails)
   ```bash
   cargo insta accept
   ```

4. **Run tests**
   ```bash
   cargo test
   cargo clippy --all-targets --all-features -- -D warnings
   ```

5. **Commit the release**
   ```bash
   git add Cargo.toml Cargo.lock CHANGELOG.md tests/snapshots/
   git commit -m "chore(release): prepare vX.Y.Z"
   ```

6. **Create and push tag**
   ```bash
   git tag -a vX.Y.Z -m "Release vX.Y.Z - brief description"
   git push origin main
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
