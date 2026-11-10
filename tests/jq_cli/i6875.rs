//! #6875: a delimiter that is the last byte of the text leaves one empty field after it, as it does
//! when a line break follows it (`a,b,` is `["a","b",""]`). The row's field iterator ended at the end of
//! the text and read nothing there, so the record lost its last field when the file had no final
//! newline. Every row below was read by Python's `csv` module (`csv.reader`, `newline=""`), the oracle
//! the reader was fuzzed against with 13,000 random tables (comma, tab, semicolon and pipe, LF and CRLF,
//! both quoting modes, with and without a final line break).

use anyhow::Result;

use crate::run_jq_full;

/// (input, delimiter, the rows as JSON lines).
const ROWS: &[(&str, &str, &[&str])] = &[
    ("a,b,", ",", &["[\"a\",\"b\",\"\"]"]),
    ("a,b\n1,", ",", &["[\"a\",\"b\"]", "[\"1\",\"\"]"]),
    ("a,b\n1,,", ",", &["[\"a\",\"b\"]", "[\"1\",\"\",\"\"]"]),
    (",", ",", &["[\"\",\"\"]"]),
    ("\"x\",", ",", &["[\"x\",\"\"]"]),
    ("a,", ",", &["[\"a\",\"\"]"]),
    (",,", ",", &["[\"\",\"\",\"\"]"]),
    ("a,b,\n", ",", &["[\"a\",\"b\",\"\"]"]),
    ("a,b\n1,\n", ",", &["[\"a\",\"b\"]", "[\"1\",\"\"]"]),
    ("a,b,\r\n", ",", &["[\"a\",\"b\",\"\"]"]),
    ("a,b\r\n1,", ",", &["[\"a\",\"b\"]", "[\"1\",\"\"]"]),
    ("a,b,\r", ",", &["[\"a\",\"b\",\"\"]"]),
    ("\"a,\"", ",", &["[\"a,\"]"]),
    ("\"x,y\",", ",", &["[\"x,y\",\"\"]"]),
    ("a\t", "\t", &["[\"a\",\"\"]"]),
    ("a\tb\t", "\t", &["[\"a\",\"b\",\"\"]"]),
    ("\t", "\t", &["[\"\",\"\"]"]),
    ("a;b;", ";", &["[\"a\",\"b\",\"\"]"]),
    (
        "\u{e9},\u{65e5}\u{672c},",
        ",",
        &["[\"\u{e9}\",\"\u{65e5}\u{672c}\",\"\"]"],
    ),
    ("a,\"b\nc\",", ",", &["[\"a\",\"b\\nc\",\"\"]"]),
];

#[test]
fn a_delimiter_that_ends_the_text_leaves_an_empty_field_6875() -> Result<()> {
    for &(input, delimiter, want) in ROWS {
        let (stdout, stderr, code) =
            run_jq_full(&["--input-dsv", delimiter, "-c", "."], Some(input))?;
        assert_eq!(code, 0, "{input:?}: {stderr}");
        let got: Vec<&str> = stdout.lines().collect();
        assert_eq!(got, want, "{input:?}");
    }
    Ok(())
}
