//! The model against the real binaries: every expected string below is the output of jq 1.7.1 or
//! jq 1.8.2 on terminus (`gen_cases.py` in the lane's scratch directory made the files and ran
//! them), with files named `f0`, `f1`, ...
//!
//! - `inputs_*`: `jq -nc '[limit(12; repeat(try [input, input_line_number, input_filename] catch
//!   [., input_line_number, input_filename]))]' f0 f1 ...` (a read past the end raises `break`).
//! - `main_*`: `jq -c . f0 f1 ...`, standard output, then `!` and the message of jq's
//!   `jq: parse error: ...` line when the main loop stopped on one (exit 5).
//! - `slurp_*`: the first program with `limit(4; ...)` under `-nc -s`.

use super::*;

/// The `INPUTS` program over `files`, as the model reads them: `reads` reads of `input`.
fn render_inputs(files: &[&[u8]], profile: ReaderProfile, slurp: bool, reads: usize) -> String {
    let mut reader = Reader::new(files, profile, slurp);
    let mut out = String::from("[");
    for read in 0..reads {
        if read > 0 {
            out.push(',');
        }
        let step = reader.next_input();
        let at = reader.position();
        let first = match step {
            Step::Value(text) => text,
            Step::Fault(message) => json_string(&message),
            Step::Slurped(members) => format!("[{}]", members.join(",")),
            Step::End => json_string("break"),
        };
        let name = if at.source == NO_SOURCE {
            String::from("null")
        } else {
            format!("\"f{}\"", at.source)
        };
        out.push_str(&format!("[{first},{},{name}]", at.line));
    }
    out.push(']');
    out
}

