//! #7741: a terminal on standard input returns end of file once per `^D` and then reads on, and jq,
//! whose stdio end-of-file indicator is sticky, never reads it again after a read that returned no
//! bytes. `--stream` and `--stream-errors` read standard input's first 3 bytes, then chained the rest
//! of standard input after them, which read it again when those 3 bytes had already met its end: an
//! empty input, a blank line or `1` and Enter needed a second `^D`, and `1` with no newline a third.
//! `Stdin`'s end of file is now sticky for the run.
//!
//! Each row is the number of `^D` jq 1.7.1 needs (Linux and macOS; jq 1.8.2 agrees): one to end the
//! stream, and one more first when the last line has no newline, which the first `^D` hands over. The
//! test types the input and all but the last `^D`, checks the run is still waiting, then types the last
//! and waits for the run within the deadline: a run that wants one more `^D` fails there instead of
//! hanging.

#![cfg(unix)]

// Each per-issue file includes the helper itself, so no shared `mod` list is edited (#5697).
#[allow(clippy::duplicate_mod)]
#[path = "../common/pty.rs"]
mod pty;

use std::process::Command;
use std::time::Duration;

use crate::cargo_run_exit::succinctly_bin;

/// The run of `args` (and `-` when `dash`) with a terminal on standard input, fed `input` and then
/// `eofs` `^D`, which must be what it takes to end it.
fn after_eofs(args: &[&str], dash: bool, input: &[u8], eofs: usize) -> (String, String, i32) {
    let terminal = pty::Pty::open();
    let mut command = Command::new(succinctly_bin());
    command.arg("jq").args(args).stdin(terminal.stdin());
    if dash {
        command.arg("-");
    }
    let what = format!(
        "jq {}{} with {:?} and {eofs} ^D typed",
        args.join(" "),
        if dash { " -" } else { "" },
        String::from_utf8_lossy(input)
    );
    let mut running = pty::spawn(&mut command);
    terminal.type_bytes(input);
    for _ in 1..eofs {
        terminal.eof();
    }
    assert!(
        !running.ended_within(Duration::from_millis(400)),
        "{what}: ended before its last ^D"
    );
    terminal.eof();
    running.wait(&what).triple()
}

#[test]
fn stream_reads_a_terminal_on_stdin_to_its_first_end_of_file_7741() {
    for args in [
        &["--stream", "-c", "."][..],
        &["--stream-errors", "-c", "."][..],
    ] {
        for dash in [false, true] {
            for (input, eofs, want) in [
                (&b""[..], 1, ""),
                (&b"\n"[..], 1, ""),
                (&b"1\n"[..], 1, "[[],1]\n"),
                (&b"1"[..], 2, "[[],1]\n"),
                (&b"\"x\"\n3"[..], 2, "[[],\"x\"]\n[[],3]\n"),
                (&b"{\"a\":1}\n"[..], 1, "[[\"a\"],1]\n[[\"a\"]]\n"),
            ] {
                assert_eq!(
                    after_eofs(args, dash, input, eofs),
                    (want.to_string(), String::new(), 0),
                    "jq {} (dash: {dash}) with {:?}",
                    args.join(" "),
                    String::from_utf8_lossy(input)
                );
            }
        }
    }
}

/// The routes that read standard input whole took jq's count already; pinned so that none reads on after
/// the end of file it has seen.
#[test]
fn every_route_reads_a_terminal_on_stdin_to_its_first_end_of_file_7741() {
    for (args, rows) in [
        (
            &["-c", "."][..],
            [
                ("", 1, ""),
                ("\n", 1, ""),
                ("1\n", 1, "1\n"),
                ("1", 2, "1\n"),
            ],
        ),
        (
            &["-R", "-c", "."][..],
            [
                ("", 1, ""),
                ("\n", 1, "\"\"\n"),
                ("1\n", 1, "\"1\"\n"),
                ("1", 2, "\"1\"\n"),
            ],
        ),
        (
            &["-s", "-c", "."][..],
            [
                ("", 1, "[]\n"),
                ("\n", 1, "[]\n"),
                ("1\n", 1, "[1]\n"),
                ("1", 2, "[1]\n"),
            ],
        ),
    ] {
        for (input, eofs, want) in rows {
            assert_eq!(
                after_eofs(args, false, input.as_bytes(), eofs),
                (want.to_string(), String::new(), 0),
                "jq {} with {input:?}",
                args.join(" ")
            );
        }
    }
}
