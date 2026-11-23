//! #7812: a stream with a parse fault inside a container that never closes (`[tru,`, `{x`, `[1 2`, or
//! random bytes after a `{`) was read on looking for the close, which never came: the run grew until it
//! was killed, where jq reports the fault as its reader gets to it. The scan's judgement of an open value
//! is a bracket and quote count; a value a stream is read on for past the look-ahead is now read by jq's
//! parser model (`JqOpenTail`, #7750) as it goes, on the plain filter's route and the lazy `input` route,
//! and its first fault ends the read.
//!
//! Every stream here is written by a writer that never closes it (a pipe, a FIFO, a terminal), so a run
//! that reads on for the close fails at [`MAX_KIB`] or the deadline instead of hanging. Each expected
//! answer is jq 1.7.1's (Linux and macOS) over the same endless stream, and jq 1.8.2's agrees. `-s` reads
//! its input whole by design (recorded in limitations.md) and is not here.

#![cfg(unix)]

// Each per-issue file includes the helper itself, so no shared `mod` list is edited (#5697).
#[allow(clippy::duplicate_mod)]
#[path = "../common/pty.rs"]
mod pty;

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::cargo_run_exit::succinctly_bin;

/// What a run over an endless stream may hold before it is killed: it answers holding the look-ahead and
/// a few windows, and the route that read on for the close grew without bound.
const MAX_KIB: u64 = 256 * 1024;

/// The line every endless writer repeats after its head: no `]` or `}` ever closes the value.
const FILL: &[u8] = b"x y z\n";

fn jq(args: &[&str]) -> Command {
    let mut command = Command::new(succinctly_bin());
    command.arg("jq").args(args);
    command
}

/// Writes `head` and then `fill` until the reader goes or `stop` is set.
fn write_endlessly(
    mut to: impl Write + Send + 'static,
    head: Vec<u8>,
    fill: &'static [u8],
    stop: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        if to.write_all(&head).is_err() {
            return;
        }
        let block = fill.repeat(4096 / fill.len().max(1) + 1);
        while !stop.load(Ordering::SeqCst) {
            if to.write_all(&block).is_err() {
                return;
            }
        }
    });
}

/// Where the endless stream comes from.
#[derive(Clone, Copy, Debug)]
enum Route {
    /// A pipe on standard input.
    Pipe,
    /// A FIFO named on the command line (`FIFO` in what the run says).
    Fifo,
    /// A terminal named by its device path (`TTY`).
    Terminal,
}

fn over_endless(
    route: Route,
    args: &[&str],
    head: &[u8],
    fill: &'static [u8],
) -> (String, String, i32) {
    let stop = Arc::new(AtomicBool::new(false));
    let what = format!(
        "jq {} over an endless {route:?} after {:?}",
        args.join(" "),
        String::from_utf8_lossy(&head[..head.len().min(20)])
    );
    let mut command = jq(args);
    let (stdout, stderr, code, name, placeholder) = match route {
        Route::Pipe => {
            command.stdin(Stdio::piped());
            let mut running = pty::spawn(&mut command);
            write_endlessly(
                running.take_stdin().expect("a piped stdin"),
                head.to_vec(),
                fill,
                stop.clone(),
            );
            let (o, e, c) = running
                .wait_within(&what, pty::DEADLINE, Some(MAX_KIB))
                .triple();
            (o, e, c, String::new(), "")
        }
        Route::Fifo => {
            let dir = tempfile::tempdir().expect("tempdir");
            let fifo = dir.path().join("stream");
            assert!(Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .expect("mkfifo")
                .success());
            {
                let (fifo, head, stop) = (fifo.clone(), head.to_vec(), stop.clone());
                std::thread::spawn(move || {
                    if let Ok(pipe) = std::fs::OpenOptions::new().write(true).open(&fifo) {
                        write_endlessly(pipe, head, fill, stop);
                    }
                });
            }
            command.arg(&fifo).stdin(Stdio::null());
            let (o, e, c) = pty::spawn(&mut command)
                .wait_within(&what, pty::DEADLINE, Some(MAX_KIB))
                .triple();
            // A writer still waiting in its open (the run never opened the FIFO) is let go.
            {
                use std::os::unix::fs::OpenOptionsExt;
                let _ = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&fifo);
            }
            (o, e, c, fifo.display().to_string(), "FIFO")
        }
        Route::Terminal => {
            let terminal = pty::Pty::open();
            terminal.type_bytes(head);
            let _streaming = terminal.stream(fill);
            command.arg(terminal.path()).stdin(Stdio::null());
            let (o, e, c) = pty::spawn(&mut command)
                .wait_within(&what, pty::DEADLINE, Some(MAX_KIB))
                .triple();
            (o, e, c, terminal.path().display().to_string(), "TTY")
        }
    };
    stop.store(true, Ordering::SeqCst);
    let stderr = if name.is_empty() {
        stderr
    } else {
        stderr.replace(&name, placeholder)
    };
    (stdout, stderr, code)
}

