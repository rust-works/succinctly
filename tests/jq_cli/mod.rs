//! Per-issue jq CLI tests (#4329), compiled into the `jq_cli_tests` binary.
//!
//! `tests/jq_cli_tests.rs` is 160k+ lines and every new test used to be appended at its end,
//! so any two PRs in flight conflicted there. **Put a new test for issue `N` in
//! `tests/jq_cli/iN.rs` and nothing else: no `mod` line, no list to edit.**
//! `automod::dir!` below declares every `*.rs` file in this directory (except `mod.rs`) as a
//! module, so two PRs for different issues touch different files only. They used to add a
//! `mod iN;` line each to this file, which conflicted in GitHub's merge-queue merge: it ignores
//! `.gitattributes`, so `merge=union` did not help there (#5697).
//!
//! - A module reaches the binary's shared helpers through `crate::`
//!   (`use crate::{run_jq_full, run_jq_stdin};`); they stay private to the binary, and every
//!   descendant module can see them.
//! - This directory is not a test target (Cargo discovers `tests/*.rs` and
//!   `tests/*/main.rs`, not `tests/*/mod.rs`), so it adds no binary and no link time.
//! - Existing tests stay where they are; do not move them (a mass move conflicts with every
//!   open PR). Extend an existing test in place when it already covers the same behaviour.
//! - A test that belongs with a topical group already in `jq_cli_tests.rs` (a `mod ... {}`
//!   block) may still go there, next to its siblings, but never at the end of the file.
//! - `scripts/deep-recursion-tests.sh` matches its tests by bare name with `--exact`, so a
//!   test it lists must stay at the root of `jq_cli_tests.rs`: inside this module its name
//!   would be `per_issue::iN::name` and neither the skip nor the run would find it.
//! - `cargo fmt` cannot follow a macro-generated `mod`, so it does not reach these files:
//!   `scripts/fmt-per-issue.sh` checks them (CI's `Format` job and `scripts/pre-push.sh` run it).
//! - Any other `*.rs` file dropped in this directory becomes a module (and a compiled one), so
//!   keep shared helpers out of it; a subdirectory or a non-`.rs` file is ignored.
//! - A file added to this directory is picked up when this file recompiles. A fresh build and CI
//!   always do; if an incremental build misses a new file, `touch` this file.

automod::dir!("tests/jq_cli");
