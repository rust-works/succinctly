//! #7738: a program that calls `input`/`inputs` over a stream (a pipe, a FIFO, a device) whose bytes are
//! not UTF-8, or hold a parse fault, read the whole stream before it answered: the lazy route handed
//! such a stream to the route that reads it whole, which never ends on a stream that does not. jq reads as
//! it goes, replaces a bad byte in a string with U+FFFD, and raises a parse error for one outside a string
//! as soon as its reader gets to it. The lazy route now substitutes a stream a window at a time and reports
//! a fault from what it has read; the rest of the stream is read only for a program that asks past the
//! fault. A regular file keeps the route that reads it whole at its first such byte, as before.
//!
//! Every stream here is written by a writer that never closes it, so a run that waits for the end fails at
//! the deadline (or at [`MAX_KIB`]) instead of answering. Each expected answer is jq 1.7.1's (Linux and
//! macOS), and jq 1.8.2's agrees, over the same endless pipe.

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

/// What a run over an endless stream may hold before it is killed: it answers holding a few windows, and
/// the route that read the stream whole grew without bound.
const MAX_KIB: u64 = 512 * 1024;

/// The line an endless writer repeats after its `head`.
const FILL: &[u8] = b"{\"b\":1}\n";

fn jq(args: &[&str]) -> Command {
    let mut command = Command::new(succinctly_bin());
    command.arg("jq").args(args);
    command
}

/// Writes `head` and then [`FILL`] until the reader goes, or `stop` is set.
fn write_endlessly(mut to: impl Write + Send + 'static, head: Vec<u8>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        if to.write_all(&head).is_err() {
            return;
        }
        let block = FILL.repeat(512);
        while !stop.load(Ordering::SeqCst) {
            if to.write_all(&block).is_err() {
                return;
            }
        }
    });
}

/// The run of `args` with `head` and then an endless stream of [`FILL`] on a pipe on standard input.
fn over_endless_pipe(args: &[&str], head: &[u8]) -> (String, String, i32) {
    let mut command = jq(args);
    command.stdin(Stdio::piped());
    let mut running = pty::spawn(&mut command);
    let stop = Arc::new(AtomicBool::new(false));
    write_endlessly(
        running.take_stdin().expect("a piped stdin"),
        head.to_vec(),
        stop.clone(),
    );
    let what = format!(
        "jq {} over an endless pipe after {:?}",
        args.join(" "),
        String::from_utf8_lossy(head)
    );
    let ran = running
        .wait_within(&what, pty::DEADLINE, Some(MAX_KIB))
        .triple();
    stop.store(true, Ordering::SeqCst);
    ran
}

/// The same over a FIFO named on the command line, its writer never closing it.
fn over_endless_fifo(args: &[&str], head: &[u8]) -> (String, String, i32) {
    let dir = tempfile::tempdir().expect("tempdir");
    let fifo = dir.path().join("stream");
    assert!(Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo")
        .success());
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (fifo, head, stop) = (fifo.clone(), head.to_vec(), stop.clone());
        std::thread::spawn(move || {
            if let Ok(pipe) = std::fs::OpenOptions::new().write(true).open(&fifo) {
                write_endlessly(pipe, head, stop);
            }
        });
    }
    let mut command = jq(args);
    command.arg(&fifo).stdin(Stdio::null());
    let what = format!("jq {} over an endless FIFO", args.join(" "));
    let (stdout, stderr, code) = pty::spawn(&mut command)
        .wait_within(&what, pty::DEADLINE, Some(MAX_KIB))
        .triple();
    let name = fifo.display().to_string();
    let ran = (stdout, stderr.replace(&name, "FIFO"), code);
    stop.store(true, Ordering::SeqCst);
    // A writer still waiting in its open (the run never opened the FIFO) is let go.
    let _ = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags_nonblock()
        .open(&fifo);
    ran
}

/// `OpenOptions::custom_flags(O_NONBLOCK)`, named.
trait NonBlock {
    fn custom_flags_nonblock(&mut self) -> &mut Self;
}

impl NonBlock for std::fs::OpenOptions {
    fn custom_flags_nonblock(&mut self) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        self.custom_flags(libc::O_NONBLOCK)
    }
}

fn ok(stdout: &str) -> (String, String, i32) {
    (stdout.to_string(), String::new(), 0)
}

fn fault(at: &str, message: &str) -> (String, String, i32) {
    (
        String::new(),
        format!("jq: error (at {at}): {message}\n"),
        5,
    )
}

