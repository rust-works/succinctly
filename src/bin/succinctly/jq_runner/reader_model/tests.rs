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
    check(
        |case| Some(case.inputs_171),
        ReaderProfile::release(Release::Jq171),
        Program::Inputs,
    );
}

#[test]
fn reader_model_main_loop_equals_jq_171() {
    check(
        |case| Some(case.main_171),
        ReaderProfile::release(Release::Jq171),
        Program::Main,
    );
}

#[test]
fn reader_model_slurp_equals_jq_171() {
    check(
        |case| Some(case.slurp_171),
        ReaderProfile::release(Release::Jq171),
        Program::Slurp,
    );
}

#[test]
fn reader_model_inputs_equal_jq_182() {
    check(
        |case| (!case.skip_182).then_some(case.inputs_182),
        ReaderProfile::release(Release::Jq182),
        Program::Inputs,
    );
}

#[test]
fn reader_model_main_loop_equals_jq_182() {
    check(
        |case| (!case.skip_182).then_some(case.main_182),
        ReaderProfile::release(Release::Jq182),
        Program::Main,
    );
}

#[test]
fn reader_model_slurp_equals_jq_182() {
    check(
        |case| (!case.skip_182).then_some(case.slurp_182),
        ReaderProfile::release(Release::Jq182),
        Program::Slurp,
    );
}

/// Every profile the CLI can ship is a real release, or departs from one only by a switch listed in
/// [`EXCEPTIONS`] with its reason; and every listed exception is a real departure.
#[test]
fn reader_model_shipped_profiles_are_releases_or_listed_exceptions() {
    for release in [Release::Jq171, Release::Jq182] {
        let real = ReaderProfile::release(release);
        let shipped = ReaderProfile::shipped_for(release);
        assert_eq!(shipped.release, release);
        let listed: Vec<&str> = EXCEPTIONS
            .iter()
            .filter(|exception| exception.release == release)
            .map(|exception| exception.field)
            .collect();
        let differing: Vec<&str> = real
            .fields()
            .iter()
            .zip(shipped.fields())
            .filter(|(a, b)| a.1 != b.1)
            .map(|(a, _)| a.0)
            .collect();
        assert_eq!(differing, listed, "{release:?}: departures vs. EXCEPTIONS");
        for exception in EXCEPTIONS {
            assert!(!exception.reason.is_empty(), "{}", exception.field);
        }
    }
    // The switch names only these two, and jq 1.8.2 by default.
    assert_eq!(release_named(None), Release::Jq182);
    assert_eq!(release_named(Some("jq171")), Release::Jq171);
    assert_eq!(release_named(Some("jq182")), Release::Jq182);
    // A shipped profile equals its release on every captured case (none holds a NaN payload or
    // a document deeper than 256).
    for release in [Release::Jq171, Release::Jq182] {
        let shipped = ReaderProfile::shipped_for(release);
        for case in CASES {
            if release == Release::Jq182 && case.skip_182 {
                continue;
            }
            let (inputs, slurp) = match release {
                Release::Jq171 => (case.inputs_171, case.slurp_171),
                Release::Jq182 => (case.inputs_182, case.slurp_182),
            };
            if case.name.contains("depth") {
                continue;
            }
            assert_eq!(
                render(case, shipped, Program::Inputs),
                inputs,
                "{}",
                case.name
            );
            assert_eq!(
                render(case, shipped, Program::Slurp),
                slurp,
                "{}",
                case.name
            );
        }
    }
}

