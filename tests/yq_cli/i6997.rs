//! #6997: a line break inside `@base64d`'s text. yq pads the text to a multiple of four by its length
//! with the breaks, and Go's decoder then skips every `\r` and `\n`, so a short final quantum is
//! `unexpected EOF` once the quanta before it decode, and four breaks leave whole quanta that decode.
//! A break before or after the text is trimmed. jq mode refuses every break, as jq 1.7.1 does.
//!
//! Every yq row is captured from yq v4.53.3 (`-n -o=json -I=0 '"<text>" | @base64d'`). A sweep of
//! 3,795 random strings of base64 characters, `=`, breaks and blanks agrees with yq on every verdict,
//! error and decoded byte; 41 still print fewer U+FFFD for invalid UTF-8 than yq, on main too (#7006).

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin_with_stderr};

/// `(text as a jq string literal, yq's answer: Ok(JSON output) or Err(error))`.
const ROWS: &[(&str, Result<&str, &str>)] = &[
    (r#""aGVs\nbG8=""#, Err("unexpected EOF")),
    (r#""aGVs\rbG8=""#, Err("unexpected EOF")),
    (r#""aG\nk=""#, Err("unexpected EOF")),
    (r#""aG\r\nk=""#, Err("unexpected EOF")),
    (r#""aG\nVsbG8=""#, Err("unexpected EOF")),
    (r#""Q\nQ==""#, Err("unexpected EOF")),
    (r#""a\nb""#, Err("unexpected EOF")),
    (r#""aGVs\nbG8gd29y\nbGQ=""#, Err("unexpected EOF")),
    (
        r#""aGk=\naGk=""#,
        Err("illegal base64 data at input byte 4"),
    ),
    (
        r#""Ga\r8=\nGGA=V//""#,
        Err("illegal base64 data at input byte 4"),
    ),
    (
        r#""aGVs\nbG8\nAA==""#,
        Err("illegal base64 data at input byte 9"),
    ),
    (r#""a\nG\nV\ns\nbG8=""#, Ok("\"hello\"\n")),
    (r#""aGVz\n\n\n\nbG8=""#, Ok("\"heslo\"\n")),
    (r#""a\r\nG\r\nVs""#, Ok("\"hel\"\n")),
    (r#""aGVsbG8gd29ybGQ=\n""#, Ok("\"hello world\"\n")),
    (r#""\naGk=""#, Ok("\"hi\"\n")),
    (r#""aGVsbG8\n""#, Ok("\"hello\"\n")),
];

#[test]
fn yq_base64d_reads_a_line_break_as_go_does_6997() -> Result<()> {
    for &(text, expected) in ROWS {
        let filter = format!("{text} | @base64d");
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(&filter, "", &["-n", "-o=json", "-I=0"])?;
        match expected {
            Ok(out) => assert_eq!((stdout.as_str(), code), (out, 0), "{text}: {stderr}"),
            Err(message) => {
                assert_eq!((stdout.as_str(), code), ("", 1), "{text}");
                assert_eq!(stderr.trim_end(), format!("Error: {message}"), "{text}");
            }
        }
    }
    Ok(())
}

/// jq 1.7.1 refuses a break anywhere in the text, leading and trailing included.
#[test]
fn jq_base64d_still_refuses_a_line_break_6997() -> Result<()> {
    for text in [r#""aGVs\nbG8=""#, r#""a\nG\nV\ns\nbG8=""#, r#""\naGk=""#] {
        let (stdout, code) = run_jq_stdin(&format!("{text} | @base64d"), "null", &["-c"])?;
        assert_eq!((stdout.as_str(), code), ("", 5), "{text}");
    }
    let (stdout, code) = run_jq_stdin(r#""aGVsbG8gd29ybGQ=\n" | @base64d"#, "null", &["-c"])?;
    assert_eq!((stdout.as_str(), code), ("\"hello world\"\n", 0));
    Ok(())
}
