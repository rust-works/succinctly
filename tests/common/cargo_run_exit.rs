//! Shared exit-code classification for `cargo run`-spawned CLI test helpers.
//!
//! Included by `tests/jq_cli_tests.rs`, `tests/cli_golden_tests.rs`,
//! `tests/cli_characterization_tests.rs` and `tests/deep_nesting_valid_tests.rs`
//! via `#[path = ...] mod`, per the `tests/common/` convention `json_oracle.rs`
//! established. #1516 fixed the signal-death misdiagnosis
//! (`.code().unwrap_or(-1)` silently coercing a killed child to a fake exit
//! code) in `jq_cli_tests.rs` alone; its own `/code-review` found the
//! identical pattern hand-rolled in three more files (#1546). Sharing one
//! copy here, rather than four independently drifting ones, is the fix.

#![allow(dead_code)] // Each consumer uses a different subset.

use anyhow::Result;

/// Maximum retries for a `cargo run` command that fails with exit code 101,
/// or whose child is killed by a signal (`ExitStatus::code()` returns `None`
/// only in that case, on Unix). Both are treated as transient: `101` often
/// means cargo lock contention between concurrently running tests, and a
/// signal death under the same heavy concurrent load (an OOM kill, another
/// session's `pkill`, or fallout from that same lock contention) is just as
/// likely to be environmental as a real bug in the code under test (#1516).
/// Shared here, not redeclared per file (#1546 code review): a retry budget
/// tuned in one file and not the others would recreate the exact drift this
/// module exists to prevent, just for a constant instead of a function.
pub const MAX_CARGO_RETRIES: u32 = 3;

/// Builds the "child was killed by signal N" error for a signal-terminated
/// process (`ExitStatus::code()` returns `None` only in that case, on Unix),
/// naming the signal and the captured stderr. Historically every call site
/// below silently coerced this to `-1` via `.code().unwrap_or(-1)`, which
/// renders as an inscrutable `left: -1, right: 0` with no indication a child
/// was ever killed -- exactly what cost a wrong root-cause call in the #1459
/// review (#1516).
pub fn signal_death_error(status: std::process::ExitStatus, stderr: &str) -> anyhow::Error {
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
        "child was killed by signal {}; stderr:\n{stderr}",
        signal.map_or_else(|| "<unknown>".to_string(), |s| s.to_string()),
    )
}

/// Extracts a real exit code from `status`, or builds [`signal_death_error`]
/// from the *raw* stderr bytes -- checked before any caller attempts a
/// strict `String::from_utf8` decode of the child's stdout/stderr (#1691
/// code review). A signal-killed child can leave a truncated multi-byte
/// UTF-8 sequence at the end of a buffer it was writing when killed; if the
/// caller decodes first, that decode's own `?` fires before this check ever
/// runs, surfacing a generic `FromUtf8Error` instead of naming the signal --
/// exactly the diagnostic loss this module exists to prevent. Taking `&[u8]`
/// rather than `&str` means the error path never needs its own successful
/// decode: `String::from_utf8_lossy` cannot fail.
///
/// Also collapses the ~29 hand-written `let Some(code) = status.code() else
/// { ... }` copies #1691's own fix introduced across 7 files into one
/// definition, per that same review round.
pub fn exit_code_or_signal_death(
    status: std::process::ExitStatus,
    raw_stderr: &[u8],
) -> Result<i32> {
    match status.code() {
        Some(code) => Ok(code),
        None => Err(signal_death_error(
            status,
            &String::from_utf8_lossy(raw_stderr),
        )),
    }
}

/// Classifies a `cargo run` child's exit for a retry loop, so each call site
/// doesn't hand-roll its own "retry on 101" check.
///
/// Returns `Ok(Some(code))` once a real exit code is available to the
/// caller, or `Ok(None)` when the caller should sleep (per `attempt`) and
/// retry -- covering both the `101` lock-contention case and a signal death
/// within `max_retries` attempts (a signal death under the same heavy
/// concurrent load -- an OOM kill, another session's `pkill`, or fallout
/// from that same lock contention -- is just as plausible as `101` is).
/// Once retries are exhausted on a signal death, returns
/// [`signal_death_error`] instead of `Ok(Some(-1))`.
pub fn classify_cargo_run_exit(
    status: std::process::ExitStatus,
    stderr: &str,
    attempt: u32,
    max_retries: u32,
) -> Result<Option<i32>> {
    if let Some(code) = status.code() {
        if code == 101 && attempt + 1 < max_retries {
            return Ok(None);
        }
        return Ok(Some(code));
    }
    if attempt + 1 < max_retries {
        return Ok(None);
    }
    Err(signal_death_error(status, stderr))
}
