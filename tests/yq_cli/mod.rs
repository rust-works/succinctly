//! Per-issue yq CLI tests (#4329), compiled into the `yq_cli_tests` binary.
//!
//! `tests/yq_cli_tests.rs` is 70k+ lines and every new test used to be appended at its end,
//! so any two PRs in flight conflicted there. **Put a new test for issue `N` in
//! `tests/yq_cli/iN.rs` and nothing else: no `mod` line, no list to edit.**
//! `automod::dir!` below declares every `*.rs` file in this directory (except `mod.rs`) as a
//! module, so two PRs for different issues touch different files only. They used to add a
//! `mod iN;` line each to this file, which conflicted in GitHub's merge-queue merge: it ignores
//! `.gitattributes`, so `merge=union` did not help there (#5697).
//!
//! - A module reaches the binary's shared helpers through `crate::`
//!   (`use crate::{run_yq_stdin, run_yq_stdin_with_stderr};`, or `use crate::run_yq_stdin;` for one); they stay private to the
//!   binary, and every descendant module can see them.
//! - This directory is not a test target (Cargo discovers `tests/*.rs` and
//!   `tests/*/main.rs`, not `tests/*/mod.rs`), so it adds no binary and no link time.
//! - Existing tests stay where they are; do not move them (a mass move conflicts with every
//!   open PR; `i4136` is the one exception, moved here as this layout's first resident).
//!   Extend an existing test in place when it already covers the same behaviour.
//! - A test that belongs with a topical group already in `yq_cli_tests.rs` (a `mod ... {}`
//!   block) may still go there, next to its siblings, but never at the end of the file.
//! - `cargo fmt` cannot follow a macro-generated `mod`, so it does not reach these files:
//!   `scripts/fmt-per-issue.sh` checks them (CI's `Format` job and `scripts/pre-push.sh` run it).
//! - Any other `*.rs` file dropped in this directory becomes a module (and a compiled one), so
//!   keep shared helpers out of it; a subdirectory or a non-`.rs` file is ignored.
//! - A file added to this directory is picked up when this file recompiles. A fresh build and CI
//!   always do; if an incremental build misses a new file, `touch` this file.

automod::dir!("tests/yq_cli");
