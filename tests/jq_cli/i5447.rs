//! #5447: in jq a `.` is a token of its own and `.name` is another, so whitespace after a dot ends
//! the identity. `. .a` is the identity with a `.a` suffix; `. a` and `.a. b` are syntax errors.
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_stdin;

const DOC: &str = r#"{"a":{"b":[1,2]}}"#;

#[test]
fn test_whitespace_after_a_dot_matches_jq_5447() -> Result<()> {
    for (filter, want) in [
        (". .a", "{\"b\":[1,2]}\n"),
        (". .\"a\"", "{\"b\":[1,2]}\n"),
        (". .[\"a\"]", "{\"b\":[1,2]}\n"),
        (". .a.b", "[1,2]\n"),
        (". .a .b", "[1,2]\n"),
        (".a .b", "[1,2]\n"),
        (".a . \"b\"", "[1,2]\n"),
        (".a .[\"b\"]", "[1,2]\n"),
        (".  # c\n.a", "{\"b\":[1,2]}\n"),
        (". | .a", "{\"b\":[1,2]}\n"),
        (". as $x | $x.a", "{\"b\":[1,2]}\n"),
        (". and true", "true\n"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, DOC, &["-c"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`");
    }
    Ok(())
}

#[test]
fn test_name_after_a_gap_is_a_syntax_error_like_jq_5447() -> Result<()> {
    for filter in [
        ". a", ". true", ". null", ". not", ".a. b", ".a . b", ".\na", ". ..",
    ] {
        let (stdout, code) = run_jq_stdin(filter, DOC, &["-c"])?;
        assert_eq!(stdout, "", "`{filter}`");
        assert_ne!(code, 0, "`{filter}` must be rejected");
    }
    Ok(())
}
