//! #7740: `--stream` and `--stream-errors` over a terminal named by its path (`/dev/pts/N` on Linux,
//! `/dev/ttysNNN` on macOS, `/dev/stdin` or `/dev/fd/0` when standard input is one) lost the 3 bytes
//! `open_chunked_inputs` probed: the probe's handle was kept only for a FIFO or socket, a device was
//! reopened and seeked back, and a terminal cannot be seeked, so the reader went on after them and
//! reported a parse error the input does not hold. The handle is kept for every source that is not a
//! regular file (`read_as_stream`, #7737).
//!
//! Expected outputs are jq 1.7.1's (Linux and macOS), and jq 1.8.2 agrees: the input is typed into the
//! terminal and ended with one `^D`, which is all jq needs on each of these routes.

#![cfg(unix)]

// Each per-issue file includes the helper itself, so no shared `mod` list is edited (#5697).
#[allow(clippy::duplicate_mod)]
#[path = "../common/pty.rs"]
mod pty;

use std::process::{Command, Stdio};

use crate::cargo_run_exit::succinctly_bin;

/// Types `input` and one `^D` into a terminal that `jq --stream`'s run reads as `how`: its device path
/// (`named`), or `/dev/stdin` / `/dev/fd/0` with the terminal on standard input.
fn stream_over_terminal(args: &[&str], how: &str, input: &[u8]) -> (String, String, i32) {
    let terminal = pty::Pty::open();
    let mut command = Command::new(succinctly_bin());
    command.arg("jq").args(args);
    if how == "named" {
        command.arg(terminal.path()).stdin(Stdio::null());
    } else {
        command.arg(how).stdin(terminal.stdin());
    }
    let running = pty::spawn(&mut command);
    terminal.type_bytes(input);
    terminal.eof();
    running
        .wait(&format!("jq {} over a terminal ({how})", args.join(" ")))
        .triple()
}

/// The hunt's case: `{"a":1}` and `[2]` on two lines. succinctly said `jq: parse error: Unfinished
/// string at EOF at line 3, column 0` (exit 5), and `--stream-errors` emitted that as an error event.
#[test]
fn stream_over_a_named_terminal_keeps_its_first_bytes_7740() {
    let events = "[[\"a\"],1]\n[[\"a\"]]\n[[0],2]\n[[0]]\n";
    for args in [
        &["--stream", "-c", "."][..],
        &["--stream-errors", "-c", "."][..],
    ] {
        for how in ["named", "/dev/stdin", "/dev/fd/0"] {
            assert_eq!(
                stream_over_terminal(args, how, b"{\"a\":1}\n[2]\n"),
                (events.to_string(), String::new(), 0),
                "jq {} over {how}",
                args.join(" ")
            );
        }
    }
}

/// Inputs the probe's 3 bytes cut differently: a line shorter than them (`1`), a document spread over
/// lines, and a scalar on a line of its own after a container.
#[test]
fn stream_over_a_named_terminal_reads_every_line_7740() {
    for (input, want) in [
        (&b"1\n2\n"[..], "[[],1]\n[[],2]\n"),
        (
            &b"{\"a\":\n[true,\nnull]}\n"[..],
            "[[\"a\",0],true]\n[[\"a\",1],null]\n[[\"a\",1]]\n[[\"a\"]]\n",
        ),
        (&b"[1]\n\"xy\"\n"[..], "[[0],1]\n[[0]]\n[[],\"xy\"]\n"),
    ] {
        for how in ["named", "/dev/stdin"] {
            assert_eq!(
                stream_over_terminal(&["--stream", "-c", "."], how, input),
                (want.to_string(), String::new(), 0),
                "{:?} over {how}",
                String::from_utf8_lossy(input)
            );
        }
    }
}
