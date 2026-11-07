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

/// The array `implode` reads is not always a stage's own: it can be bound (`. as $a | $a | implode`), a
/// fold's state, an update's right side or a path-tracked value, where the embed table or the path
/// resolver decide what may be answered without the bridge. Every row was captured from jq 1.7.1.
#[test]
fn implode_over_a_bound_folded_updated_or_path_tracked_array_is_jqs_answer_6523() -> Result<()> {
    for (filter, input, expected) in [
        (
            ". as $a | [$a, $a] | map(implode)",
            "[97,98]",
            "[\"ab\",\"ab\"]",
        ),
        (
            "reduce (1,2) as $i ([]; . + [96 + $i]) | implode",
            "null",
            "\"ab\"",
        ),
        (
            "[foreach (1,2,3) as $i ([]; . + [96 + $i]; implode)]",
            "null",
            "[\"a\",\"ab\",\"abc\"]",
        ),
        (
            ".a |= (explode | map(. + 1) | implode)",
            "{\"a\":\"abc\"}",
            "{\"a\":\"bcd\"}",
        ),
        (
            ".[] |= (explode | implode)",
            "[\"ab\",\"cd\"]",
            "[\"ab\",\"cd\"]",
        ),
        ("def f: explode | implode; f", "\"héllo\"", "\"héllo\""),
        ("[explode | implode?]", "\"ab\"", "[\"ab\"]"),
        (
            "try path(explode | implode) catch .",
            "\"ab\"",
            "\"Invalid path expression with result \\\"ab\\\"\"",
        ),
        (
            "try del(explode | implode) catch .",
            "\"ab\"",
            "\"Invalid path expression with result \\\"ab\\\"\"",
        ),
        (
            ".[0] as $a | ($a | explode) as $e | $e | implode",
            "[\"ab\"]",
            "\"ab\"",
        ),
        (
            "[.[] | select(explode | implode | length > 1)]",
            "[\"ab\",\"c\"]",
            "[\"ab\"]",
        ),
        ("[1e1000] | implode", "null", "\"\u{fffd}\""),
        ("[65.9] | implode", "null", "\"A\""),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(stdout, format!("{expected}\n"), "{filter} on {input}");
        assert_eq!(stderr, "", "{filter} on {input}");
        assert_eq!(code, 0, "{filter} on {input}");
    }
    Ok(())
}
