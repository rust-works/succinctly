//! #7445: a bad escape in a program string literal is the message of jq's JSON decoder for the run
//! of escapes it sits in, `Invalid escape at line 1, column 4 (while parsing '"\q"')`, not a lexer
//! message of succinctly's own. jq's lexer hands each run of consecutive escapes of a literal
//! (`\n\q` is one run; a `\(` interpolation and any literal text end it) to `jv_parse` as the string
//! `"<run>"`, so the message, its column and the quoted text are deterministic, and each bad run is
//! an error of its own (`"\q\(1)\q"` is two, `2 compile errors`). Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -n PROGRAM`, stderr, exit 3) and agrees with jq 1.8.2 in everything but the
//! error-position suffix (jqlang/jq#3257: `at <top-level>, line 1, column 2:` and a caret line), which
//! is jq's 1.8 presentation of the position and is not matched (`docs/compliance/jq/limitations.md`).
//! What follows ` at <top-level>` is the position line and the echoed source, whose padding is a
//! recorded residual, so a row compares the messages and the count.

use anyhow::Result;

use crate::run_jq_full;

/// (program, the message of each `jq: error: ... at <top-level>` line, the trailer).
const ROWS: &[(&str, &[&str], &str)] = &[
    (
        "\"\\q\"",
        &["Invalid escape at line 1, column 4 (while parsing '\"\\q\"')"],
        "1 compile error",
    ),
    (
        "\"ab\\q\"",
        &["Invalid escape at line 1, column 4 (while parsing '\"\\q\"')"],
        "1 compile error",
    ),
    (
        "\"\\n\\q\"",
        &["Invalid escape at line 1, column 6 (while parsing '\"\\n\\q\"')"],
        "1 compile error",
    ),
    (
        "\"\\u12\"",
        &["Invalid \\uXXXX escape at line 1, column 6 (while parsing '\"\\u12\"')"],
        "1 compile error",
    ),
    (
        "\"\\ud800\"",
        &["Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 8 (while parsing '\"\\ud800\"')"],
        "1 compile error",
    ),
    (
        "\"\\(1)\\ud800\"",
        &["Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 8 (while parsing '\"\\ud800\"')"],
        "1 compile error",
    ),
    (
        "\"\\q\\(1)\\q\"",
        &["Invalid escape at line 1, column 4 (while parsing '\"\\q\"')", "Invalid escape at line 1, column 4 (while parsing '\"\\q\"')"],
        "2 compile errors",
    ),
    (
        "\"a\\qb\\rc\\zd\"",
        &["Invalid escape at line 1, column 4 (while parsing '\"\\q\"')", "Invalid escape at line 1, column 4 (while parsing '\"\\z\"')"],
        "2 compile errors",
    ),
    (
        "@base64 \"\\ud800\"",
        &["Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 8 (while parsing '\"\\ud800\"')"],
        "1 compile error",
    ),
    (
        "{\"\\ud800\":1}",
        &["Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 8 (while parsing '\"\\ud800\"')"],
        "1 compile error",
    ),
    (
        ".\"\\ud800\"",
        &["Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 8 (while parsing '\"\\ud800\"')"],
        "1 compile error",
    ),
    (
        "\"\\u\"",
        &["Invalid \\uXXXX escape at line 1, column 4 (while parsing '\"\\u\"')"],
        "1 compile error",
    ),
    (
        "\"\\u12g4\"",
        &["Invalid characters in \\uXXXX escape at line 1, column 8 (while parsing '\"\\u12g4\"')"],
        "1 compile error",
    ),
    (
        "\"\\ud800\\udc0\"",
        &["Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 13 (while parsing '\"\\ud800\\udc0\"')"],
        "1 compile error",
    ),
    (
        "\"\\uzzzz\"",
        &["Invalid characters in \\uXXXX escape at line 1, column 8 (while parsing '\"\\uzzzz\"')"],
        "1 compile error",
    ),
    (
        "\"\\x\"",
        &["Invalid escape at line 1, column 4 (while parsing '\"\\x\"')"],
        "1 compile error",
    ),
    (
        "\"\\ud800\\q\"",
        &["Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 10 (while parsing '\"\\ud800\\q\"')"],
        "1 compile error",
    ),
    (
        "{\"a\":\"\\q\"}",
        &["Invalid escape at line 1, column 4 (while parsing '\"\\q\"')"],
        "1 compile error",
    ),
    (
        "{\"\\q\":1}",
        &["Invalid escape at line 1, column 4 (while parsing '\"\\q\"')"],
        "1 compile error",
    ),
    (
        ".\"\\q\"",
        &["Invalid escape at line 1, column 4 (while parsing '\"\\q\"')"],
        "1 compile error",
    ),
    (
        ".[\"\\q\"]",
        &["Invalid escape at line 1, column 4 (while parsing '\"\\q\"')"],
        "1 compile error",
    ),
    (
        "include \"\\ \"; 0",
        &["Invalid escape at line 1, column 4 (while parsing '\"\\ \"')"],
        "1 compile error",
    ),
    (
        "import \"\\q\" as x; 0",
        &["Invalid escape at line 1, column 4 (while parsing '\"\\q\"')"],
        "1 compile error",
    ),
    (
        "\"\\ud800\\u0041\"",
        &["Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 14 (while parsing '\"\\ud800\\u0041\"')"],
        "1 compile error",
    ),
    (
        "\"x\\u00e9\\ud800\"",
        &["Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 14 (while parsing '\"\\u00e9\\ud800\"')"],
        "1 compile error",
    ),
    (
        "\"\\u00e9\\q\\(1)\\u12\\(2)\\ud800\"",
        &["Invalid escape at line 1, column 10 (while parsing '\"\\u00e9\\q\"')", "Invalid \\uXXXX escape at line 1, column 6 (while parsing '\"\\u12\"')", "Invalid \\uXXXX\\uXXXX surrogate pair escape at line 1, column 8 (while parsing '\"\\ud800\"')"],
        "3 compile errors",
    ),
];

