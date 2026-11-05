//! #6265: a literal-bound slice of a string read from the document is cut where it stands (jq mode)
//! instead of copied whole by the bridge first. The answers must not move: every row was captured from
//! jq 1.7.1 (`/usr/bin/jq -c`).

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn a_literal_slice_of_a_document_string_answers_as_jq_does_6265() -> Result<()> {
    for (input, filter, expected) in [
        (
            r#""héllo é 😀 x \u0000 y""#,
            r".[1:3], .[:2], .[-2:], .[0:], .[5:1], .[7:9], .[100:]",
            r#""él"
"hé"
" y"
"héllo é 😀 x \u0000 y"
""
" 😀"
""
"#,
        ),
        (
            r#""plain ascii text""#,
            r".[1:3], .[-4:], .[:0], .[3:3], .[0:]",
            r#""la"
"text"
""
""
"plain ascii text"
"#,
        ),
        (
            r#""""#,
            r".[1:3], .[0:], .[:1]",
            r#"""
""
""
"#,
        ),
        (
            r#"["abc","déf\n\"x"]"#,
            r".[1][1:3], .[0][:2], [.[]|.[1:]]",
            r#""éf"
"ab"
["bc","éf\n\"x"]
"#,
        ),
        (
            r#"{"a":"abcdef","b":"😀😀z"}"#,
            r".a[1:3], .b[1:2], .b[-1:]",
            r#""bc"
"😀"
"z"
"#,
        ),
        (
            r#""abcdef""#,
            r#". as $s | $s[1:3], ($s|.[2:]), (.[1:3]?), (try .[1:3] catch "e")"#,
            r#""bc"
"cdef"
"bc"
"bc"
"#,
        ),
        (
            r#""ab""#,
            r".[1:3], .[-100:100]",
            r#""b"
"ab"
"#,
        ),
    ] {
        let (stdout, code) = run_jq_stdin(filter, input, &["-c"])?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, 0),
            "{filter} on {input}"
        );
    }
    Ok(())
}

/// A string, or the text around one, that is malformed: the slice must raise (or not) as it did through
/// the bridge, and as jq does. Each row is `(input, filter, stdout, exit code)` from jq 1.7.1.
#[test]
fn a_literal_slice_of_a_malformed_string_raises_as_before_6265() -> Result<()> {
    for (input, filter, expected, expected_code) in [
        (r#""abc"#, r".[1:3]", r"", 5),
        (r#""abc"#, r#"try .[1:3] catch "e""#, r"", 5),
        (r#""a\qb""#, r".[1:3]", r"", 5),
        (r#""a\ud800b""#, r".[1:3]", r"", 5),
        (r#""a\u12""#, r".[1:3]", r"", 5),
        (r#"["abc" "def"]"#, r".[0][1:3]", r"", 5),
        (r#"{"a":"abcdef""#, r".a[1:3]", r"", 5),
        (
            r#""abc" x"#,
            r".[1:3]",
            r#""bc"
"#,
            5,
        ),
        (
            r#""a	b
c""#,
            r".[1:3]",
            r"",
            5,
        ),
        (
            r#""abc"]"#,
            r".[1:3]",
            r#""bc"
"#,
            5,
        ),
    ] {
        let (stdout, code) = run_jq_stdin(filter, input, &["-c"])?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, expected_code),
            "{filter} on {input}"
        );
    }
    Ok(())
}
