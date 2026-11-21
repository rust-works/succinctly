//! #6434, reversed by #7524: when the cut of the value an error message embeds falls inside a
//! multi-byte character, jq 1.7.1 wrote one U+FFFD where the partial character was (#6434 matched
//! that). jq 1.8 fixed it (jqlang/jq#3249, "should not break Unicode characters"): the cut moves back
//! to the character start and nothing replaces it. The budgets stay 1.7.1's (11 bytes, and 26 for
//! `path()`), so each row is the 1.7.1 message minus its U+FFFD (`/usr/bin/jq -c 'try F catch .'`);
//! `tests/jq_cli/i7524.rs` pins jq's own #3249 rows. Pin moved: every expected string below.

use anyhow::Result;

use crate::run_jq_full;

fn caught(filter: &str, input: &str) -> Result<String> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", &format!("try {filter} catch .")], Some(input))?;
    assert_eq!((stderr.as_str(), code), ("", 0), "{filter} on {input}");
    Ok(stdout.trim_end().to_owned())
}

#[test]
fn a_cut_that_splits_a_character_backs_up_to_its_start_7524() -> Result<()> {
    for (filter, input, message) in [
        // One, two or three bytes of a 4-byte character kept.
        (
            "(.[])",
            r#""a😀😀😀""#,
            r#"Cannot iterate over string (\"a😀😀...)"#,
        ),
        (
            "(.[])",
            r#""abcd😀😀😀""#,
            r#"Cannot iterate over string (\"abcd😀...)"#,
        ),
        (
            "(.[])",
            r#""abc😀😀😀""#,
            r#"Cannot iterate over string (\"abc😀...)"#,
        ),
        // Of a 3-byte character.
        (
            "(.[])",
            r#""あああああ""#,
            r#"Cannot iterate over string (\"あああ...)"#,
        ),
        (
            "(.[])",
            r#""abcdefgh€€""#,
            r#"Cannot iterate over string (\"abcdefgh...)"#,
        ),
        (
            "(. + 1)",
            r#"{"b":{"":"😀"}}"#,
            r#"object ({\"b\":{\"\":\"...) and number (1) cannot be added"#,
        ),
        // The wide budget of `path()`'s own message.
        (
            "path(tostring)",
            r#"["€€€€€€€€€€€€"]"#,
            r#"Invalid path expression with result \"[\\\"€€€€€€€..."#,
        ),
        (
            "path(tostring)",
            r#"["😀😀😀😀😀😀😀😀"]"#,
            r#"Invalid path expression with result \"[\\\"😀😀😀😀😀..."#,
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
fn the_uncaught_message_is_cut_the_same_way_7524() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", ".[]"], Some("\"abc😀😀😀\"\n"))?;
    assert_eq!((stdout.as_str(), code), ("", 5));
    assert_eq!(
        stderr.trim_end(),
        "jq: error (at <stdin>:1): Cannot iterate over string (\"abc😀...)"
    );
    Ok(())
}