/// jq's U+FFFD rule (`jvp_utf8_next`, the same in 1.7.1 and 1.8.2), on the bytes of a string body:
/// the counts the research ran on both jq binaries (Unicode's Table 3-8 to 3-11 examples and the
/// tail swallow), where Rust's `from_utf8_lossy` gives 8, 8, 7, 4 and keeps `Zb`.
#[test]
fn reader_model_replaces_bad_utf8_as_jq_does() {
    let count = |bytes: &[u8]| jq_string_sized(bytes).matches('\u{fffd}').count();
    assert_eq!(count(b"\xC0\xAF\xE0\x80\xBF\xF0\x81\x82\x41"), 4);
    assert_eq!(count(b"\xED\xA0\x80\xED\xBF\xBF\xED\xAF\x41"), 3);
    assert_eq!(count(b"\xF4\x91\x92\x93\xFF\x41\x80\xBF\x42"), 4);
    assert_eq!(
        jq_string_sized(b"\xE1\x80\xE2\xF0\x91\x92\xF1\xBF\x41"),
        "\u{fffd}".repeat(4)
    );
    assert_eq!(jq_string_sized(b"\xF0Zb"), "\u{fffd}");
    assert_eq!(jq_string_sized(b"a\xC3\xA9"), "a\u{e9}");
    // Through the parser: a lone low surrogate escape is encoded, then replaced.
    let files: &[&[u8]] = &[b"\"\\udc00x\" \"\xF0\\n\"\n"];
    let profile = ReaderProfile::shipped();
    let mut reader = Reader::new(files, profile, false);
    assert_eq!(
        reader.next_input(),
        Step::Value(String::from("\"\u{fffd}x\""))
    );
    // `\n` decodes to a byte the lead takes with it: the string is one U+FFFD.
    assert_eq!(
        reader.next_input(),
        Step::Value(String::from("\"\u{fffd}\""))
    );
}

/// jq 1.8.2 completes a full read that ends inside a UTF-8 sequence with raw bytes, a newline
/// among them, which no read counts; 1.7.1's 4,095-byte read does not.
#[test]
fn reader_model_completes_a_utf8_sequence_cut_by_a_full_read_as_jq_182() {
    let assert_backtrack = |bytes: &[u8], missing: Option<usize>| {
        assert_eq!(utf8_backtrack(bytes), missing, "{bytes:x?}");
    };
    assert_backtrack(b"ab", Some(0));
    assert_backtrack(b"a\xE2", Some(2));
    assert_backtrack(b"a\xE2\x82", Some(1));
    assert_backtrack(b"a\xE2\x82\xAC", Some(0));
    assert_backtrack(b"a\xFF", None);
    assert_backtrack(b"\x80\x80", None);
    assert_backtrack(b"a\xC3\xA9\xA9", None);
    // `1`, 4,089 spaces, then a three-byte lead at byte 4,090: the read takes `\n2` with it.
    let mut file = b"1".to_vec();
    file.resize(4090, b' ');
    file.extend_from_slice(b"\xE2\n2\n");
    let files: &[&[u8]] = &[&file];
    // `jq -nc '[limit(4; repeat(try [input, input_line_number] catch [., input_line_number]))]'`:
    // 1.8.2 `[[1,0],["Invalid numeric literal at line 2, column 0",0],["break",1],["break",1]]`
    // (the `2` goes with the buffer the fault throws away); 1.7.1 `[[1,1],[...,1],[2,2],["break",2]]`.
    let mut reader = Reader::new(files, ReaderProfile::release(Release::Jq182), false);
    let mut items = Vec::new();
    while let Some(item) = reader.next_item() {
        items.push(item);
    }
    assert_eq!(
        items,
        vec![
            Item::Value {
                text: String::from("1"),
                source: 0,
                line: 0
            },
            Item::Fault {
                message: String::from("Invalid numeric literal at line 2, column 0"),
                source: 0,
                line: 0,
            },
        ]
    );
    assert_eq!(reader.position(), Position { source: 0, line: 1 });
    let mut reader = Reader::new(files, ReaderProfile::release(Release::Jq171), false);
    let mut lines = Vec::new();
    while let Some(item) = reader.next_item() {
        lines.push(match item {
            Item::Value { line, .. } | Item::Fault { line, .. } => line,
        });
    }
    assert_eq!(lines, [1, 1, 2]);
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
    // Round 3: a seam is no trigger on its own, whatever byte the source before it ends in (the
    // perf guard's `split*` rows are a well-formed document cut across files). The token-byte seam
    // trigger of round 2 is `SUCCINCTLY_READER_MODEL_SEAM=1`, which this test does not set.
    assert!(!t(&[b"1", b"2\n"], false));
    assert!(!t(&[b"1\n", b"[", b"", b"]\n"], false));
    assert!(!t(&[b"[1,", b"2]"], false));
    assert!(!t(&[b"tr", b"ue"], false));
    assert!(!t(&[b"{\"a\":1}", b"[2]", b"\"x\"", b"3 ", b"4"], false));
}
