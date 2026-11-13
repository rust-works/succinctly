//! #7102: a run of backslashes before a character that is not `(` collapses twice in yq, once in its
//! lexer and once when the string is read. With `n` backslashes, `k = n / 2` of them survive the lexer;
//! an odd `n` leaves one more, which reads an escape with the next character when yq decodes it
//! (`n t r b f a v "`) and stays text in front of that character when it does not. The second pass turns
//! each pair of what is left into one backslash and keeps a lone one, and decodes nothing, so `\\n` is
//! the two characters `\n` while `\n` is a newline. The result keeps `ceil(n / 4)` backslashes before an
//! ordinary character (1, 1, 1, 1, 2, ... for n = 1..), and `ceil((n - 1) / 4)` before a decoded one,
//! whose decoded character follows.
//!
//! That is for a string whose interpolations all close. A string with an unclosed `\(` and a program run
//! with `--string-interpolation=false` collapse once (`n / 2` pairs, and a lone one stays), which
//! `succinctly yq` already did. A run before `(` is #7063's.
//!
//! Every row was captured from yq v4.53.3 (`yq -n -o=json -I0 --from-file FILE`, so no shell touches the
//! backslashes), for `n` = 1..=14 before each character of [`AFTER`].

use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::TempDir;

/// What follows a run, and the character yq's lexer decodes it to when an odd run's last backslash
/// reads it as an escape (empty: it decodes nothing, so the backslash stays).
const AFTER: &[(&str, &str)] = &[
    ("x", ""),
    ("n", "\n"),
    ("t", "\t"),
    ("r", "\r"),
    ("b", "\u{8}"),
    ("f", "\u{c}"),
    ("a", "\u{7}"),
    ("v", "\u{b}"),
    ("d", ""),
    ("u0041", ""),
    ("/", ""),
    (" ", ""),
    ("\u{e9}", ""),
    ("\"", "\""),
];

/// Backslashes kept for n = 1..=14, per [`AFTER`] row, in a string whose interpolations all close.
/// `-1`: yq refuses the program (an even run before `"` ends the string, and `z"` is not a term).
const TWICE: [[i8; 14]; 14] = [
    [1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4],
    [0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4],
    [0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4],
    [0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4],
    [0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4],
    [0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4],
    [0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4],
    [0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4],
    [1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4],
    [1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4],
    [1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4],
    [1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4],
    [1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4],
    [0, -1, 1, -1, 1, -1, 2, -1, 2, -1, 3, -1, 3, -1],
];

/// The same for a string collapsed once: `--string-interpolation=false`, or an unclosed `\(` after the run.
const ONCE: [[i8; 14]; 14] = [
    [1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7],
    [0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7],
    [0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7],
    [0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7],
    [0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7],
    [0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7],
    [0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7],
    [0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7],
    [1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7],
    [1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7],
    [1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7],
    [1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7],
    [1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7],
    [0, -1, 1, -1, 2, -1, 3, -1, 4, -1, 5, -1, 6, -1],
];

/// Backslashes kept for a run of n = 1..=14 right before the closing quote: an odd run escapes the quote
/// and leaves the string unterminated, which yq refuses.
const BEFORE_CLOSING_QUOTE: [i8; 14] = [-1, 1, -1, 1, -1, 2, -1, 2, -1, 3, -1, 3, -1, 4];

/// `(stdout, exit code)` of `succinctly yq ARGS -n -o=json -I0 --from-file FILE` for `program`.
fn run(args: &[&str], program: &str) -> Result<(String, Option<i32>)> {
    let dir = TempDir::new()?;
    let file = dir.path().join("p.yq");
    std::fs::write(&file, program)?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(args)
        .args(["-n", "-o=json", "-I0", "--from-file"])
        .arg(&file)
        .stdin(Stdio::null())
        .output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    ))
}