/// The `MAIN` run over `files`, as the model reads them.
fn render_main(files: &[&[u8]], profile: ReaderProfile) -> String {
    let mut reader = Reader::new(files, profile, false);
    let mut out = String::new();
    loop {
        match reader.next_input() {
            Step::Value(text) => {
                out.push_str(&text);
                out.push('\n');
            }
            Step::Fault(message) => {
                out.push('!');
                out.push_str(&message);
                break;
            }
            Step::Slurped(_) | Step::End => break,
        }
    }
    out
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// A case's files: literal pieces, and runs of one byte.
fn build(files: &[&[Piece]]) -> Vec<Vec<u8>> {
    files
        .iter()
        .map(|pieces| {
            let mut bytes = Vec::new();
            for piece in *pieces {
                match *piece {
                    Piece::Lit(text) => bytes.extend_from_slice(text),
                    Piece::Run(byte, count) => bytes.resize(bytes.len() + count, byte),
                }
            }
            bytes
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Piece {
    Lit(&'static [u8]),
    Run(u8, usize),
}
use Piece::{Lit, Run};

struct Case {
    name: &'static str,
    files: &'static [&'static [Piece]],
    inputs_171: &'static str,
    main_171: &'static str,
    slurp_171: &'static str,
    inputs_182: &'static str,
    main_182: &'static str,
    slurp_182: &'static str,
    /// jq 1.8.2 differs from the model's 1.8.2 profile here for a reason the model does not port
    /// (said beside the case in `gen_cases.py`), so only 1.7.1 is checked.
    skip_182: bool,
}

include!("cases.rs");

#[derive(Clone, Copy)]
enum Program {
    Inputs,
    Main,
    Slurp,
}

fn render(case: &Case, profile: ReaderProfile, run: Program) -> String {
    let files = build(case.files);
    let files: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    match run {
        Program::Inputs => render_inputs(&files, profile, false, 12),
        Program::Main => render_main(&files, profile),
        Program::Slurp => render_inputs(&files, profile, true, 4),
    }
}

fn check(select: impl Fn(&Case) -> Option<&'static str>, profile: ReaderProfile, run: Program) {
    let mut failed = Vec::new();
    let mut checked = 0;
    for case in CASES {
        let Some(expected) = select(case) else {
            continue;
        };
        checked += 1;
        let got = render(case, profile, run);
        if got != expected {
            failed.push(format!(
                "{}:\n  model: {got}\n  jq:    {expected}",
                case.name
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {checked} cases differ:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

#[test]
fn reader_model_has_at_least_forty_cases() {
    assert!(CASES.len() >= 40, "{} cases", CASES.len());
}

#[test]
fn reader_model_inputs_equal_jq_171() {
    check(|case| Some(case.inputs_171), ReaderProfile::jq171(), Program::Inputs);
}

#[test]
fn reader_model_main_loop_equals_jq_171() {
    check(|case| Some(case.main_171), ReaderProfile::jq171(), Program::Main);
}

#[test]
fn reader_model_slurp_equals_jq_171() {
    check(|case| Some(case.slurp_171), ReaderProfile::jq171(), Program::Slurp);
}

#[test]
fn reader_model_inputs_equal_jq_182() {
    check(
        |case| (!case.skip_182).then_some(case.inputs_182),
        ReaderProfile::jq182(),
        Program::Inputs,
    );
}

#[test]
fn reader_model_main_loop_equals_jq_182() {
    check(
        |case| (!case.skip_182).then_some(case.main_182),
        ReaderProfile::jq182(),
        Program::Main,
    );
}

#[test]
fn reader_model_slurp_equals_jq_182() {
    check(
        |case| (!case.skip_182).then_some(case.slurp_182),
        ReaderProfile::jq182(),
        Program::Slurp,
    );
}

/// `succinctly()` is jq 1.7.1 but for the NUL cut of a newline-less read (#5654) and a NaN's
/// payload (#7522), so it equals 1.7.1 on every case with no NUL (none holds such a NaN).
#[test]
fn reader_model_succinctly_profile_is_171_without_the_nul_cut() {
    let profile = ReaderProfile::succinctly();
    assert_eq!(
        ReaderProfile {
            nul_cut: true,
            nan_payload: true,
            ..profile
        },
        ReaderProfile::jq171()
    );
    let mut checked = 0;
    for case in CASES {
        if build(case.files).iter().any(|file| file.contains(&0)) {
            continue;
        }
        checked += 1;
        assert_eq!(render(case, profile, Program::Inputs), case.inputs_171, "{}", case.name);
        assert_eq!(render(case, profile, Program::Slurp), case.slurp_171, "{}", case.name);
    }
    assert!(checked >= 30, "{checked} cases without a NUL");
}

#[test]
fn reader_model_trigger_reads_the_edges_only() {
    let t = |sources: &[&[u8]], nul: bool| trigger(sources.iter().copied(), nul);
    // Well-formed input never fires it.
    assert!(!t(&[b"1\n2\n"], false));
    assert!(!t(&[b"1\n", b"2\n"], false));
    assert!(!t(&[b"1\n", b"2"], false));
    assert!(!t(&[b"\xEF\xBB\xBF1\n", b"2\n"], false));
    assert!(!t(&[b"", b"1", b""], false));
    assert!(!t(&[], false));
    // A NUL anywhere.
    assert!(t(&[b"1\n"], true));
    // A mark that is not whole in the first source with bytes.
    assert!(t(&[b"\xEF"], false));
    assert!(t(&[b"", b"\xEF\xBB", b"\xBF1\n"], false));
    assert!(t(&[b"\xEF1\n"], false));
    // A source before the last that ends in a byte a bare token or an open container ends in.
    assert!(t(&[b"1", b"2\n"], false));
    assert!(t(&[b"1\n", b"[", b"", b"]\n"], false));
    assert!(t(&[b"[1,", b"2]"], false));
    assert!(t(&[b"tr", b"ue"], false));
    // One that ends where a string or container closes, or on white space, does not.
    assert!(!t(&[b"{\"a\":1}", b"[2]", b"\"x\"", b"3 ", b"4"], false));
}
