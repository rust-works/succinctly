//! #7737: a lone input that is a character device (`/dev/urandom`, a terminal, a serial line), named on
//! the command line or on standard input, is read the way #7478 reads a pipe: the plain filter and `-R`
//! a window at a time once it holds one or two windows, the lazy `input`/`inputs` start from its front.
//! It was read to its end before the program ran, so a device that never ends grew the run until it was
//! killed, where jq answers at once. A terminal typed into ends at its `^D` before it
//! holds a window, and is read whole as before.
//!
//! Every expected output below is jq 1.7.1's (`/usr/bin/jq` on macOS, `~/.local/bin/jq` on Linux), and
//! jq 1.8.2 agrees on each. Random bytes give random parse errors, so the `/dev/urandom` rows pin the
//! shape of the answer (an exit status, a number, the start of jq's message), and that it comes at once
//! in little memory: every run here is killed and fails past [`MAX_KIB`] or the deadline instead of growing.

#![cfg(unix)]

// Each per-issue file includes the helper itself, so no shared `mod` list is edited (#5697).
#[allow(clippy::duplicate_mod)]
#[path = "../common/pty.rs"]
mod pty;

use std::process::{Command, Stdio};

use crate::cargo_run_exit::succinctly_bin;

/// What a run over an endless device may hold before it is killed: it answers holding a window or two,
/// and the route that read the device whole grew without bound.
const MAX_KIB: u64 = 512 * 1024;

fn jq(args: &[&str]) -> Command {
    let mut command = Command::new(succinctly_bin());
    command.arg("jq").args(args);
    command
}

/// `/dev/urandom` as the run's lone input three ways: named, on standard input, and as `/dev/stdin`.
fn over_urandom(args: &[&str]) -> Vec<(&'static str, pty::Ran)> {
    ["named", "stdin", "/dev/stdin"]
        .into_iter()
        .map(|route| {
            let mut command = jq(args);
            match route {
                "named" => {
                    command.arg("/dev/urandom").stdin(Stdio::null());
                }
                "stdin" => {
                    command.stdin(std::fs::File::open("/dev/urandom").expect("open /dev/urandom"));
                }
                _ => {
                    command
                        .arg("/dev/stdin")
                        .stdin(std::fs::File::open("/dev/urandom").expect("open /dev/urandom"));
                }
            }
            let what = format!("jq {} over /dev/urandom ({route})", args.join(" "));
            let ran = pty::spawn(&mut command).wait_within(&what, pty::DEADLINE, Some(MAX_KIB));
            (route, ran)
        })
        .collect()
}

/// jq 1.7.1: `-R -n 'input|length' /dev/urandom` prints the length of the first line (533, 446, 139 in
/// three runs) and exits 0, on every route. succinctly read the device whole first.
#[test]
fn raw_input_over_dev_urandom_answers_the_first_line_7737() {
    for (route, ran) in over_urandom(&["-R", "-n", "input|length"]) {
        let (stdout, stderr, code) = ran.triple();
        assert_eq!((stderr.as_str(), code), ("", 0), "{route}: {stdout:?}");
        assert!(
            stdout.ends_with('\n') && stdout.trim_end().parse::<u64>().is_ok(),
            "{route}: {stdout:?}"
        );
    }
}

/// jq 1.7.1: `-c . /dev/urandom` stops at the first byte that is not JSON, `jq: parse error: Invalid
/// numeric literal at line 1, column 6` (the column is the bytes'), exit 5.
#[test]
fn plain_filter_over_dev_urandom_stops_at_its_first_fault_7737() {
    for (route, ran) in over_urandom(&["-c", "."]) {
        let (_, stderr, code) = ran.triple();
        assert_eq!(code, 5, "{route}: {stderr:?}");
        assert!(
            stderr
                .lines()
                .last()
                .is_some_and(|line| line.starts_with("jq: parse error: ")),
            "{route}: {stderr:?}"
        );
    }
}

/// A terminal that streams lines, as a serial device or a program writing to a terminal does: each run
/// answers before the stream ends, which it never does here, as jq 1.7.1 and 1.8.2 do on their first
/// line. The terminal named on the command line (`/dev/pts/N`, `/dev/ttysNNN`) and on standard input.
#[test]
fn a_terminal_that_streams_lines_answers_before_its_end_7737() {
    let line: &[u8] = b"{\"a\":1}\n";
    for (args, want) in [
        (&["-c", "halt"][..], ""),
        (&["-c", "first(.), halt"][..], "{\"a\":1}\n"),
        (&["-R", "-c", "first(.), halt"][..], "\"{\\\"a\\\":1}\"\n"),
        (&["-R", "-n", "-c", "input"][..], "\"{\\\"a\\\":1}\"\n"),
        (&["-n", "-c", "first(inputs)"][..], "{\"a\":1}\n"),
        (
            &["-n", "-c", "[limit(2; inputs)]"][..],
            "[{\"a\":1},{\"a\":1}]\n",
        ),
    ] {
        for named in [true, false] {
            // `-n` with an unnamed terminal on standard input is #7742's.
            if !named && args.contains(&"-n") {
                continue;
            }
            let terminal = pty::Pty::open();
            let _streaming = terminal.stream(line);
            let mut command = jq(args);
            if named {
                command.arg(terminal.path()).stdin(Stdio::null());
            } else {
                command.stdin(terminal.stdin());
            }
            let what = format!(
                "jq {} over a streaming terminal (named: {named})",
                args.join(" ")
            );
            let ran = pty::spawn(&mut command).wait_within(&what, pty::DEADLINE, Some(MAX_KIB));
            assert_eq!(ran.triple(), (want.to_string(), String::new(), 0), "{what}");
        }
    }
}

