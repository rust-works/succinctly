//! #7444: a `--slurp` fault is worded as jq's parser words it (#5576) also when the file list
//! holds `/dev/null` or a regular file behind `/dev/fd/N`. The re-read that finds jq's first fault
//! gave up on both: `/dev/null` is a character device, and opening `/dev/fd/N` can hand back the
//! run's own descriptor with its offset at the end (macOS). `/dev/null` now reads again as empty,
//! and a regular file is read again from its start by position. Every stdout, stderr and exit
//! status below was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 prints the same, except
//! that it parses the 257-deep document (jqlang/jq#3328 raised the depth limit, a 1.8 limit
//! change 1.7.1 stays the reference for). Standard input and a pipe still cannot be read again:
//! that wording is a recorded divergence ("A `--slurp` stream is read again from its files").

#![cfg(unix)]

use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

/// Runs the shell command `script` in a directory holding `t.json` (`{"a":[}`), `u.json`
/// (`{"a":[1}`), `one.json` (`1` and a newline) and `deep.json` (257 nested arrays), with `$J`
/// set to `succinctly jq`.
fn run(script: &str) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    for (name, text) in [
        ("t.json", "{\"a\":[}".to_string()),
        ("u.json", "{\"a\":[1}".to_string()),
        ("one.json", "1\n".to_string()),
        (
            "deep.json",
            format!("{}{}\n", "[".repeat(257), "]".repeat(257)),
        ),
    ] {
        std::fs::write(dir.path().join(name), text)?;
    }
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new("sh");
            command
                .current_dir(dir.path())
                .env("LC_ALL", "C")
                .args(["-c", &format!("J=\"$0 jq\"; {script}")])
                .arg(succinctly_bin());
            command
        },
        None,
    )?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        code,
    ))
}

#[test]
fn test_slurp_fault_after_dev_null_or_dev_fd_is_jqs_parse_error_7444() -> Result<()> {
    let unmatched_1 = "jq: parse error: Unmatched '}' at line 1, column 7\n";
    let unmatched_2 = "jq: parse error: Unmatched '}' at line 2, column 7\n";
    let cases: &[(&str, &str, &str, i32)] = &[
        ("$J -sc . /dev/null t.json", "", unmatched_1, 5),
        ("$J -sc . one.json /dev/null t.json", "", unmatched_2, 5),
        (
            "$J -sc . one.json /dev/null /dev/null t.json",
            "",
            unmatched_2,
            5,
        ),
        (
            "$J -sc '.[1]' one.json /dev/null t.json",
            "",
            unmatched_2,
            5,
        ),
        (
            "$J -sc . /dev/null u.json",
            "",
            "jq: parse error: Objects must consist of key:value pairs at line 1, column 8\n",
            5,
        ),
        (
            "$J -sc . /dev/null deep.json",
            "",
            "jq: parse error: Exceeds depth limit for parsing at line 1, column 257\n",
            5,
        ),
        ("$J -sc . /dev/fd/3 3<t.json", "", unmatched_1, 5),
        (
            "$J -sc . one.json /dev/fd/3 3<u.json",
            "",
            "jq: parse error: Objects must consist of key:value pairs at line 2, column 8\n",
            5,
        ),
        ("$J -sc . /dev/stdin < t.json", "", unmatched_1, 5),
        // Unchanged: a well-formed list, and the list without `/dev/null`.
        ("$J -sc . /dev/null one.json /dev/null", "[1]\n", "", 0),
        ("$J -sc . t.json /dev/null", "", unmatched_1, 5),
    ];
    for &(script, stdout, stderr, code) in cases {
        assert_eq!(
            run(script)?,
            (stdout.to_string(), stderr.to_string(), code),
            "{script}"
        );
    }
    Ok(())
}
