//! #5634: a cap on the address space (`ulimit -v`) below the evaluation thread's stack made every
//! run die with `failed to spawn the evaluation thread`, whatever its input, where jq reads the
//! same document in the same cap. The stack is reserved address space, so the CLI now settles for
//! the largest half of it the process can still map. Linux only: macOS refuses `ulimit -v`.

#![cfg(target_os = "linux")]

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;

/// Run `succinctly jq` under `ulimit -v <kib>`, returning `(stdout, stderr, exit status)`.
fn run_under_cap(kib: usize, args: &[&str], input: &str) -> Result<(String, String, i32)> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(format!("ulimit -v {kib} && exec \"$0\" jq \"$@\""))
        .arg(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(input.as_bytes())?;
    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

/// A cap the full stack cannot be mapped in: a debug build reserves 2 GiB, a release build 256 MiB.
const fn capped_kib() -> usize {
    if cfg!(debug_assertions) {
        1024 * 1024
    } else {
        128 * 1024
    }
}

#[test]
fn test_a_capped_address_space_still_reads_a_document_5634() -> Result<()> {
    let numbers: Vec<String> = (0..1000).map(|n| n.to_string()).collect();
    let input = format!("[{}]", numbers.join(","));
    let (stdout, stderr, status) = run_under_cap(capped_kib(), &["-c", "length"], &input)?;
    assert_eq!((stdout.as_str(), status), ("1000\n", 0), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_a_capped_address_space_still_refuses_deep_recursion_5634() -> Result<()> {
    // The registered floor follows the stack the thread really got, so a runaway `def` is refused
    // instead of overflowing the smaller stack.
    let (_, stderr, status) = run_under_cap(capped_kib(), &["-n", "def f: [f]; f"], "")?;
    assert_eq!(status, 5, "stderr: {stderr}");
    assert!(
        stderr.contains("exceeded maximum recursion depth"),
        "stderr: {stderr}"
    );
    Ok(())
}