/// About 300 KiB of [`FILL`]: past the 256 KiB front and the first window.
fn past_a_window() -> Vec<u8> {
    FILL.repeat(300 * 1024 / FILL.len())
}

#[test]
fn a_stream_that_is_not_utf8_answers_as_jq_does_7738() {
    let big = past_a_window();
    let mut window = big.clone();
    window.extend_from_slice(b"{\"a\":\"\xff\"}\n");
    let mut two = big.clone();
    two.extend_from_slice(b"{\"a\":\"\xfe\"}\n");
    two.extend_from_slice(&big);
    two.extend_from_slice(b"{\"a\":\"\xed\xa0\x80\"}\n");
    let fffd = "\u{fffd}";
    for (args, head, want) in [
        // A bad byte outside a string is a parse error, at once.
        (
            &["-n", "-c", "input"][..],
            b"\xff\n".to_vec(),
            fault("<stdin>:1", "Invalid numeric literal at line 2, column 0"),
        ),
        (
            &["-n", "-c", "first(inputs)"][..],
            b"\xff\n".to_vec(),
            fault("<stdin>:1", "Invalid numeric literal at line 2, column 0"),
        ),
        (
            &["-n", "-c", "input"][..],
            b"\xef\xbb\xbf\xff\n".to_vec(),
            fault("<stdin>:1", "Invalid numeric literal at line 2, column 0"),
        ),
        (
            &["-n", "-c", "[limit(3; inputs)]"][..],
            b"{\"a\":1}\n\xff\n".to_vec(),
            fault("<stdin>:2", "Invalid numeric literal at line 3, column 0"),
        ),
        // In a string it is U+FFFD (an overlong pair is two), and the stream goes on.
        (
            &["-n", "-c", "first(inputs)"][..],
            b"{\"a\":\"x\xffy\"}\n".to_vec(),
            ok(&format!("{{\"a\":\"x{fffd}y\"}}\n")),
        ),
        (
            &["-n", "-c", "[limit(3; inputs)]"][..],
            b"{\"a\":\"x\xffy\"}\n".to_vec(),
            ok(&format!(
                "[{{\"a\":\"x{fffd}y\"}},{{\"b\":1}},{{\"b\":1}}]\n"
            )),
        ),
        (
            &["-n", "-c", "first(inputs)"][..],
            b"{\"a\":\"\xc0\xaf\"}\n".to_vec(),
            ok(&format!("{{\"a\":\"{fffd}{fffd}\"}}\n")),
        ),
        // A bad byte past the front, in a window of its own, and in two windows.
        (
            &["-n", "-c", "first(inputs | select(has(\"a\")))"][..],
            window,
            ok(&format!("{{\"a\":\"{fffd}\"}}\n")),
        ),
        (
            &["-n", "-c", "[limit(2; inputs | select(has(\"a\")))]"][..],
            two,
            ok(&format!("[{{\"a\":\"{fffd}\"}},{{\"a\":\"{fffd}\"}}]\n")),
        ),
    ] {
        assert_eq!(
            over_endless_pipe(args, &head),
            want,
            "jq {} after {:?}",
            args.join(" "),
            String::from_utf8_lossy(&head[..head.len().min(40)])
        );
    }
}

/// A parse fault in UTF-8 text is reported from what has been read too, after a substituted window as
/// before one, at jq's line and column (counted over the bytes as read, not over the substitution).
#[test]
fn a_fault_in_a_stream_is_reported_at_once_7738() {
    let mut lossy_then_fault = b"{\"a\":\"\xff\"}\n".to_vec();
    lossy_then_fault.extend_from_slice(&past_a_window());
    lossy_then_fault.extend_from_slice(b"[1, tru]\n");
    let mut lossy_then_nul = b"{\"a\":\"\xff\"}\n".to_vec();
    lossy_then_nul.extend_from_slice(&past_a_window());
    lossy_then_nul.extend_from_slice(b"nul\n");
    for (args, head, want) in [
        (
            &["-n", "-c", "input"][..],
            b"tru\n".to_vec(),
            fault("<stdin>:1", "Invalid literal at line 2, column 0"),
        ),
        (
            &[
                "-n",
                "-c",
                "first(inputs | select(has(\"a\") | not) | select(.b != 1))",
            ][..],
            lossy_then_fault,
            fault("<stdin>:38402", "Invalid literal at line 38402, column 8"),
        ),
        (
            &["-n", "-c", "[inputs | select(has(\"a\"))] | length"][..],
            lossy_then_nul,
            fault("<stdin>:38402", "Invalid literal at line 38403, column 0"),
        ),
    ] {
        assert_eq!(
            over_endless_pipe(args, &head),
            want,
            "jq {}",
            args.join(" ")
        );
    }
}

