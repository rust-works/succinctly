//! #7521: `tonumber` refuses a string with a NUL byte in it, as jq 1.8.2 does (jqlang/jq#3496,
//! listed in the 1.8 NEWS under changes to existing functions: "Fix `tonumber` and `toboolean` to
//! reject strings with embedded null bytes"). jq 1.7.1 hands its number parser a C string, so it
//! stops at the NUL and answers the prefix (`"1\u00002" | tonumber` is `1`). The refusal is the
//! 1.8.2 wording, `string ("1\u00002") cannot be parsed as a number`, for every string with a NUL,
//! including one jq 1.7.1 also refuses (`"a\u0000b"`, which it words as a parse fault in `a`). The
//! difference from the pinned 1.7.1 is recorded in the limitations page. Every row below was
//! captured from jq 1.8.2 (`/opt/homebrew/bin/jq -c`).

use anyhow::Result;

use crate::run_jq_full;

/// (input, filter, expected stdout, expected message in stderr, exit code).
const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    (
        "\"1\\u00002\"",
        "tonumber",
        "",
        "string (\"1\\u00002\") cannot be parsed as a number",
        5,
    ),
    (
        "\"1\\u0000\"",
        "tonumber",
        "",
        "string (\"1\\u0000\") cannot be parsed as a number",
        5,
    ),
    (
        "\"12\\u0000abc\"",
        "tonumber",
        "",
        "string (\"12\\u0000abc\") cannot be parsed as a number",
        5,
    ),
    (
        "\"\\u0000\"",
        "tonumber",
        "",
        "string (\"\\u0000\") cannot be parsed as a number",
        5,
    ),
    (
        "\"\\u00001\"",
        "tonumber",
        "",
        "string (\"\\u00001\") cannot be parsed as a number",
        5,
    ),
    (
        "\" 1\\u0000\"",
        "tonumber",
        "",
        "string (\" 1\\u0000\") cannot be parsed as a number",
        5,
    ),
    (
        "\"a\\u0000b\"",
        "tonumber",
        "",
        "string (\"a\\u0000b\") cannot be parsed as a number",
        5,
    ),
    (
        "\"x\\u0000 y\"",
        "tonumber",
        "",
        "string (\"x\\u0000 y\") cannot be parsed as a number",
        5,
    ),
    (
        "\"é\\u00001\"",
        "tonumber",
        "",
        "string (\"é\\u00001\") cannot be parsed as a number",
        5,
    ),
    ("\"1\\u0000\"", "[tonumber?]", "[]\n", "", 0),
    (
        "\"1\\u0000\"",
        "try tonumber catch .",
        "\"string (\\\"1\\\\u0000\\\") cannot be parsed as a number\"\n",
        "",
        0,
    ),
    ("[\"1\\u0000\",\"2\"]", "map(tonumber?)", "[2]\n", "", 0),
    (
        "[\"1\\u0000\",\"2\"]",
        "map(try tonumber catch \"E\")",
        "[\"E\",2]\n",
        "",
        0,
    ),
    (
        "{\"a\":\"1\\u0000x\"}",
        ".a |= tonumber",
        "",
        "string (\"1\\u0000x\") cannot be parsed as a number",
        5,
    ),
    (
        "{\"a\":\"1\\u0000x\"}",
        "try (.a | tonumber) catch .",
        "\"string (\\\"1\\\\u0000x\\\") cannot be parsed as a number\"\n",
        "",
        0,
    ),
    ("\"1\"", "tonumber", "1\n", "", 0),
];

#[test]
fn tonumber_refuses_a_string_with_a_nul_7521() -> Result<()> {
    for (input, filter, stdout, message, code) in ROWS {
        let (out, err, status) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(&out, stdout, "{input} | {filter}: stderr: {err}");
        assert_eq!(status, *code, "{input} | {filter}: stderr: {err}");
        assert!(err.contains(message), "{input} | {filter}: stderr: {err}");
    }
    Ok(())
}
