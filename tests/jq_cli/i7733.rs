//! #7733: at the user's process or thread limit (`ulimit -u`, `prlimit --nproc`, a container's pids
//! limit) every run died with `failed to spawn the evaluation thread ... (os error 11)`, exit 1, even
//! `-n 1`, where jq 1.7.1, which is single-threaded, runs. The CLI now runs on the main thread when
//! no evaluation thread can be created, with the native-stack floor registered at the stack that
//! thread has (ADR-0025), so deep recursion is still refused instead of overflowing.
//!
//! Linux only: macOS does not count threads against `RLIMIT_NPROC` (both tools answer there), and a
//! limit of 1 is below the thread count of any user, so the evaluation thread cannot be created.
//! Run as root the limit is not enforced and these rows take the ordinary route; they answer the
//! same either way.

#![cfg(target_os = "linux")]

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;

/// Run `succinctly jq <args>` under `ulimit -u 1`, returning `(stdout, stderr, exit status)`.
fn run_at_thread_limit(args: &[&str], input: &str) -> Result<(String, String, i32)> {
    let mut child = Command::new("bash")
        .arg("-c")
        .arg("ulimit -u 1 && exec \"$0\" jq \"$@\"")
        .arg(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("piped");
    let input = input.to_string();
    // A run that never reads its input may close it first: the write's failure is not the test's.
    let feeder = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let output = child.wait_with_output()?;
    feeder.join().expect("feeder");
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

/// jq 1.7.1 under `prlimit --nproc=1` on Linux x86_64: `-n 1` prints `1`, `[range(10)]|add` prints
/// `45`, exit 0; `succinctly jq` failed to spawn the evaluation thread, exit 1, on both.
#[test]
fn a_run_at_the_thread_limit_answers_7733() -> Result<()> {
    for (args, stdout) in [
        (&["-n", "1"][..], "1\n"),
        (&["-n", "-c", "[range(10)]|add"][..], "45\n"),
        (&["-c", ".a"][..], "1\n"),
    ] {
        let (out, err, status) = run_at_thread_limit(args, "{\"a\":1}")?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, "", 0),
            "{args:?}"
        );
    }
    Ok(())
}

/// A large input under the limit: 200,000 numbers on standard input, read whole.
#[test]
fn a_large_input_at_the_thread_limit_answers_7733() -> Result<()> {
    let numbers: Vec<String> = (0..200_000).map(|n| n.to_string()).collect();
    let input = format!("[{}]", numbers.join(","));
    let (out, err, status) = run_at_thread_limit(&["-c", "[length, add]"], &input)?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("[200000,19999900000]\n", "", 0)
    );
    Ok(())
}

/// The floor follows the main thread's stack: a shallow recursion answers, and a runaway one is
/// refused with jq's depth error instead of overflowing the smaller stack.
#[test]
fn a_run_at_the_thread_limit_still_refuses_deep_recursion_7733() -> Result<()> {
    let (out, err, status) = run_at_thread_limit(
        &[
            "-n",
            "def f(n): if n == 0 then 0 else f(n - 1) + 1 end; f(20)",
        ],
        "",
    )?;
    assert_eq!((out.as_str(), err.as_str(), status), ("20\n", "", 0));
    for program in ["def f: [f]; f", "def f(n): f(n + 1) + 1; f(0)"] {
        let (_, stderr, status) = run_at_thread_limit(&["-n", program], "")?;
        assert_eq!(status, 5, "{program}: stderr: {stderr}");
        assert!(
            stderr.contains("exceeded maximum recursion depth"),
            "{program}: stderr: {stderr}"
        );
    }
    Ok(())
}
