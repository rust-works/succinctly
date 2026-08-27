//! Shared exit-code classification for a spawned child process.
//!
//! Extracted from `tests/common/cargo_run_exit.rs` (#1691) so
//! `bench_runner::orchestrate::ssh`'s production code and the CLI
//! integration tests share one definition instead of drifting copies
//! (#1516 -> #1546 -> #1691 already had to consolidate this once for the
//! test-only copies; #1696 found the same pattern hand-rolled again in
//! production code). `tests/common/cargo_run_exit.rs` includes this file
//! via `#[path]`, the same mechanism it already uses for its own sharing
//! across `tests/*.rs`.

use anyhow::Result;

/// Builds the "child was killed by signal N" error for a signal-terminated
/// process (`ExitStatus::code()` returns `None` only in that case, on
/// Unix). Takes raw stderr bytes rather than an already-decoded `&str` so
/// callers never need a decode that can fail before reaching this point --
/// see [`exit_code_or_signal_death`].
pub fn signal_death_error(status: std::process::ExitStatus, raw_stderr: &[u8]) -> anyhow::Error {
    // `ExitStatus::code()` returns `None` only when the child was
    // terminated by a signal (Unix) -- there is no other cause.
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    anyhow::anyhow!(
        "child was killed by signal {}; stderr:\n{}",
        signal.map_or_else(|| "<unknown>".to_string(), |s| s.to_string()),
        String::from_utf8_lossy(raw_stderr),
    )
}

/// Extracts a real exit code from `status`, or builds [`signal_death_error`]
/// from the *raw* stderr bytes -- checked before any caller attempts a
/// strict `String::from_utf8` decode of the child's stdout/stderr. A
/// signal-killed child can leave a truncated multi-byte UTF-8 sequence at
/// the end of a buffer it was writing when killed; if the caller decodes
/// first, that decode's own `?` fires before this check ever runs,
/// surfacing a generic `FromUtf8Error` (or, worse, `read_to_string`
/// silently discarding everything already read) instead of naming the
/// signal. Taking `&[u8]` rather than `&str` means the error path never
/// needs its own successful decode: `String::from_utf8_lossy` cannot fail.
pub fn exit_code_or_signal_death(
    status: std::process::ExitStatus,
    raw_stderr: &[u8],
) -> Result<i32> {
    match status.code() {
        Some(code) => Ok(code),
        None => Err(signal_death_error(status, raw_stderr)),
    }
}