/// Runs every `AFTER` row of `table` for each run length in one program per length, as an array of
/// strings `PRE a RUN AFTER z SUF`, and compares it with what yq printed: `a`, the kept backslashes, the
/// decoded character (an odd run that reads an escape) or `AFTER` itself, then `z`, with `epre` and
/// `esuf` for what the slots of `PRE` and `SUF` print.
fn check_table(
    args: &[&str],
    table: &[[i8; 14]; 14],
    pre: &str,
    epre: &str,
    suf: &str,
    esuf: &str,
) -> Result<()> {
    for n in 1..=14usize {
        let mut pieces = Vec::new();
        let mut want = Vec::new();
        for (&(after, decoded), kept) in AFTER.iter().zip(table) {
            let kept = kept[n - 1];
            if kept < 0 {
                continue;
            }
            pieces.push(format!("\"{pre}a{}{after}z{suf}\"", "\\".repeat(n)));
            let tail = if n % 2 == 1 && !decoded.is_empty() {
                decoded
            } else {
                after
            };
            want.push(format!(
                "{epre}a{}{tail}z{esuf}",
                "\\".repeat(usize::try_from(kept)?)
            ));
        }
        let program = format!("[{}]", pieces.join(", "));
        let (stdout, code) = run(args, &program)?;
        assert_eq!(code, Some(0), "n = {n}: {program}");
        let got: Vec<String> = serde_json::from_str(&stdout)?;
        for (i, (got, want)) in got.iter().zip(&want).enumerate() {
            assert_eq!(got, want, "n = {n}: {}", pieces[i]);
        }
        assert_eq!(got.len(), want.len(), "n = {n}: {program}");
    }
    Ok(())
}

#[test]
fn a_backslash_run_before_a_character_collapses_twice_as_in_yq_7102() -> Result<()> {
    check_table(&[], &TWICE, "", "", "", "")
}

/// A slot before the run and one after it leave the answer as it is: only a slot that does not close
/// changes it.
#[test]
fn closed_slots_around_a_run_leave_the_double_collapse_7102() -> Result<()> {
    check_table(&[], &TWICE, "\\(1)", "1", "\\(2)", "2")
}

#[test]
fn a_run_collapses_once_in_a_string_with_an_unclosed_slot_7102() -> Result<()> {
    // The unclosed `\(` after the run makes yq skip interpolation and keep the string as written.
    check_table(&[], &ONCE, "", "", "\\(", "\\(")?;
    check_table(&[], &ONCE, "\\(1 ", "\\(1 ", "", "")
}

#[test]
fn a_run_collapses_once_with_string_interpolation_off_7102() -> Result<()> {
    check_table(&["--string-interpolation=false"], &ONCE, "", "", "", "")
}

#[test]
fn a_run_before_the_closing_quote_collapses_twice_7102() -> Result<()> {
    for (i, &kept) in BEFORE_CLOSING_QUOTE.iter().enumerate() {
        let n = i + 1;
        let program = format!("\"a{}\"", "\\".repeat(n));
        let (stdout, code) = run(&[], &program)?;
        if kept < 0 {
            assert_eq!((stdout.as_str(), code), ("", Some(1)), "n = {n}");
        } else {
            let want = format!("\"a{}\"\n", "\\\\".repeat(usize::try_from(kept)?));
            assert_eq!((stdout, code), (want, Some(0)), "n = {n}");
        }
    }
    Ok(())
}

/// An even run before `"` ends the string, so what follows is not a term; yq refuses it.
#[test]
fn an_even_run_before_a_quote_ends_the_string_7102() -> Result<()> {
    for n in [2usize, 4, 6] {
        let program = format!("\"a{}\"z\"", "\\".repeat(n));
        let (stdout, code) = run(&[], &program)?;
        assert_eq!((stdout.as_str(), code), ("", Some(1)), "n = {n}");
    }
    Ok(())
}

/// The same literal read as an object key, as a string argument and as a replacement: one lexer.
#[test]
fn the_double_collapse_holds_in_a_key_an_argument_and_a_replacement_7102() -> Result<()> {
    let four = "\\".repeat(4);
    let eight = "\\".repeat(8);
    for (program, want) in [
        (
            format!("{{\"a{four}x\": 1}} | keys | .[0]"),
            "\"a\\\\x\"\n".to_string(),
        ),
        (format!("\"a{eight}x\" | length"), "4\n".to_string()),
        (
            format!("\"x\" | sub(\"x\"; \"a{eight}x\")"),
            "\"a\\\\\\\\x\"\n".to_string(),
        ),
    ] {
        let (stdout, code) = run(&[], &program)?;
        assert_eq!((stdout, code), (want, Some(0)), "{program}");
    }
    Ok(())
}
