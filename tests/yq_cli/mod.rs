//! Per-issue yq CLI tests (#4329), compiled into the `yq_cli_tests` binary.
//!
//! `tests/yq_cli_tests.rs` is 70k+ lines and every new test used to be appended at its end,
//! so any two PRs in flight conflicted there. **Put a new test for issue `N` in
//! `tests/yq_cli/iN.rs` and add one `mod iN;` line to the list below, in numeric order.**
//! Two PRs for different issues then touch different files, and their `mod` lines land in
//! different gaps of the sorted list. `.gitattributes` also marks this file `merge=union`,
//! so two lines added into the same gap keep both instead of conflicting.
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

mod i1071;
mod i1076;
mod i1128;
mod i1129;
mod i1325;
mod i1791;
mod i1998;
mod i2126;
mod i2186;
mod i2505;
mod i2513;
mod i2650;
mod i2679;
mod i2789;
mod i2810;
mod i2835;
mod i2839;
mod i3556;
mod i3563;
mod i3576;
mod i3600;
mod i4099;
mod i4103;
mod i4136;
mod i4183;
mod i4185;
mod i4206;
mod i4268;
mod i4386;
mod i4461;
mod i4476;
mod i4646;
mod i4851;
mod i4856;
mod i4865;
mod i4886;
mod i4899;
mod i4903;
mod i4905;
mod i4909;
mod i4916;
mod i4921;
mod i4922;
mod i4926;
mod i4939;
mod i4947;
mod i4950;
mod i4959;
mod i4977;
mod i4983;
mod i4996;
mod i4997;
mod i5001;
mod i5003;
mod i5052;
mod i5053;
mod i5053_crlf;
mod i5053_foot;
mod i5056;
mod i5058;
mod i5062;
mod i5064;
mod i5067;
mod i5067_empty;
mod i5068;
mod i5070;
mod i5073;
mod i5075;
mod i5078;
mod i5083;
mod i5086;
mod i5086_quoted;
mod i5089;
mod i5233;
mod i5245;
mod i5248;
mod i5249;
mod i5257;
mod i5258;
mod i5261;
mod i5278;
mod i5283;
mod i5286;
mod i5292;
mod i5293;
mod i5297;
mod i5299;
mod i5302;
mod i5305;
mod i5309;
mod i5356;
mod i5361;
mod i5363;
mod i5367;
mod i5374;
mod i5379;
mod i5388;
mod i5397;
mod i865;
