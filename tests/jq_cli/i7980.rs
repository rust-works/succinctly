//! #7980: `input_line_number` one short (`0` where jq says `1`) for a value whose line ends just
//! past jq's first 4,095-byte read and holds invalid UTF-8 after the value.
//!
//! The invalid byte is jq's fault, so the run reads the value through the lazy prefix the route
//! keeps before a fault. That hand-off counted its lines over the text with U+FFFD in it, three
//! bytes for each replaced byte, which moved jq's read edge two bytes early. #7974's
//! `mark_lazy_read_as` (#7948's second shape) fixed it; this pins the value-before-the-fault shape,
//! which #7948's test does not reach.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`), on a file and on standard input. jq 1.8.2
//! reads 4,091 bytes at a time and moves the band of `0`s; the pin is 1.7.1 (ADR-0018, #7963).

use anyhow::Result;

use crate::{run_jq_binary_stdin, run_jq_over_byte_files};

/// `"` + `a`s + `"` + `tail` + a line break.
fn line(a_count: usize, tail: &[u8]) -> Vec<u8> {
    let mut text = vec![b'"'];
    text.resize(1 + a_count, b'a');
    text.push(b'"');
    text.extend_from_slice(tail);
    text.push(b'\n');
    text
}

/// For each tail, jq 1.7.1's `input_line_number` at 4,086 to 4,094 `a`s.
const ROWS: [(&[u8], [&str; 9]); 3] = [
    (b"\xff", ["1", "1", "1", "1", "1", "1", "0", "0", "1"]),
    (b" \xef", ["1", "1", "1", "1", "1", "0", "0", "0", "1"]),
    (b"\xc3", ["1", "1", "1", "1", "1", "1", "0", "0", "1"]),
];

#[test]
fn a_value_before_invalid_utf8_counts_its_line_in_the_bytes_jq_read_7980() -> Result<()> {
    let mut wrong = Vec::new();
    for (tail, lines) in ROWS {
        for (a_count, want) in (4086..).zip(lines) {
            let text = line(a_count, tail);
            let want_out = format!("{want}\n");
            let want_err = "jq: parse error: Invalid numeric literal at line 2, column 0\n";

            let (out, err, code, _) =
                run_jq_over_byte_files(&["-c", "input_line_number"], &[&text])?;
            let err = err.replace("succinctly jq:", "jq:");
            if (out.as_str(), err.as_str(), code) != (want_out.as_str(), want_err, 5) {
                wrong.push(format!("file {tail:?} {a_count}: {out:?} {err:?} {code}"));
            }

            let (out, code) = run_jq_binary_stdin("input_line_number", &text, &["-c"])?;
            if (out.as_slice(), code) != (want_out.as_bytes(), 5) {
                let out = String::from_utf8_lossy(&out);
                wrong.push(format!("stdin {tail:?} {a_count}: {out:?} {code}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "rows that differ from jq 1.7.1:\n{}",
        wrong.join("\n")
    );
    Ok(())
}
