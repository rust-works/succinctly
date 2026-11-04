//! #6229: a malformed character class is refused with Oniguruma's own wording: `empty char-class`
//! for a class that opens with a `]` and has no other `]` to close it, `premature end of
//! char-class` for any other class the pattern ends inside, and `end pattern at escape` for a
//! trailing backslash. Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`), which words them
//! `Regex failure: <message>`; this tool words them `invalid regex: <message>`.

use anyhow::Result;

use crate::run_jq_full;

fn refusal_of(pattern: &str, flags: Option<&str>) -> Result<(String, i32)> {
    let quoted = serde_json::to_string(pattern)?;
    let filter = match flags {
        Some(flags) => format!(r#""a]" | test({quoted}; "{flags}")"#),
        None => format!(r#""a]" | test({quoted})"#),
    };
    let (stdout, stderr, code) = run_jq_full(&["-nc", &filter], None)?;
    assert_eq!(stdout.trim_end(), "", "{pattern}");
    Ok((stderr, code))
}

#[test]
fn a_class_that_opens_with_a_close_bracket_and_never_closes_is_empty_6229() -> Result<()> {
    for pattern in [
        "[]", "[^]", "a[]", "[]a", "[^]a", "(a[])", r"[]\\", r"x[^]\",
    ] {
        let (stderr, code) = refusal_of(pattern, None)?;
        assert_eq!(code, 5, "{pattern}: {stderr:?}");
        assert!(
            stderr.contains("invalid regex: empty char-class"),
            "{pattern}: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn a_class_the_pattern_ends_inside_is_premature_6229() -> Result<()> {
    for pattern in [
        "[",
        "[a",
        "[^",
        "a[b",
        "[a-",
        "[a&&",
        "[[:alpha:]",
        "(a[)",
        r"[\]",
        r"[^\]",
        r"[a\]",
        r"[]\]",
        r"[^]\]x",
        "[]a]b[c",
        "[[a]b[",
    ] {
        let (stderr, code) = refusal_of(pattern, None)?;
        assert_eq!(code, 5, "{pattern}: {stderr:?}");
        assert!(
            stderr.contains("invalid regex: premature end of char-class"),
            "{pattern}: {stderr:?}"
        );
    }
    Ok(())
}

/// Under the `x` flag a blank inside the class is a member, so the class is still open.
#[test]
fn an_unterminated_class_is_refused_under_the_extended_flag_6229() -> Result<()> {
    let (stderr, code) = refusal_of("[a ", Some("x"))?;
    assert_eq!(code, 5, "{stderr:?}");
    assert!(stderr.contains("premature end of char-class"), "{stderr:?}");
    Ok(())
}

#[test]
fn a_trailing_backslash_escapes_nothing_6229() -> Result<()> {
    for pattern in [r"a\", r"[\", r"(a\"] {
        let (stderr, code) = refusal_of(pattern, None)?;
        assert_eq!(code, 5, "{pattern}: {stderr:?}");
        assert!(
            stderr.contains("invalid regex: end pattern at escape"),
            "{pattern}: {stderr:?}"
        );
    }
    Ok(())
}

/// A `]` first in a class is a member when another `]` follows, and a `[` inside one is a literal.
#[test]
fn well_formed_classes_still_match_6229() -> Result<()> {
    for (pattern, expected) in [
        ("[]a]", r#"["a","]"]"#),
        ("[]]", r#"["]"]"#),
        ("[^]a]", "[]"),
        ("[^]]", r#"["a"]"#),
        (r"[]\\]", r#"["]"]"#),
        (r"[\]]", r#"["]"]"#),
        ("[[a]", r#"["a"]"#),
        ("[[]", "[]"),
        ("[a[]", r#"["a"]"#),
        ("x[]y]z", "[]"),
        (r"\Q[\E", "[]"),
    ] {
        let filter = format!(
            r#""a]" | [match({};"g")|.string]"#,
            serde_json::to_string(pattern)?
        );
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{pattern}: {stderr:?}");
        assert_eq!(code, 0, "{pattern}");
    }
    Ok(())
}

/// An error earlier in the pattern is the one reported, as in jq.
#[test]
fn an_earlier_refusal_stays_first_6229() -> Result<()> {
    for (pattern, message) in [
        ("[[:foo:][", "invalid POSIX bracket type"),
        ("(?P<n>a)[", "undefined group option"),
        (r"[]\p{Foo}", "empty char-class"),
    ] {
        let (stderr, code) = refusal_of(pattern, None)?;
        assert_eq!(code, 5, "{pattern}: {stderr:?}");
        assert!(stderr.contains(message), "{pattern}: {stderr:?}");
    }
    Ok(())
}