/// The same bytes give the same answer whatever carries them: a terminal (typed, then one `^D`, named,
/// on standard input, or as `/dev/stdin`), a pipe on standard input, a FIFO named, and a regular file
/// named or on standard input. Each answer is jq 1.7.1's over a file, which jq gives on every route.
#[test]
fn a_terminal_pipe_fifo_and_file_read_the_same_7737() {
    let input: &[u8] = b"{\"a\":1}\n[2,\n3]\n\"x\"\n";
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("in.json");
    std::fs::write(&file, input).expect("write the input");
    for (row, (args, want)) in [
        (&["-c", "."][..], "{\"a\":1}\n[2,3]\n\"x\"\n"),
        (
            &["-R", "-c", "."][..],
            "\"{\\\"a\\\":1}\"\n\"[2,\"\n\"3]\"\n\"\\\"x\\\"\"\n",
        ),
        (
            &["--stream", "-c", "."][..],
            "[[\"a\"],1]\n[[\"a\"]]\n[[0],2]\n[[1],3]\n[[1]]\n[[],\"x\"]\n",
        ),
        (&["-s", "-c", "."][..], "[{\"a\":1},[2,3],\"x\"]\n"),
        (&["-n", "-c", "[inputs]"][..], "[{\"a\":1},[2,3],\"x\"]\n"),
        (&["-c", "first(.), halt"][..], "{\"a\":1}\n"),
    ]
    .into_iter()
    .enumerate()
    {
        for route in [
            "pty named",
            "pty stdin",
            "pty /dev/stdin",
            "pipe",
            "fifo",
            "file",
            "file stdin",
        ] {
            // `-n` with an unnamed terminal on standard input is #7742's.
            if route == "pty stdin" && args.contains(&"-n") {
                continue;
            }
            let what = format!("jq {} over {route}", args.join(" "));
            let mut command = jq(args);
            let ran = if let Some(how) = route.strip_prefix("pty ") {
                let terminal = pty::Pty::open();
                match how {
                    "named" => command.arg(terminal.path()).stdin(Stdio::null()),
                    "stdin" => command.stdin(terminal.stdin()),
                    _ => command.arg("/dev/stdin").stdin(terminal.stdin()),
                };
                let running = pty::spawn(&mut command);
                terminal.type_bytes(input);
                terminal.eof();
                running.wait(&what)
            } else if route == "pipe" {
                let output = crate::fifo_feed::output_stdin(&mut command, input).expect("run");
                pty::Ran {
                    status: output.status,
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                }
            } else if route == "fifo" {
                let feed = crate::fifo_feed::Feed::new(
                    dir.path().join(format!("fifo{row}")),
                    input.to_vec(),
                );
                command.arg(feed.path());
                let output = crate::fifo_feed::output(&mut command, &[feed]).expect("run");
                pty::Ran {
                    status: output.status,
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                }
            } else if route == "file" {
                command.arg(&file).stdin(Stdio::null());
                pty::spawn(&mut command).wait(&what)
            } else {
                command.stdin(std::fs::File::open(&file).expect("open the input"));
                pty::spawn(&mut command).wait(&what)
            };
            assert_eq!(ran.triple(), (want.to_string(), String::new(), 0), "{what}");
        }
    }
}

/// `/dev/null` is a character device too, so it now takes the stream routes, which hand its empty read
/// to the route that reads it whole: the answers are jq 1.7.1's (and 1.8.2's), as before.
#[test]
fn dev_null_reads_as_before_7737() {
    for (args, named, want) in [
        (&["-c", "."][..], true, (String::new(), String::new(), 0)),
        (&["-c", "."][..], false, (String::new(), String::new(), 0)),
        (
            &["-R", "-c", "."][..],
            true,
            (String::new(), String::new(), 0),
        ),
        (
            &["-R", "-c", "."][..],
            false,
            (String::new(), String::new(), 0),
        ),
        (
            &["-s", "-c", "."][..],
            true,
            ("[]\n".into(), String::new(), 0),
        ),
        (
            &["-n", "input"][..],
            true,
            (
                String::new(),
                "jq: error (at /dev/null:0): break\n".into(),
                5,
            ),
        ),
        (
            &["-n", "input"][..],
            false,
            (String::new(), "jq: error (at <stdin>:0): break\n".into(), 5),
        ),
        (
            &["--stream", "-c", "."][..],
            true,
            (String::new(), String::new(), 0),
        ),
        (
            &["--seq", "-c", "."][..],
            false,
            (
                String::new(),
                "jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 0\n"
                    .into(),
                0,
            ),
        ),
    ] {
        let mut command = jq(args);
        if named {
            command.arg("/dev/null").stdin(Stdio::null());
        } else {
            command.stdin(std::fs::File::open("/dev/null").expect("open /dev/null"));
        }
        let what = format!("jq {} over /dev/null (named: {named})", args.join(" "));
        assert_eq!(
            pty::spawn(&mut command).wait(&what).triple(),
            want,
            "{what}"
        );
    }
}