#[test]
fn test_bad_escape_in_a_program_string_is_jq_decoders_message_7445() -> Result<()> {
    for (program, messages, trailer) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-n", program], None)?;
        assert_eq!(stdout, "", "{program}");
        assert_eq!(code, 3, "{program}: {stderr}");
        let got: Vec<&str> = stderr
            .lines()
            .filter_map(|line| line.strip_prefix("jq: error: "))
            .filter_map(|line| {
                line.split_once(" at <top-level>")
                    .map(|(message, _)| message)
            })
            .collect();
        assert_eq!(&got, messages, "{program}: {stderr}");
        let last = stderr.lines().last().unwrap_or_default();
        assert_eq!(last, format!("jq: {trailer}"), "{program}");
    }
    Ok(())
}

/// Where the program has a bad escape, the other errors jq reports follow it in the order it
/// meets them: here a syntax error at the end of the program. Only the first line of each is
/// compared, since a syntax error's wording is succinctly's own (the #2703 residual).
#[test]
fn test_bad_escape_then_a_syntax_error_counts_both_7445() -> Result<()> {
    for program in [r#"1 + "\q" +"#, r#""\q" | foo("#, r#"[1,"\q",2"#] {
        let (_, stderr, code) = run_jq_full(&["-n", program], None)?;
        assert_eq!(code, 3, "{program}: {stderr}");
        assert!(
            stderr.starts_with(
                "jq: error: Invalid escape at line 1, column 4 (while parsing '\"\\q\"') at <top-level>"
            ),
            "{program}: {stderr}"
        );
        assert!(
            stderr.ends_with("jq: 2 compile errors\n"),
            "{program}: {stderr}"
        );
    }
    Ok(())
}

/// Escapes that are fine are untouched: a lone low surrogate is U+FFFD, a pair is one character.
#[test]
fn test_good_escapes_in_a_program_string_still_decode_7445() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &[
            "-n",
            "-c",
            r#""\udc00", "\ud83d\ude00", "a\tb\u00e9\(1)\n""#,
        ],
        None,
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "\"\u{fffd}\"\n\"\u{1f600}\"\n\"a\\tb\u{e9}1\\n\"\n");
    Ok(())
}
