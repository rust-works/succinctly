//! Shared exit-code classification for `cargo run`-spawned CLI test helpers.
//!
//! Included by `tests/jq_cli_tests.rs`, `tests/cli_golden_tests.rs`,
//! `tests/cli_characterization_tests.rs`, `tests/deep_nesting_valid_tests.rs`
//! and `tests/json_validate_tests.rs` via `#[path = ...] mod`, per the
//! `tests/common/` convention `json_oracle.rs` established. #1516 fixed the
//! signal-death misdiagnosis
//! (`.code().unwrap_or(-1)` silently coercing a killed child to a fake exit
//! code) in `jq_cli_tests.rs` alone; its own `/code-review` found the
//! identical pattern hand-rolled in three more files (#1546). Sharing one
//! copy here, rather than four independently drifting ones, is the fix.
//!
//! `signal_death_error`/`exit_code_or_signal_death`'s actual logic now lives
//! in `src/bin/succinctly/exit_status.rs` (#1696 found the same
//! `.code().unwrap_or(-1)` pattern hand-rolled a fifth time, in production
//! orchestration code) -- included below via the same `#[path]` mechanism
//! this file is itself included with, rather than re-hand-copied, so the
//! two call sites can't drift apart again. This module's own wrappers keep
//! every existing call site here on its established `&str` signature.

#![allow(dead_code)] // Each consumer uses a different subset.

use anyhow::Result;

#[path = "../../src/bin/succinctly/exit_status.rs"]
mod exit_status;

