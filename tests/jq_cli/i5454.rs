//! #5454: jq's grammar has no slice after a `.` (`Term '.' '[' Exp ']'` takes an index, not a
//! slice) and no empty parameter or argument list. Rows were captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_stdin;

const DOC: &str = r#"{"a":{"b":[1,2]}}"#;

#[test]
fn test_slice_after_a_dot_and_empty_parens_are_syntax_errors_like_jq_5454() -> Result<()> {
    for filter in [
        "[1,2,3].[1:]",
        "\"abc\".[:2]",
        ".a.b.[1:]",
        ". .[1:]",
        ".a.[1:]",
        "[1,2,3].[1:]?",
        "def f(): 1; f",
        "def f: 1; f()",
        "def f: 1; f ()",
        "def f(a): a; f()",
    ] {
        let (stdout, code) = run_jq_stdin(filter, DOC, &["-c"])?;
        assert_eq!(stdout, "", "`{filter}`");
        assert_eq!(code, 3, "`{filter}` must be a compile error");
    }
    Ok(())
}

#[test]
fn test_neighbouring_forms_still_parse_5454() -> Result<()> {
    for (filter, want) in [
        ("[1,2,3].[1]", "2\n"),
        ("[1,2,3].[]", "1\n2\n3\n"),
        (".a.b[1:]", "[2]\n"),
        (".a.b | .[1:]", "[2]\n"),
        (".a.b.[0]", "1\n"),
        ("def f: 1; f", "1\n"),
        ("def f(a): a; f(1)", "1\n"),
        ("[1]|map ( . )", "[1]\n"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, DOC, &["-c"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`");
    }
    Ok(())
}
