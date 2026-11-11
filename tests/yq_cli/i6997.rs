//! #6997: yq-mode `@base64d` reads its text through yq's base64 decoder (the one `-p base64` uses,
//! #6925), so a line break inside the text is `unexpected EOF`, as in yq. Every row was captured
//! from yq v4.53.3 (`yq -p json -o json '.s | @base64d'`).

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// (the JSON text of the string, yq's stdout, yq's error line, yq's exit status).
const ROWS: &[(&str, &str, &str, i32)] = &[
    (r#""aGVs\nbG8=""#, "", "Error: unexpected EOF", 1),
    (r#""aGVs\rbG8=""#, "", "Error: unexpected EOF", 1),
    (r#""aG\nk=""#, "", "Error: unexpected EOF", 1),
    (r#""aG\nVsbG8=""#, "", "Error: unexpected EOF", 1),
    (
        r#""aGk=\naGk=""#,
        "",
        "Error: illegal base64 data at input byte 4",
        1,
    ),
    (r#""aGVsbG8=\n""#, "\"hello\"\n", "", 0),
    (r#""\naGk=""#, "\"hi\"\n", "", 0),
    (r#""aGVsbG8\n""#, "\"hello\"\n", "", 0),
    (
        r#""====""#,
        "",
        "Error: illegal base64 data at input byte 0",
        1,
    ),
    (
        r#""ab=!""#,
        "",
        "Error: illegal base64 data at input byte 2",
        1,
    ),
    (r#""QQ""#, "\"A\"\n", "", 0),
    // A CRLF pair inside the text: both bytes were counted in the padding and both are dropped.
    (r#""aG\r\nk=""#, "", "Error: unexpected EOF", 1),
    (r#""aGVs\r\nbG8=""#, "", "Error: unexpected EOF", 1),
    (r#""aGVsbG8\r\n=""#, "", "Error: unexpected EOF", 1),
    (
        r#""aGk=\r\naGk=""#,
        "",
        "Error: illegal base64 data at input byte 4",
        1,
    ),
];

#[test]
fn base64d_reads_a_line_break_as_yq_does_6997() -> Result<()> {
    for &(text, stdout, error, code) in ROWS {
        let (out, err, status) =
            run_yq_stdin_with_stderr("@base64d", text, &["-p", "json", "-o", "json"])?;
        // A run with `RUST_BACKTRACE` set (the coverage job) adds a backtrace after the error line.
        let line = err.lines().next().unwrap_or("");
        assert_eq!(
            (out.as_str(), line, status),
            (stdout, error, code),
            "{text}"
        );
    }
    Ok(())
}

/// Go's decoder hands the text to `Encoding.Decode` a chunk at a time: an offset past the first
/// 680 bytes counts from its chunk (yq v4.53.3: `illegal base64 data at input byte 564`).
#[test]
fn base64d_counts_an_offset_from_its_chunk_6997() -> Result<()> {
    let mut text = "enp6".repeat(2000);
    text.replace_range(5000..5001, "!");
    let (_, err, code) = run_yq_stdin_with_stderr(
        "@base64d",
        &format!("\"{text}\""),
        &["-p", "json", "-o", "json"],
    )?;
    let line = err.lines().next().unwrap_or("");
    assert_eq!(
        (line, code),
        ("Error: illegal base64 data at input byte 564", 1)
    );
    Ok(())
}
