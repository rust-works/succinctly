//! #7742: with `-n`, no input files and a terminal on standard input, `input` and `inputs` saw an empty
//! stream (`[inputs]` printed `[]` before anything was typed, `input` raised `break`): the read was
//! left out on purpose (#723), for fear of blocking on a terminal the program never reads. jq 1.7.1 and
//! 1.8.2 read the terminal, a line at a time, and only when the program asks. So does succinctly now:
//! the lazy route reads nothing of a terminal until `input` asks, and then a line as it is typed.
//!
//! Each row is jq 1.7.1's answer and the number of `^D` it needs (Linux and macOS; jq 1.8.2 agrees),
//! zero where jq answers as soon as the lines are typed. The test types the input and all but the last
//! `^D`, checks the run is still waiting (when it needs one), then types the last and waits for the run
//! within the deadline, so a run that waits for more than jq does fails there instead of hanging.

#![cfg(unix)]

// Each per-issue file includes the helper itself, so no shared `mod` list is edited (#5697).
#[allow(clippy::duplicate_mod)]
#[path = "../common/pty.rs"]
mod pty;

use std::process::{Command, Stdio};
use std::time::Duration;

use crate::cargo_run_exit::succinctly_bin;

/// How the terminal reaches the run.
#[derive(Clone, Copy, Debug)]
enum Route {
    /// On standard input, with no input file.
    Stdin,
    /// On standard input, named as `-`.
    Dash,
    /// Named by its device path, standard input null.
    Named,
}

fn typed(args: &[&str], route: Route, input: &[u8], eofs: usize) -> (String, String, i32) {
    let terminal = pty::Pty::open();
    let mut command = Command::new(succinctly_bin());
    command.arg("jq").args(args);
    match route {
        Route::Stdin => command.stdin(terminal.stdin()),
        Route::Dash => command.arg("-").stdin(terminal.stdin()),
        Route::Named => command.arg(terminal.path()).stdin(Stdio::null()),
    };
    let what = format!(
        "jq {} ({route:?}) with {:?} and {eofs} ^D",
        args.join(" "),
        String::from_utf8_lossy(input)
    );
    let mut running = pty::spawn(&mut command);
    terminal.type_bytes(input);
    if eofs > 0 {
        for _ in 1..eofs {
            terminal.eof();
        }
        assert!(
            !running.ended_within(Duration::from_millis(400)),
            "{what}: ended before its last ^D"
        );
        terminal.eof();
    }
    let ran = running.wait(&what).triple();
    // The name the run gives the terminal, which jq gives it too.
    let name = terminal.path().display().to_string();
    (
        ran.0.replace(&name, "TTY"),
        ran.1.replace(&name, "TTY"),
        ran.2,
    )
}

fn ok(stdout: &str) -> (String, String, i32) {
    (stdout.to_string(), String::new(), 0)
}

