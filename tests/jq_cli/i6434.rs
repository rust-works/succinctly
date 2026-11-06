//! #6434: when jq cuts the value an error message embeds in the middle of a multi-byte character,
//! the message holds one U+FFFD where the partial character was, at the narrow budget (11 bytes)
//! and at the wide one `path()` uses (26). Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -c 'try F catch .'`).

use anyhow::Result;

use crate::run_jq_full;

fn caught(filter: &str, input: &str) -> Result<String> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", &format!("try {filter} catch .")], Some(input))?;
    assert_eq!((stderr.as_str(), code), ("", 0), "{filter} on {input}");
    Ok(stdout.trim_end().to_owned())
}

#[test]
fn a_cut_that_splits_a_character_writes_one_replacement_6434() -> Result<()> {
    for (filter, input, message) in [
        // One, two or three bytes of a 4-byte character kept.
        (
            "(.[])",
            r#""a😀😀😀""#,
            r#"Cannot iterate over string (\"a😀😀�...)"#,
        ),
        (
            "(.[])",
            r#""abcd😀😀😀""#,
            r#"Cannot iterate over string (\"abcd😀�...)"#,
        ),
        (
            "(.[])",
            r#""abc😀😀😀""#,
            r#"Cannot iterate over string (\"abc😀�...)"#,
        ),
        // Of a 3-byte character.
        (
            "(.[])",
            r#""あああああ""#,
            r#"Cannot iterate over string (\"あああ�...)"#,
        ),
        (
            "(.[])",
            r#""abcdefgh€€""#,
            r#"Cannot iterate over string (\"abcdefgh�...)"#,
        ),
        (
            "(. + 1)",
            r#"{"b":{"":"😀"}}"#,
            r#"object ({\"b\":{\"\":\"�...) and number (1) cannot be added"#,
        ),
        // The wide budget of `path()`'s own message.
        (
            "path(tostring)",
            r#"["€€€€€€€€€€€€"]"#,
            r#"Invalid path expression with result \"[\\\"€€€€€€€�..."#,
        ),
        (
            "path(tostring)",
            r#"["😀😀😀😀😀😀😀😀"]"#,
            r#"Invalid path expression with result \"[\\\"😀😀😀😀😀�..."#,
        ),
    ] {
        assert_eq!(
            caught(filter, input)?,
            format!("\"{message}\""),
            "{filter} on {input}"
        );
    }
    Ok(())
}

#[test]
fn a_cut_on_a_character_boundary_writes_none_6434() -> Result<()> {
    for (filter, input, message) in [
        (
            "(.[])",
            r#""ab😀😀😀""#,
            r#"Cannot iterate over string (\"ab😀😀...)"#,
        ),
        (
            "(.[])",
            r#""ααααααααα""#,
            r#"Cannot iterate over string (\"ααααα...)"#,
        ),
        (
            "path(tostring)",
            r#"["a€€€€€€€€€€€€"]"#,
            r#"Invalid path expression with result \"[\\\"a€€€€€€€..."#,
        ),
    ] {
        assert_eq!(
            caught(filter, input)?,
            format!("\"{message}\""),
            "{filter} on {input}"
        );
    }
    Ok(())
}

/// Without `try`, stderr has the same message.
#[test]
fn the_uncaught_message_has_the_replacement_too_6434() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", ".[]"], Some("\"abc😀😀😀\"\n"))?;
    assert_eq!((stdout.as_str(), code), ("", 5));
    assert_eq!(
        stderr.trim_end(),
        "jq: error (at <stdin>:1): Cannot iterate over string (\"abc😀�...)"
    );
    Ok(())
}
