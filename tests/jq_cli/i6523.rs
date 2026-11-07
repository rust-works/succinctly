//! #6523: `implode` of an array that an earlier stage built (`explode | implode`, `explode | map(. + 1) |
//! implode`, `[.[] | explode | implode]`) was written out as JSON text, indexed and read back one number at a
//! time by the reindex bridge: `.strings | map(explode | implode)` over a 9 MB file took 921 ms against jq's
//! 187 ms. It is answered from the owned array now. Every row was captured from jq 1.7.1; a codepoint that
//! `char::from_u32` rejects (a surrogate, past U+10FFFF, negative, outside `u32`) is U+FFFD, a fractional one
//! truncates, and anything that is not a number is jq's own error, all of which the bridge still answers.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn implode_of_an_array_a_stage_built_is_the_string_jq_answers_6523() -> Result<()> {
    for (filter, input, expected) in [
        (
            "explode | implode",
            "\"héllo 🌍 日本語\"",
            "\"héllo 🌍 日本語\"",
        ),
        ("explode | implode", "\"\"", "\"\""),
        ("explode | map(. + 1) | implode", "\"HAL\"", "\"IBM\""),
        ("explode | map(. - 1) | implode", "\"IBM\"", "\"HAL\""),
        (
            "explode | reverse | implode",
            "\"stressed\"",
            "\"desserts\"",
        ),
        (
            "explode | map(select(. != 97)) | implode",
            "\"banana\"",
            "\"bnn\"",
        ),
        (
            "[.[] | explode | implode]",
            "[\"a\",\"bc\",\"\",\"é\"]",
            "[\"a\",\"bc\",\"\",\"é\"]",
        ),
        (
            "map(explode | map(. + 1) | implode)",
            "[\"HAL\",\"abc\"]",
            "[\"IBM\",\"bcd\"]",
        ),
        (
            "explode | map(. + 55296) | implode",
            "\"\\u0000\\u0001\"",
            "\"\u{fffd}\u{fffd}\"",
        ),
        (
            "explode | map(. + 1114112) | implode",
            "\"a\"",
            "\"\u{fffd}\"",
        ),
        (
            "explode | map(. - 98) | implode",
            "\"abc\"",
            "\"\u{fffd}\\u0000\\u0001\"",
        ),
        (
            "explode | map(. * 4294967296 + 65) | implode",
            "\"a\"",
            "\"\u{fffd}\"",
        ),
        ("explode | map(. + 0.5) | implode", "\"abc\"", "\"abc\""),
        ("explode | map(. / 1) | implode", "\"abc\"", "\"abc\""),
        (
            "try (explode | map(tostring) | implode) catch .",
            "\"a\"",
            "\"string (\\\"97\\\") can't be imploded, unicode codepoint needs to be numeric\"",
        ),
        ("[explode | map(. + 1) | implode?]", "\"a\"", "[\"b\"]"),
        ("explode | . + [128512] | implode", "\"x\"", "\"x😀\""),
        ("explode | implode | explode | implode", "\"ab\"", "\"ab\""),
        ("(explode | implode) as $s | $s | length", "\"héllo\"", "5"),
        ("explode | implode | ascii_upcase", "\"abc\"", "\"ABC\""),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(stdout, format!("{expected}\n"), "{filter} on {input}");
        assert_eq!(stderr, "", "{filter} on {input}");
        assert_eq!(code, 0, "{filter} on {input}");
    }
    Ok(())
}

#[test]
fn implode_of_a_non_number_element_keeps_jqs_error_6523() -> Result<()> {
    for (filter, input, message) in [
        (
            "explode | map(tostring) | implode",
            "\"a\"",
            "string (\"97\") can't be imploded, unicode codepoint needs to be numeric",
        ),
        (
            "explode | map(null) | implode",
            "\"a\"",
            "null (null) can't be imploded, unicode codepoint needs to be numeric",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&[filter], Some(input))?;
        assert_eq!(stdout, "", "{filter} on {input}");
        assert_eq!(
            stderr,
            format!("jq: error (at <stdin>:0): {message}\n"),
            "{filter} on {input}"
        );
        assert_eq!(code, 5, "{filter} on {input}");
    }
    Ok(())
}
