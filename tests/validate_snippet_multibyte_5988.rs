//! #5988: the context line a validation error prints is cut on character boundaries.
//!
//! `jq --validate`, `json validate`, `yaml validate` and `text validate utf8` each cut a long line
//! to a window around the error, by byte, and panicked (exit 101) when a cut fell inside a
//! multi-byte character. They share `error_snippet::truncate_line` now. Each test pipes a line of
//! euro signs (3 bytes each, so byte 80 is never a boundary) that is invalid near its end, near
//! its start and in the middle, and checks the exit status is the validator's own and that the
//! caret stands under the character the error names.
#![cfg(feature = "cli")]

use std::io::Write;
use std::process::{Command, Stdio};

/// `(stdout, stderr, exit code)` of `succinctly <args>` with `input` on stdin.
fn run(args: &[&str], input: &[u8]) -> (String, String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawns");
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(input)
        .expect("writes");
    let out = child.wait_with_output().expect("waits");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().expect("exits, not killed by a signal"),
    )
}

/// The two lines of `stderr` that show the source line and its caret: `(shown, caret column)`,
/// the caret column counted in characters after the `| ` gutter.
fn shown_and_caret(stderr: &str) -> (String, usize) {
    let mut lines = stderr.lines().skip_while(|l| !l.contains(" | "));
    let source = lines.next().expect("a source line");
    let shown = source.split_once(" | ").expect("gutter").1.to_string();
    let caret = lines.next().expect("a caret line");
    let offset = caret
        .split_once("| ")
        .expect("gutter")
        .1
        .chars()
        .take_while(|&c| c == ' ')
        .count();
    (shown, offset)
}

/// Run `args` over `input` and check the error is reported without a panic, with the caret under
/// `under`.
fn check(args: &[&str], input: &str, want_code: i32, under: char) {
    let (_, stderr, code) = run(args, input.as_bytes());
    assert!(!stderr.contains("panicked"), "{args:?}: {stderr}");
    assert_eq!(code, want_code, "{args:?}: {stderr}");
    let (shown, caret) = shown_and_caret(&stderr);
    assert_eq!(shown.chars().nth(caret), Some(under), "{args:?}: {stderr}");
}

#[test]
fn json_errors_in_a_multibyte_line_do_not_panic() {
    let euros = "€".repeat(40);
    // The error is the `1` after the closing quote: near the end of the line.
    let late = format!("\"{euros}\" 1\n");
    check(&["jq", "--validate", "."], &late, 3, '1');
    check(&["json", "validate"], &late, 1, '1');
    // Near the start (the second value's quote), and in the middle.
    let early = format!("1 \"{euros}\"\n");
    check(&["jq", "--validate", "."], &early, 3, '"');
    let middle = format!("[\"{euros}\", \"{euros}\" 2, \"{euros}\"]\n");
    check(&["jq", "--validate", "."], &middle, 3, '2');
    check(&["json", "validate"], &middle, 1, '2');
}

#[test]
fn a_very_long_multibyte_line_does_not_panic() {
    let line = format!("\"{}\" 1\n", "€".repeat(3000));
    check(&["jq", "--validate", "."], &line, 3, '1');
    check(&["json", "validate"], &line, 1, '1');
}

#[test]
fn yaml_errors_in_a_multibyte_line_do_not_panic() {
    let euros = "€".repeat(40);
    check(
        &["yaml", "validate"],
        &format!("a: \"{euros}\" x: y\n"),
        1,
        'x',
    );
}

#[test]
fn text_errors_in_a_multibyte_line_do_not_panic() {
    let mut line = "a".repeat(60).into_bytes();
    line.extend("€".repeat(30).bytes());
    line.push(0xFF);
    line.extend(b"bbbbbbbbbbbbbbbbbbbb\n");
    let (_, stderr, code) = run(&["text", "validate", "utf8"], &line);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert_eq!(code, 1, "{stderr}");
    let (shown, caret) = shown_and_caret(&stderr);
    assert_eq!(shown.chars().nth(caret), Some('\u{FFFD}'), "{stderr}");
}
