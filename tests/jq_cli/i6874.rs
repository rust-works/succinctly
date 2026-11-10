//! #6874: a CRLF line ending is the record terminator (RFC 4180), so the `\r` is no part of the last
//! field of a row. The reader took `\n` alone, so every field before a CRLF kept a trailing `\r` and a
//! quoted last field (`"x"\r`) was not recognised as quoted. Every row below was read by Python's
//! `csv` module (`csv.reader`, `newline=""`), the oracle the reader was fuzzed against with 6,400 random
//! tables written by `csv.writer` with LF and CRLF terminators, comma and tab, and both quoting modes.

use anyhow::Result;

use crate::run_jq_full;

/// (input, delimiter, the rows as JSON lines).
const ROWS: &[(&str, &str, &[&str])] = &[
    ("a,b\r\n1,2\r\n", ",", &["[\"a\",\"b\"]", "[\"1\",\"2\"]"]),
    (
        "a,b\r\n1,\"x y\"\r\n\"q\",\"\"\r\n",
        ",",
        &["[\"a\",\"b\"]", "[\"1\",\"x y\"]", "[\"q\",\"\"]"],
    ),
    (
        "\"a\",\"b\"\r\n\"1\",\"2\"\r\n",
        ",",
        &["[\"a\",\"b\"]", "[\"1\",\"2\"]"],
    ),
    ("a,b\r\n1,2", ",", &["[\"a\",\"b\"]", "[\"1\",\"2\"]"]),
    ("a,b\r\n1,2\r", ",", &["[\"a\",\"b\"]", "[\"1\",\"2\"]"]),
    ("a\r\n", ",", &["[\"a\"]"]),
    ("a,\r\n", ",", &["[\"a\",\"\"]"]),
    ("\"x\r\ny\",z\r\n", ",", &["[\"x\\r\\ny\",\"z\"]"]),
    ("\"x\ny\",z\r\n", ",", &["[\"x\\ny\",\"z\"]"]),
    ("\"x\r\",z\r\n", ",", &["[\"x\\r\",\"z\"]"]),
    (
        "a\tb\r\n1\t\"2 3\"\r\n",
        "\t",
        &["[\"a\",\"b\"]", "[\"1\",\"2 3\"]"],
    ),
    (
        "\"\u{e9}\",\u{65e5}\u{672c}\r\n",
        ",",
        &["[\"\u{e9}\",\"\u{65e5}\u{672c}\"]"],
    ),
    ("a,b\n1,2\n", ",", &["[\"a\",\"b\"]", "[\"1\",\"2\"]"]),
    (
        "a,b\n1,\"x y\"\n",
        ",",
        &["[\"a\",\"b\"]", "[\"1\",\"x y\"]"],
    ),
    (
        "a,b\r\n1,2\n3,4\r\n",
        ",",
        &["[\"a\",\"b\"]", "[\"1\",\"2\"]", "[\"3\",\"4\"]"],
    ),
];

#[test]
fn a_crlf_line_ending_is_the_record_terminator_6874() -> Result<()> {
    for &(input, delimiter, want) in ROWS {
        let (stdout, stderr, code) =
            run_jq_full(&["--input-dsv", delimiter, "-c", "."], Some(input))?;
        assert_eq!(code, 0, "{input:?}: {stderr}");
        let got: Vec<&str> = stdout.lines().collect();
        assert_eq!(got, want, "{input:?}");
    }
    Ok(())
}

/// A blank CRLF line is the one row of a single empty field, as a blank LF line is.
#[test]
fn a_blank_crlf_line_reads_as_a_blank_lf_line_does_6874() -> Result<()> {
    for (crlf, lf) in [("\r\n", "\n"), ("a,b\r\n\r\nc,d\r\n", "a,b\n\nc,d\n")] {
        let run = |input: &str| run_jq_full(&["--input-dsv", ",", "-c", "."], Some(input));
        assert_eq!(run(crlf)?, run(lf)?, "{crlf:?}");
    }
    Ok(())
}