/// The `--features` value a `cargo run`-spawned CLI subprocess should build
/// with -- always `cli` (needed for the binary itself), or just
/// `bench-runner` whenever the enclosing `cargo test` invocation also
/// requested it (`Cargo.toml`'s `bench-runner = ["cli", ...]` already pulls
/// `cli` in, so naming both would be redundant).
///
/// A hardcoded `"cli"` at each call site used to clobber a shared
/// `target/debug/succinctly` binary another test target in the same run
/// depends on (#1705): `cargo run --features cli` rebuilds and relinks that
/// exact path with a narrower feature set than the outer `cargo test
/// --features cli,...,bench-runner` invocation that compiled *this* test
/// binary, silently dropping the `bench` subcommand out from under
/// `tests/orchestrate_cli_tests.rs`'s own `env!("CARGO_BIN_EXE_succinctly")`
/// if that test target happens to run afterward in the same `cargo test`
/// process. `cfg!(feature = "bench-runner")` reads whether *this* compiled
/// test binary was built with it, which is exactly the feature set the
/// inner `cargo run` needs to match to avoid re-linking the shared binary
/// out from under a sibling test target.
pub fn cargo_run_features() -> &'static str {
    if cfg!(feature = "bench-runner") {
        "bench-runner"
    } else {
        "cli"
    }
}

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
/// process, naming the signal and the captured stderr. Historically every
/// call site below silently coerced this to `-1` via `.code().unwrap_or(-1)`,
/// which renders as an inscrutable `left: -1, right: 0` with no indication a
/// child was ever killed -- exactly what cost a wrong root-cause call in the
/// #1459 review (#1516). Thin `&str` wrapper around
/// `exit_status::signal_death_error` (see that module for the mechanism) so
/// this file's existing callers don't all need to switch to raw bytes.
pub fn signal_death_error(status: std::process::ExitStatus, stderr: &str) -> anyhow::Error {
    exit_status::signal_death_error(status, stderr.as_bytes())
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
/// definition, per that same review round. Delegates to
/// `exit_status::exit_code_or_signal_death`.
pub fn exit_code_or_signal_death(
    status: std::process::ExitStatus,
    raw_stderr: &[u8],
) -> Result<i32> {
    exit_status::exit_code_or_signal_death(status, raw_stderr)
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

/// Path to the pre-built `succinctly` binary this test binary's own build
/// already produced -- a compile-time constant, not a second cargo
/// invocation, so callers need `#![cfg(feature = "cli")]` (matching the
/// `[[bin]] required-features = ["cli"]` this depends on): the outer
/// `cargo test` invocation has to have `cli` active for the bin target,
/// and therefore this path, to exist at all.
pub fn succinctly_bin() -> &'static str {
    env!("CARGO_BIN_EXE_succinctly")
}

/// Spawns `build()` (already configured with args), optionally writing
/// `stdin_input` to it, retrying the whole spawn+wait up to
/// [`MAX_CARGO_RETRIES`] times on either of two transient failures: `spawn()`
/// itself returning `ENOENT` (a concurrent, differently-featured `cargo
/// test` invocation sharing this `target/` directory can be transiently
/// mid-relink of the shared `succinctly_bin()` path, #550), or the spawned
/// child being killed by a signal rather than exiting with a real code -- an
/// OOM kill, another session's `pkill`, or a stall-guard's process-group
/// kill (`cargo-guard.sh`, by design, on a detected stall, #935) catching
/// this specific child, all just as plausible under this repo's routine
/// heavy concurrent multi-session load as the cargo-lock-contention case
/// `classify_cargo_run_exit` retries for. Unlike that function, there is
/// no exit-101 case to also retry on here: a direct `succinctly_bin()`
/// spawn has no cargo invocation of its own to contend on a lock.
///
/// Shared by every direct-spawn CLI test helper (#1847 review) --
/// `cli_golden_tests.rs`, `cli_characterization_tests.rs`,
/// `deep_nesting_valid_tests.rs`, `json_validate_tests.rs`, and (via #1884's
/// follow-up fix, after that file's own bespoke copy was found to have
/// silently dropped signal-death retry) `jq_cli_tests.rs` each independently
/// dropped some or all of this retry surface (along with the lock-contention
/// case that genuinely no longer applies) when first converting away from
/// `cargo run`; sharing one copy here instead of five independently
/// drifting ones is the same fix `classify_cargo_run_exit`'s own doc comment
/// already describes for the exit-code-classification half of this same
/// problem. (#1884 code review, round 3: this function's own `spawn()` call
/// used a bare `?` with no `ENOENT` handling until this paragraph's fix --
/// the one piece of `spawn_jq_full`'s old protection that got lost when
/// `jq_cli_tests.rs` first adopted this shared helper, since this function
/// didn't have it either at the time.)
///
/// `build` is called fresh on each retry attempt, since a spawned
/// `std::process::Command` cannot be reused. `stdout`/`stderr` are always
/// piped. `stdin` is piped and `stdin_input` written to it when `Some`;
/// when `None`, `stdin` is explicitly set to `Stdio::null()` -- **not**
/// left unconfigured, which would default to inheriting this test
/// process's own stdin under `.spawn()` (unlike `.output()`, whose
/// documented default *is* `Stdio::null()`; an earlier version of this
/// function relied on that same default by simply never calling
/// `.stdin(...)` in the `None` case, silently inheriting instead once
/// converted to `.spawn()`/`.wait_with_output()` -- confirmed live: a
/// spawned child with only stdout/stderr piped echoes back whatever this
/// process's own stdin contains, where the old `.output()` calls it
/// replaced saw immediate EOF). Getting this wrong reintroduces exactly
/// the class of hang this whole conversion exists to eliminate, just
/// relocated to stdin instead of an orphaned grandchild.
///
/// Retries sleep `100 * (attempt + 1)` ms between attempts, matching
/// every pre-conversion call site's own backoff -- an immediate retry
/// under the same sustained load that caused the first signal death (an
/// OOM condition that hasn't cleared, a stall-guard still killing the
/// process group) is more likely to be killed again in the same narrow
/// window, burning through the retry budget faster than a short backoff
/// would.
///
/// Returns the real exit code alongside the `Output` so callers never
/// need their own `output.status.code().expect(...)` -- that invariant
/// (a real code is always present in an `Ok` result) is asserted here
/// exactly once, via [`exit_code_or_signal_death`], rather than
/// re-asserted independently at every call site.
pub fn spawn_with_signal_retry(
    mut build: impl FnMut() -> std::process::Command,
    stdin_input: Option<&[u8]>,
) -> Result<(std::process::Output, i32)> {
    for attempt in 0..MAX_CARGO_RETRIES {
        let mut command = build();
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(if stdin_input.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            });
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound && attempt + 1 < MAX_CARGO_RETRIES =>
            {
                std::thread::sleep(std::time::Duration::from_millis(100 * (attempt as u64 + 1)));
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        // Write, but don't propagate a failure yet (#1891): if the child
        // exits or closes stdin early (e.g. an argv-parse error before it
        // ever reads stdin), `write_all` can fail before the child has been
        // waited on. Returning via `?` at that point would drop `child`
        // without reaping it -- `Child`'s `Drop` does not wait() the OS
        // process, so the early return would leak a zombie for the rest of
        // this test binary's run. `wait_with_output()` below doesn't care
        // whether the write succeeded; call it unconditionally first so the
        // child is always reaped, then surface the write error if there was
        // one.
        //
        // Ordering invariant: this write runs to completion (blocking on a
        // full pipe buffer) *before* `wait_with_output()`'s own concurrent
        // stdout/stderr draining starts, which is the classic double-pipe
        // deadlock shape if the child fills its stdout/stderr buffer while
        // waiting on stdin. Currently safe only because every binary this
        // helper spawns reads stdin to completion before writing any
        // output, and the small inputs/outputs these tests use never fill
        // an OS pipe buffer either way -- not an invariant this function
        // enforces itself.
        let write_result = stdin_input.and_then(|input| {
            use std::io::Write;
            child.stdin.take().map(|mut sin| sin.write_all(input))
        });
        let output = child.wait_with_output()?;
        write_result.transpose()?;
        if let Some(code) = output.status.code() {
            return Ok((output, code));
        }
        if attempt + 1 >= MAX_CARGO_RETRIES {
            // #1691: raw bytes, not a `String::from_utf8` decode of
            // `output.stderr` first -- a signal-killed child can leave a
            // truncated multi-byte UTF-8 sequence at the end of a buffer
            // it was writing when killed.
            exit_code_or_signal_death(output.status, &output.stderr)?;
            unreachable!("exit_code_or_signal_death errors whenever status.code() is None");
        }
        std::thread::sleep(std::time::Duration::from_millis(100 * (attempt as u64 + 1)));
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::spawn_with_signal_retry;

    /// #1891: a `write_all` failure used to `?` out of
    /// `spawn_with_signal_retry` before the child was ever waited on,
    /// leaking a zombie process for the rest of this test binary's run.
    /// Reproduces the failure condition -- a large stdin payload to a
    /// child that closes its own stdin (and so its own read end of the
    /// pipe) almost immediately, reliably forcing `write_all` to observe
    /// a broken pipe (a small write can silently land in the kernel pipe
    /// buffer even after the reader has gone away, so this needs to be
    /// large enough to exceed that buffer) -- and verifies via a
    /// process-table check that the *specific* child process is not left
    /// as a zombie either way.
    ///
    /// Checks one targeted PID, not a system-wide zombie count: this
    /// crate's own test suite runs many test binaries and threads
    /// concurrently, each spawning its own short-lived subprocesses, so a
    /// broad "any zombie owned by this process" scan sees transient noise
    /// from unrelated sibling tests under load and is not reliable here
    /// (this shape of flake was caught live during this issue's own
    /// verification). The child writes its own PID to a temp file as its
    /// first action -- independent of whatever happens to its stdin --
    /// so the PID is known even on the failure path, where
    /// `spawn_with_signal_retry`'s own `Output` (whose stdout would
    /// otherwise carry it) is never returned.
    ///
    /// Skips the assertion (not the exercise of the code path itself) if
    /// `ps` isn't available in this environment, rather than failing on
    /// an environment limitation.
    #[test]
    #[cfg(unix)]
    fn reaps_child_on_stdin_write_failure_1891() {
        let pid_file = tempfile::NamedTempFile::new().expect("create temp file");
        let pid_path = pid_file.path().to_path_buf();

        let big_input = vec![0u8; 8 * 1024 * 1024];
        // The write failing (or not -- the OS's exact timing isn't
        // guaranteed) is the path under test here, not a test failure in
        // itself; what matters is that this doesn't panic and doesn't
        // leave a zombie either way.
        let _ = spawn_with_signal_retry(
            || {
                let mut cmd = std::process::Command::new("sh");
                cmd.args(["-c", &format!("echo $$ > {}", pid_path.display())]);
                cmd
            },
            Some(&big_input),
        );

        // The child writing its PID file and exiting isn't necessarily
        // synchronous with this function returning; give it a moment.
        std::thread::sleep(std::time::Duration::from_millis(200));
        let Ok(pid_text) = std::fs::read_to_string(&pid_path) else {
            eprintln!("skipping zombie assertion: child never wrote its PID file");
            return;
        };
        let Ok(pid) = pid_text.trim().parse::<u32>() else {
            eprintln!("skipping zombie assertion: unparseable PID {pid_text:?}");
            return;
        };

        let Ok(output) = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
        else {
            eprintln!("skipping zombie assertion: `ps` unavailable in this environment");
            return;
        };
        let stat = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stat.contains('Z'),
            "child pid {pid} is a zombie (stat: {stat:?}) -- not reaped"
        );
    }
}
