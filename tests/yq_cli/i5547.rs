//! #5547: `succinctly yq ... | head -n1` ends with SIGPIPE like yq's (status 141, nothing on
//! stderr), not `failed to flush output: Broken pipe` and exit 1. Unix only.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

use anyhow::Result;

/// SIGPIPE's number on every Unix this binary builds for.
const SIGPIPE: i32 = 13;

#[test]
fn test_a_closed_stdout_pipe_ends_the_process_with_sigpipe_5547() -> Result<()> {
    // A long sequence, so the child is still writing when the reader goes away.
    let doc = format!(
        "[{}]",
        (0..300_000)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    for args in [vec![".[]"], vec!["-o=json", ".[]"], vec!["-P", ".[]"]] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("yq")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let input = doc.clone();
        let feeder = std::thread::spawn(move || {
            // The child may die before it has read everything: a broken pipe here is fine.
            let _ = stdin.write_all(input.as_bytes());
        });
        let stdout = child.stdout.take().expect("piped stdout");
        let mut first = String::new();
        BufReader::new(stdout).read_line(&mut first)?;
        let output = child.wait_with_output()?;
        feeder.join().expect("feeder thread");
        assert!(!first.trim_end().is_empty(), "{args:?}: first line");
        assert_eq!(
            output.status.signal(),
            Some(SIGPIPE),
            "{args:?}: code={:?} stderr={:?}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stderr, b"", "{args:?}");
    }
    Ok(())
}
