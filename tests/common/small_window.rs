//! The windowed routes of `succinctly jq` over small fixtures (#7376).
//!
//! The routes read a regular file, a list of files or a pipe a window at a time once it holds four
//! windows, and a window was 1 MiB (256 KiB since #7412): a CLI test of them needed a fixture of 4 MiB
//! or more, run several times, and those tests were the heaviest block of `jq_cli_tests`. A debug build of the binary reads a
//! smaller window from `SUCCINCTLY_DEBUG_JQ_WINDOW_BYTES` (`window::window_bytes` in
//! `src/bin/succinctly/jq_runner/window.rs`), with every limit that is a multiple of the window
//! scaled with it. A release build does not have the variable: there the window is a constant.
//!
//! A test asks for the small window by building its fixtures from [`scaled`] counts and its runs
//! with [`apply`]:
//!
//! - in a debug build (`cargo test`, and CI's `opt-level=1` legs, which keep `debug_assertions`)
//!   [`WINDOW`] is 64 KiB, [`scaled`] divides by 16 and [`apply`] sets the variable;
//! - in a release build (`cargo test --release`) [`WINDOW`] is the real 256 KiB, [`scaled`] divides
//!   by 4 and [`apply`] does nothing, so the same test runs the shipped route over a fixture a
//!   quarter of the one it was written with.
//!
//! What a converted test must keep true, since most of these tests compare the windowed route with
//! the one that reads the input whole and so still pass when the route is not taken:
//!
//! - assert the fixture is at least [`MIN_BYTES`] wherever the test used to assert 4 MiB;
//! - derive line numbers, ids and counts in the expectations from the scaled count, not from the
//!   literals of the 60,000-line fixture;
//! - a size that was chosen against a window (a document bigger than one, a stretch longer than
//!   four) is [`scaled`] too, so it stays on the same side of the limit;
//! - a size chosen against something that does not scale (a pipe's 64 KiB buffer, a 4096-byte
//!   read, the 32 files held open) is left as it is.
//!
//! Not every windowed test is converted: at least one per route stays on the shipped window in the
//! debug build, so the constants that ship are still run end to end by `cargo test`.

use std::process::Command;

/// The variable a debug build of `succinctly` reads its window from.
pub const ENV: &str = "SUCCINCTLY_DEBUG_JQ_WINDOW_BYTES";

/// The window the binary under test runs a converted test with.
pub const WINDOW: usize = if cfg!(debug_assertions) {
    64 << 10
} else {
    1 << 18
};

/// How many times smaller than the 1 MiB window the tests were written for [`WINDOW`] is: what
/// [`scaled`] divides by.
pub const SCALE: usize = (1 << 20) / WINDOW;

/// The fewest bytes the windowed routes take: four windows.
pub const MIN_BYTES: usize = 4 * WINDOW;

/// A count or size written for the 1 MiB window, at the window in use.
pub const fn scaled(count: usize) -> usize {
    count / SCALE
}

/// Runs `command` with [`WINDOW`]: the variable in a debug build, nothing in a release build.
pub fn apply(command: &mut Command) -> &mut Command {
    if cfg!(debug_assertions) {
        command.env(ENV, WINDOW.to_string());
    }
    command
}

/// `succinctly jq ARGS` with [`WINDOW`], `stdin_input` on an anonymous pipe when `Some`, held to
/// `fifo_feed`'s deadline: `fifo_feed::spawn_jq_deadline` for a converted test.
#[cfg(unix)]
pub fn spawn_jq(
    args: &[&str],
    stdin_input: Option<&[u8]>,
) -> anyhow::Result<(std::process::Output, i32)> {
    crate::fifo_feed::spawn_jq_deadline_with(args, stdin_input, |command| {
        apply(command);
    })
}