#[test]
fn null_input_reads_a_terminal_on_stdin_as_jq_does_7742() {
    let lines = "1\n2\n3\n";
    for (args, input, eofs, want) in [
        (&["-n", "-c", "[inputs]"][..], lines, 1, ok("[1,2,3]\n")),
        (&["-n", "-c", "input"][..], lines, 0, ok("1\n")),
        (&["-n", "-c", "[input, input]"][..], lines, 0, ok("[1,2]\n")),
        (&["-n", "-c", "first(inputs)"][..], lines, 0, ok("1\n")),
        (
            &["-n", "-c", "[input, input_line_number]"][..],
            lines,
            0,
            ok("[1,1]\n"),
        ),
        (
            &["-n", "-c", "reduce inputs as $x (0; .+$x)"][..],
            lines,
            1,
            ok("6\n"),
        ),
        (&["-n", "-c", "input"][..], "[1,\n2]\n3\n", 0, ok("[1,2]\n")),
        (
            &["-n", "-c", "input"][..],
            "{\"a\":1} {\"b\":2}\n",
            0,
            ok("{\"a\":1}\n"),
        ),
        (
            &["-n", "-c", "[input, input]"][..],
            "1 2\n",
            0,
            ok("[1,2]\n"),
        ),
        // A line with no newline is handed on by a first `^D`, and the stream ends at a second.
        (&["-n", "-c", "input"][..], "1", 2, ok("1\n")),
        // A BOM at the front is skipped, as jq skips it.
        (&["-n", "-c", "input"][..], "\u{feff}1\n", 0, ok("1\n")),
        // Nothing typed: the stream ends at the first `^D`.
        (&["-n", "-c", "[inputs]"][..], "", 1, ok("[]\n")),
        (
            &["-n", "-c", "input"][..],
            "",
            1,
            (String::new(), "jq: error (at <stdin>:0): break\n".into(), 5),
        ),
        (
            &["-n", "-c", "try input catch ."][..],
            "",
            1,
            ok("\"break\"\n"),
        ),
        // A program that never asks for an input never waits for the terminal.
        (
            &["-n", "-c", "if false then input else 1 end"][..],
            "",
            0,
            ok("1\n"),
        ),
        (&["-n", "-c", "input_line_number"][..], "", 0, ok("0\n")),
    ] {
        assert_eq!(
            typed(args, Route::Stdin, input.as_bytes(), eofs),
            want,
            "jq {} with {input:?}",
            args.join(" ")
        );
    }
}

/// A terminal named as `-` or by its path is read a line at a time too, as jq reads it.
#[test]
fn null_input_reads_a_named_terminal_a_line_at_a_time_7742() {
    for route in [Route::Dash, Route::Named] {
        assert_eq!(
            typed(&["-n", "-c", "input"], route, b"1\n2\n3\n", 0),
            ok("1\n"),
            "{route:?}"
        );
        assert_eq!(
            typed(&["-n", "-c", "[inputs]"], route, b"1\n2\n3\n", 1),
            ok("[1,2,3]\n"),
            "{route:?}"
        );
    }
    assert_eq!(
        typed(
            &["-n", "-c", "input_filename, input, input_filename"],
            Route::Named,
            b"1\n",
            0
        ),
        ok("null\n1\n\"TTY\"\n")
    );
}

/// What is left of #7742, recorded in `docs/compliance/jq/limitations.md`: `-R -n` and `--stream -n`
/// read the terminal, with jq's answers, but not a line at a time: `-R` waits for its `^D` (or a window of
/// typing) before the program runs, and `--stream` for 3 bytes, where jq answers the line `1` as it is
/// typed. And a line the lazy route cannot parse (a fault, a byte that is not UTF-8) is reported after
/// the `^D`, with jq's message, where jq reports it as the line is typed. Each row checks the run is
/// still waiting before its `^D`: a fix makes that fail, and the record is then to be updated.
#[test]
fn what_waits_for_more_than_a_line_on_a_terminal_7742() {
    let fault = |message: &str| {
        (
            String::new(),
            format!("jq: error (at <stdin>:1): {message}\n"),
            5,
        )
    };
    for (args, input, want) in [
        (
            &["-R", "-n", "-c", "input"][..],
            &b"1\n2\n3\n"[..],
            ok("\"1\"\n"),
        ),
        (
            &["--stream", "-n", "-c", "input"][..],
            &b"1\n"[..],
            ok("[[],1]\n"),
        ),
        (
            &["-n", "-c", "input"][..],
            &b"tru\n"[..],
            fault("Invalid literal at line 2, column 0"),
        ),
        (
            &["-n", "-c", "input"][..],
            &b"\xff\n"[..],
            fault("Invalid numeric literal at line 2, column 0"),
        ),
    ] {
        assert_eq!(
            typed(args, Route::Stdin, input, 1),
            want,
            "jq {} with {:?}",
            args.join(" "),
            String::from_utf8_lossy(input)
        );
    }
}