/// A FIFO named on the command line, and `/dev/urandom` named or on standard input, answer at once too.
/// Random bytes give random answers, so those rows pin that the run ends, in little memory, with jq's
/// kind of answer: a parse error (exit 5), or, when the bytes happen to start with a value, that value.
#[test]
fn a_fifo_and_a_device_answer_at_once_7738() {
    assert_eq!(
        over_endless_fifo(&["-n", "-c", "first(inputs)"], b"{\"a\":\"x\xffy\"}\n"),
        ok("{\"a\":\"x\u{fffd}y\"}\n")
    );
    assert_eq!(
        over_endless_fifo(&["-n", "-c", "input"], b"\xff\n"),
        fault("FIFO:1", "Invalid numeric literal at line 2, column 0")
    );
    for program in ["input", "first(inputs)", "[limit(3; inputs)]"] {
        for named in [true, false] {
            let mut command = jq(&["-n", "-c", program]);
            if named {
                command.arg("/dev/urandom").stdin(Stdio::null());
            } else {
                command.stdin(std::fs::File::open("/dev/urandom").expect("open /dev/urandom"));
            }
            let what = format!("jq -n -c {program} over /dev/urandom (named: {named})");
            let (stdout, stderr, code) = pty::spawn(&mut command)
                .wait_within(&what, pty::DEADLINE, Some(MAX_KIB))
                .triple();
            let at = if named { "/dev/urandom" } else { "<stdin>" };
            assert!(
                (code == 5 && stderr.starts_with(&format!("jq: error (at {at}:")))
                    || (code == 0 && stderr.is_empty() && !stdout.is_empty()),
                "{what}: {code} {stdout:?} {stderr:?}"
            );
        }
    }
}

/// A program that asks past the fault gets what jq's reader reads after it, which needs the rest of the
/// stream: over a pipe that ends, as jq answers it.
#[test]
fn past_a_fault_the_stream_is_read_on_7738() {
    let mut command = jq(&["-n", "-c", "[(try input catch .), input]"]);
    let output = crate::fifo_feed::output_stdin(&mut command, b"tru\n{\"b\":1}\n").expect("run");
    assert_eq!(
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code()
        ),
        (
            "[\"Invalid literal at line 2, column 0\",{\"b\":1}]\n".to_string(),
            String::new(),
            Some(0)
        )
    );
}

/// A regular file, named or redirected onto standard input, keeps the route that reads it whole at its
/// first bad byte: its length bounds the read. The answers are jq 1.7.1's.
#[test]
fn a_regular_file_reads_as_before_7738() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("in.json");
    std::fs::write(&file, b"{\"a\":\"x\xffy\"}\n{\"b\":1}\n\xff\n").expect("write");
    for named in [true, false] {
        let mut command = jq(&["-n", "-c", "[limit(2; inputs)], input"]);
        if named {
            command.arg(&file).stdin(Stdio::null());
        } else {
            command.stdin(std::fs::File::open(&file).expect("open"));
        }
        let at = if named {
            file.display().to_string()
        } else {
            "<stdin>".to_string()
        };
        let (stdout, stderr, code) = pty::spawn(&mut command).wait("a regular file").triple();
        assert_eq!(
            (
                stdout,
                stderr.replace(&file.display().to_string(), "FILE"),
                code
            ),
            (
                "[{\"a\":\"x\u{fffd}y\"},{\"b\":1}]\n".to_string(),
                format!(
                    "jq: error (at {}:3): Invalid numeric literal at line 4, column 0\n",
                    if named { "FILE" } else { "<stdin>" }
                ),
                5
            ),
            "named: {named} ({at})"
        );
    }
}

/// A terminal is a stream too: a line typed that is not UTF-8 is reported as it is typed, as jq does,
/// where the lazy route read on to the `^D` (#7742 recorded it).
#[test]
fn a_terminal_line_that_is_not_utf8_is_reported_as_typed_7738() {
    let terminal = pty::Pty::open();
    let mut command = jq(&["-n", "-c", "input"]);
    command.stdin(terminal.stdin());
    let running = pty::spawn(&mut command);
    terminal.type_bytes(b"\xff\n");
    assert_eq!(
        running
            .wait("jq -n -c input with a bad byte typed")
            .triple(),
        fault("<stdin>:1", "Invalid numeric literal at line 2, column 0")
    );
}
