//! #5651: yq reads a string that parses as a timestamp (RFC 3339 or `2006-01-02`, as Go's
//! `time.Parse` reads them) on the left of `<`, `<=`, `>` or `>=` as a time, and the right operand
//! the same way: two timestamps compare as instants (the same moment in two zones is equal), and a
//! right operand that is not one -- a number, a bool, `null`, or text in another layout -- raises
//! Go's own `parsing time ...` error. `==` and `!=` stay textual. Every row was captured from yq
//! v4.53.3. Not covered: an unquoted `!!timestamp` document value whose text no layout reads
//! (`2001-01-01 10:00:00`), a `!!str` on the left of a `!!timestamp`, and the spelling of a `null`
//! (`~`) in the error, which need a timestamp kind that `OwnedValue` does not have.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "a: 2001-01-01\nb: 2002-01-01T00:00:00Z\nc: \"2001-01-01\"\ne: 5\ny: 2001-01-01T00:00:00Z\nz: 2001-01-01T10:00:00+10:00\nx: \"2001-01-01T00:00:00\"\nf: 1.5\n";

/// `(filter, Ok(stdout) | Err(the text yq prints after "Error: "))`.
const ROWS: &[(&str, Result<&str, &str>)] = &[
    (".a < .b", Ok("true\n")),
    (".b < .a", Ok("false\n")),
    (".a <= .b", Ok("true\n")),
    (".b >= .a", Ok("true\n")),
    (".a > .b", Ok("false\n")),
    (
        ".a < 1",
        Err("parsing time \"1\" as \"2006-01-02\": cannot parse \"1\" as \"2006\""),
    ),
    (
        ".a > 1",
        Err("parsing time \"1\" as \"2006-01-02\": cannot parse \"1\" as \"2006\""),
    ),
    (
        ".a <= 1.5",
        Err("parsing time \"1.5\" as \"2006-01-02\": cannot parse \"1.5\" as \"2006\""),
    ),
    (
        ".a < .f",
        Err("parsing time \"1.5\" as \"2006-01-02\": cannot parse \"1.5\" as \"2006\""),
    ),
    (
        ".c < 1",
        Err("parsing time \"1\" as \"2006-01-02\": cannot parse \"1\" as \"2006\""),
    ),
    (
        ".c < .e",
        Err("parsing time \"5\" as \"2006-01-02\": cannot parse \"5\" as \"2006\""),
    ),
    (".y < .z", Ok("false\n")),
    (".y <= .z", Ok("true\n")),
    (".y >= .z", Ok("true\n")),
    (".y > .z", Ok("false\n")),
    (".y == .z", Ok("false\n")),
    (".y != .z", Ok("true\n")),
    (
        ".a < \"x\"",
        Err("parsing time \"x\" as \"2006-01-02\": cannot parse \"x\" as \"2006\""),
    ),
    (
        ".a < \"\"",
        Err("parsing time \"\" as \"2006-01-02\": cannot parse \"\" as \"2006\""),
    ),
    (
        ".a < null",
        Err("parsing time \"null\" as \"2006-01-02\": cannot parse \"null\" as \"2006\""),
    ),
    (
        ".a < true",
        Err("parsing time \"true\" as \"2006-01-02\": cannot parse \"true\" as \"2006\""),
    ),
    (
        ".a < false",
        Err("parsing time \"false\" as \"2006-01-02\": cannot parse \"false\" as \"2006\""),
    ),
    (
        ".a < .x",
        Err("parsing time \"2001-01-01T00:00:00\": extra text: \"T00:00:00\""),
    ),
    (".e < .a", Err("!!int not yet supported for comparison")),
    (".c < [1]", Err("!!seq () cannot be subtracted from !!str")),
    ("[.a, .b] | .[0] < .[1]", Ok("true\n")),
    ("select(.a < \"2001-06-01\") | .e", Ok("5\n")),
    (
        "select(.a < \"x\") | .e",
        Err("parsing time \"x\" as \"2006-01-02\": cannot parse \"x\" as \"2006\""),
    ),
    (
        "\"2001-01-01\" < 2",
        Err("parsing time \"2\" as \"2006-01-02\": cannot parse \"2\" as \"2006\""),
    ),
    (
        "\"2001-01-01T00:00:00+14:00\" < \"2001-01-01T00:00:00Z\"",
        Ok("true\n"),
    ),
    (
        "\"2001-01-01T00:00:00.5Z\" > \"2001-01-01T00:00:00Z\"",
        Ok("true\n"),
    ),
    (
        "\"2001-01-01T1:00:00Z\" < \"2001-01-01T2:00:00Z\"",
        Ok("true\n"),
    ),
    ("\"2001-02-30\" < \"2001-03-01\"", Ok("true\n")),
    ("\"2001-13-01\" < \"2001-03-01\"", Ok("false\n")),
    (
        "\"2001-01-01\" < \"2001-1-1\"",
        Err("parsing time \"2001-1-1\" as \"2006-01-02\": cannot parse \"1-1\" as \"01\""),
    ),
    (
        ".a < \"20010101\"",
        Err("parsing time \"20010101\" as \"2006-01-02\": cannot parse \"0101\" as \"-\""),
    ),
    (
        ".a < \"2001-01-01T00:00:00\"",
        Err("parsing time \"2001-01-01T00:00:00\": extra text: \"T00:00:00\""),
    ),
];

#[test]
fn test_a_timestamp_string_is_compared_as_a_time_5651() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, DOC, &[])?;
        match expected {
            Ok(want) => assert_eq!((stdout.as_str(), code), (*want, 0), "{filter}: {stderr:?}"),
            Err(message) => {
                assert_eq!(stdout, "", "{filter}");
                assert_eq!(stderr, format!("Error: {message}\n"), "{filter}");
                assert_eq!(code, 1, "{filter}");
            }
        }
    }
    Ok(())
}

/// jq mode orders a string against a number by kind, whatever the string spells.
#[test]
fn test_jq_mode_does_not_read_a_string_as_a_time_5651() -> Result<()> {
    let (stdout, code) = crate::run_jq_stdin(
        "[.a < 1, .a < \"x\", .a < .b]",
        "{\"a\":\"2001-01-01T00:00:00Z\",\"b\":\"2001-01-01T10:00:00+10:00\"}",
        &["-c"],
    )?;
    assert_eq!((stdout.as_str(), code), ("[false,true,true]\n", 0));
    Ok(())
}
