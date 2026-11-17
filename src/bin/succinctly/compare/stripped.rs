//! A stripped copy of the binary under test, for the performance dimension (#7321).
//!
//! `cargo build --release` leaves the symbol table in `target/release/succinctly` (24.7 MB on
//! macOS), while `.github/workflows/release.yml` ships the output of a plain `strip` (19.6 MB).
//! A smaller image maps fewer pages and relocates less before `main`, so timing the unstripped
//! build overstates the shipped binary's cost by about 0.07 ms per start on an Apple M4 Pro:
//! 3-4% of the startup rows' `jq -n 1`. The rows time a copy stripped the way the release
//! archive is, so the published figures are the shipped binary's.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A copy of a binary, stripped as the release workflow strips it, in a directory that is
/// removed when this value drops.
#[derive(Debug)]
pub struct StrippedCopy {
    dir: PathBuf,
    exe: PathBuf,
}

impl StrippedCopy {
    /// Copy `exe` under `parent` and strip the copy with plain `strip` (the release workflow's
    /// own step, not `strip -x`). The directory is per process, so two runs that share `parent`
    /// do not replace each other's binary.
    pub fn of(exe: &Path, parent: &Path) -> Result<Self> {
        let dir = parent.join(format!("stripped-{}", std::process::id()));
        std::fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
        // Built before the copy so a failure below still removes the directory.
        let mut copy = Self {
            exe: dir.join("succinctly"),
            dir,
        };
        std::fs::copy(exe, &copy.exe)
            .with_context(|| format!("copying {} to {}", exe.display(), copy.exe.display()))?;
        run("strip", &[copy.exe.as_os_str()])?;
        // On macOS `strip` leaves the ad-hoc signature stale, and an arm64 kernel kills a binary
        // whose signature does not match. Signing again is what the linker did the first time.
        if cfg!(target_os = "macos") {
            run(
                "codesign",
                &[
                    "--force".as_ref(),
                    "-s".as_ref(),
                    "-".as_ref(),
                    copy.exe.as_os_str(),
                ],
            )?;
        }
        copy.exe = copy.exe.canonicalize()?;
        Ok(copy)
    }

    pub fn path(&self) -> &Path {
        &self.exe
    }
}

impl Drop for StrippedCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<()> {
    let out = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("running `{program}`"))?;
    if !out.status.success() {
        bail!(
            "`{program}` failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The copy is smaller than the binary it came from and still runs; the original is untouched.
    /// The test binary is the subject: it is a debug build, so it carries a symbol table.
    #[test]
    fn a_stripped_copy_is_smaller_than_the_original_and_still_runs() {
        let me = std::env::current_exe().unwrap();
        let before = std::fs::metadata(&me).unwrap().len();
        let parent =
            std::env::temp_dir().join(format!("stripped-copy-test-{}", std::process::id()));
        let dir;
        {
            let copy = StrippedCopy::of(&me, &parent).unwrap();
            dir = copy.dir.clone();
            let after = std::fs::metadata(copy.path()).unwrap().len();
            assert!(after < before, "{after} bytes stripped, {before} before");
            assert_eq!(std::fs::metadata(&me).unwrap().len(), before);
            let out = Command::new(copy.path()).arg("--list").output().unwrap();
            assert!(out.status.success(), "{out:?}");
            assert!(String::from_utf8_lossy(&out.stdout).contains("a_stripped_copy_is_smaller"));
        }
        assert!(!dir.exists(), "the copy's directory outlived it");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// A binary that cannot be copied is an error, and the directory made for the copy is removed.
    #[test]
    fn a_missing_binary_is_an_error_and_leaves_nothing() {
        let parent = std::env::temp_dir().join(format!("stripped-copy-bad-{}", std::process::id()));
        std::fs::create_dir_all(&parent).unwrap();
        let err = StrippedCopy::of(&parent.join("absent"), &parent).unwrap_err();
        assert!(format!("{err:#}").contains("copying"), "{err:#}");
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&parent);
    }
}