/// The name a route's `input` error gives the stream.
fn at(route: Route) -> &'static str {
    match route {
        Route::Pipe => "<stdin>",
        Route::Fifo => "FIFO",
        Route::Terminal => "TTY",
    }
}

const ROUTES: [Route; 3] = [Route::Pipe, Route::Fifo, Route::Terminal];

#[test]
fn a_fault_inside_a_container_that_never_closes_is_reported_7812() {
    for (head, message) in [
        (&b"[tru,"[..], "Invalid literal at line 1, column 5"),
        (&b"{x"[..], "Invalid numeric literal at line 1, column 4"),
        (&b"[1 2"[..], "Invalid numeric literal at line 1, column 6"),
        (
            &b"{\"a\" 1"[..],
            "Invalid numeric literal at line 1, column 8",
        ),
        (
            &b"[{\"a\":[1,"[..],
            "Invalid numeric literal at line 1, column 11",
        ),
        (
            &b"[1,2,3\n"[..],
            "Invalid numeric literal at line 2, column 2",
        ),
        (
            &b"{\"a\":\n1 2"[..],
            "Invalid numeric literal at line 2, column 5",
        ),
    ] {
        // A terminal is read a line a read, which is slow to the look-ahead: two shapes there.
        let routes: &[Route] = if head == b"[tru," || head == b"{\"a\":\n1 2" {
            &ROUTES
        } else {
            &ROUTES[..2]
        };
        for &route in routes {
            let line = if message.contains("line 2") { 2 } else { 1 };
            assert_eq!(
                over_endless(route, &["-c", "."], head, FILL),
                (String::new(), format!("jq: parse error: {message}\n"), 5),
                "-c . {route:?} {:?}",
                String::from_utf8_lossy(head)
            );
            for program in ["input", "[inputs]"] {
                assert_eq!(
                    over_endless(route, &["-n", "-c", program], head, FILL),
                    (
                        String::new(),
                        format!("jq: error (at {}:{line}): {message}\n", at(route)),
                        5
                    ),
                    "-n {program} {route:?} {:?}",
                    String::from_utf8_lossy(head)
                );
            }
        }
    }
}

/// `--stream` read a chunk at a time already, and answers the same; `-R` reads lines, which the open
/// container does not change.
#[test]
fn stream_and_raw_input_answer_as_before_7812() {
    for route in ROUTES {
        assert_eq!(
            over_endless(route, &["--stream", "-c", "."], b"[{\"a\":[1,", FILL),
            (
                "[[0,\"a\",0],1]\n".to_string(),
                "jq: parse error: Invalid numeric literal at line 1, column 11\n".to_string(),
                5
            ),
            "{route:?}"
        );
        assert_eq!(
            over_endless(route, &["-R", "-n", "-c", "input"], b"[tru,", FILL),
            ok("\"[tru,x y z\"\n"),
            "{route:?}"
        );
    }
}

fn ok(stdout: &str) -> (String, String, i32) {
    (stdout.to_string(), String::new(), 0)
}

/// A value longer than the look-ahead that is valid is read on to its close as before, by both routes:
/// jq's parser model holds it open, and does not stop the read.
#[test]
fn a_long_valid_value_is_read_to_its_close_7812() {
    let mut long = b"[".to_vec();
    long.extend(b"1,".repeat(2_000_000));
    long.extend_from_slice(b"1]\n");
    for route in [Route::Pipe, Route::Fifo] {
        assert_eq!(
            over_endless(route, &["-c", "length, halt"], &long, b"{\"b\":1}\n"),
            ok("2000001\n"),
            "{route:?}"
        );
        assert_eq!(
            over_endless(
                route,
                &["-n", "-c", "input | length"],
                &long,
                b"{\"b\":1}\n"
            ),
            ok("2000001\n"),
            "{route:?}"
        );
    }
}

/// A regular file keeps its route (its length bounds the read), with jq's answer.
#[test]
fn a_regular_file_reads_as_before_7812() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("in.json");
    let mut bytes = b"[tru,".to_vec();
    bytes.extend(FILL.repeat(500_000));
    std::fs::write(&file, &bytes).expect("write");
    let name = file.display().to_string();
    let run = |args: &[&str]| {
        let mut command = jq(args);
        command.arg(&file).stdin(Stdio::null());
        let (o, e, c) = pty::spawn(&mut command).wait("a regular file").triple();
        (o, e.replace(&name, "FILE"), c)
    };
    assert_eq!(
        run(&["-c", "."]),
        (
            String::new(),
            "jq: parse error: Invalid literal at line 1, column 5\n".to_string(),
            5
        )
    );
    assert_eq!(
        run(&["-n", "-c", "input"]),
        (
            String::new(),
            "jq: error (at FILE:1): Invalid literal at line 1, column 5\n".to_string(),
            5
        )
    );
}
