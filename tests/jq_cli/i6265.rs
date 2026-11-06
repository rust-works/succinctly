//! #6265: a literal-bound slice of a string read from the document is cut where it stands (jq mode)
//! instead of copied whole by the bridge first. The answers must not move: every row was captured from
//! jq 1.7.1 (`/usr/bin/jq -c`).

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin};

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

/// The answer over every kind of target, with a later stage and under `?` and `try`, as stdout,
/// stderr and exit status. Generated from `/usr/bin/jq -c`, 1.7.1.
#[rustfmt::skip]
const PATH_OF_A_NARROW_SLICE: &[(&str, &str, &str, &str, i32)] = &[
    ("[1,2,3]", "path(.[1:2])", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("[1,2,3]", "path(.[:1])", "[{\"start\":null,\"end\":1}]\n", "", 0),
    ("[1,2,3]", "path(.[-1:])", "[{\"start\":-1,\"end\":null}]\n", "", 0),
    ("[1,2,3]", "path(.[1.5:2.5])", "[{\"start\":1.5,\"end\":2.5}]\n", "", 0),
    ("[1,2,3]", "path(.[null:1])", "[{\"start\":null,\"end\":1}]\n", "", 0),
    ("[1,2,3]", "path(.[1:2]?)", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("[1,2,3]", "path(try .[1:2])", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("[1,2,3]", "path(.[1:2][0])", "[{\"start\":1,\"end\":2},0]\n", "", 0),
    ("[1,2,3]", "path(.[1:2]|.[0])", "[{\"start\":1,\"end\":2},0]\n", "", 0),
    ("[1,2,3]", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
    ("[1,2,3]", "path(.[0:1][])", "[{\"start\":0,\"end\":1},0]\n", "", 0),
    ("[1,2,3]", "path(.[0:1][]?)", "[{\"start\":0,\"end\":1},0]\n", "", 0),
    ("[1,2,3]", "path(.[0:1][0:1])", "[{\"start\":0,\"end\":1},{\"start\":0,\"end\":1}]\n", "", 0),
    ("[1,2,3]", "path(.[5:6][0])", "[{\"start\":5,\"end\":6},0]\n", "", 0),
    ("[1,2,3]", "path(.a[:1])", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
    ("[1,2,3]", "path(.a[:1]?)", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
    ("[1,2,3]", "path(.a[:1][0])", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
    ("[1,2,3]", "[path(.[0],.[1:2][0]?,.[1])]", "[[0],[{\"start\":1,\"end\":2},0],[1]]\n", "", 0),
    ("[1,2,3]", "path(.[0:1]|.[0]?)", "[{\"start\":0,\"end\":1},0]\n", "", 0),
    ("[1,2,3]", "[path(.[0:1],.[1:2])]", "[[{\"start\":0,\"end\":1}],[{\"start\":1,\"end\":2}]]\n", "", 0),
    ("[1,2,3]", "first(path(.[0:1],error(\"x\")))", "[{\"start\":0,\"end\":1}]\n", "", 0),
    ("[1,2,3]", "[limit(1;path(.[0:1],.[1:2]))]", "[[{\"start\":0,\"end\":1}]]\n", "", 0),
    ("[1,2,3]", "path(.[]?|.[0:1]?)", "", "", 0),
    ("[1,2,3]", ".[0:1] as $x|path(.[0:1])", "[{\"start\":0,\"end\":1}]\n", "", 0),
    ("\"abc\"", "path(.[1:2])", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("\"abc\"", "path(.[:1])", "[{\"start\":null,\"end\":1}]\n", "", 0),
    ("\"abc\"", "path(.[-1:])", "[{\"start\":-1,\"end\":null}]\n", "", 0),
    ("\"abc\"", "path(.[1.5:2.5])", "[{\"start\":1.5,\"end\":2.5}]\n", "", 0),
    ("\"abc\"", "path(.[null:1])", "[{\"start\":null,\"end\":1}]\n", "", 0),
    ("\"abc\"", "path(.[1:2]?)", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("\"abc\"", "path(try .[1:2])", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("\"abc\"", "path(.[1:2][0])", "", "jq: error (at <stdin>:0): Cannot index string with number\n", 5),
    ("\"abc\"", "path(.[1:2]|.[0])", "", "jq: error (at <stdin>:0): Cannot index string with number\n", 5),
    ("\"abc\"", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index string with string \"a\"\n", 5),
    ("\"abc\"", "path(.[0:1][])", "", "jq: error (at <stdin>:0): Cannot iterate over string (\"a\")\n", 5),
    ("\"abc\"", "path(.[0:1][]?)", "", "", 0),
    ("\"abc\"", "path(.[0:1][0:1])", "[{\"start\":0,\"end\":1},{\"start\":0,\"end\":1}]\n", "", 0),
    ("\"abc\"", "path(.[5:6][0])", "", "jq: error (at <stdin>:0): Cannot index string with number\n", 5),
    ("\"abc\"", "path(.a[:1])", "", "jq: error (at <stdin>:0): Cannot index string with string \"a\"\n", 5),
    ("\"abc\"", "path(.a[:1]?)", "", "jq: error (at <stdin>:0): Cannot index string with string \"a\"\n", 5),
    ("\"abc\"", "path(.a[:1][0])", "", "jq: error (at <stdin>:0): Cannot index string with string \"a\"\n", 5),
    ("\"abc\"", "[path(.[0],.[1:2][0]?,.[1])]", "", "jq: error (at <stdin>:0): Cannot index string with number\n", 5),
    ("\"abc\"", "path(.[0:1]|.[0]?)", "", "", 0),
    ("\"abc\"", "[path(.[0:1],.[1:2])]", "[[{\"start\":0,\"end\":1}],[{\"start\":1,\"end\":2}]]\n", "", 0),
    ("\"abc\"", "first(path(.[0:1],error(\"x\")))", "[{\"start\":0,\"end\":1}]\n", "", 0),
    ("\"abc\"", "[limit(1;path(.[0:1],.[1:2]))]", "[[{\"start\":0,\"end\":1}]]\n", "", 0),
    ("\"abc\"", "path(.[]?|.[0:1]?)", "", "", 0),
    ("\"abc\"", ".[0:1] as $x|path(.[0:1])", "[{\"start\":0,\"end\":1}]\n", "", 0),
    ("\"héllo\"", "path(.[1:2])", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("\"héllo\"", "path(.[:1])", "[{\"start\":null,\"end\":1}]\n", "", 0),
    ("\"héllo\"", "path(.[-1:])", "[{\"start\":-1,\"end\":null}]\n", "", 0),
    ("\"héllo\"", "path(.[1.5:2.5])", "[{\"start\":1.5,\"end\":2.5}]\n", "", 0),
    ("\"héllo\"", "path(.[null:1])", "[{\"start\":null,\"end\":1}]\n", "", 0),
    ("\"héllo\"", "path(.[1:2]?)", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("\"héllo\"", "path(try .[1:2])", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("\"héllo\"", "path(.[1:2][0])", "", "jq: error (at <stdin>:0): Cannot index string with number\n", 5),
    ("\"héllo\"", "path(.[1:2]|.[0])", "", "jq: error (at <stdin>:0): Cannot index string with number\n", 5),
    ("\"héllo\"", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index string with string \"a\"\n", 5),
    ("\"héllo\"", "path(.[0:1][])", "", "jq: error (at <stdin>:0): Cannot iterate over string (\"h\")\n", 5),
    ("\"héllo\"", "path(.[0:1][]?)", "", "", 0),
    ("\"héllo\"", "path(.[0:1][0:1])", "[{\"start\":0,\"end\":1},{\"start\":0,\"end\":1}]\n", "", 0),
    ("\"héllo\"", "path(.[5:6][0])", "", "jq: error (at <stdin>:0): Cannot index string with number\n", 5),
    ("\"héllo\"", "path(.a[:1])", "", "jq: error (at <stdin>:0): Cannot index string with string \"a\"\n", 5),
    ("\"héllo\"", "path(.a[:1]?)", "", "jq: error (at <stdin>:0): Cannot index string with string \"a\"\n", 5),
    ("\"héllo\"", "path(.a[:1][0])", "", "jq: error (at <stdin>:0): Cannot index string with string \"a\"\n", 5),
    ("\"héllo\"", "[path(.[0],.[1:2][0]?,.[1])]", "", "jq: error (at <stdin>:0): Cannot index string with number\n", 5),
    ("\"héllo\"", "path(.[0:1]|.[0]?)", "", "", 0),
    ("\"héllo\"", "[path(.[0:1],.[1:2])]", "[[{\"start\":0,\"end\":1}],[{\"start\":1,\"end\":2}]]\n", "", 0),
    ("\"héllo\"", "first(path(.[0:1],error(\"x\")))", "[{\"start\":0,\"end\":1}]\n", "", 0),
    ("\"héllo\"", "[limit(1;path(.[0:1],.[1:2]))]", "[[{\"start\":0,\"end\":1}]]\n", "", 0),
    ("\"héllo\"", "path(.[]?|.[0:1]?)", "", "", 0),
    ("\"héllo\"", ".[0:1] as $x|path(.[0:1])", "[{\"start\":0,\"end\":1}]\n", "", 0),
    ("null", "path(.[1:2])", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("null", "path(.[:1])", "[{\"start\":null,\"end\":1}]\n", "", 0),
    ("null", "path(.[-1:])", "[{\"start\":-1,\"end\":null}]\n", "", 0),
    ("null", "path(.[1.5:2.5])", "[{\"start\":1.5,\"end\":2.5}]\n", "", 0),
    ("null", "path(.[null:1])", "[{\"start\":null,\"end\":1}]\n", "", 0),
    ("null", "path(.[1:2]?)", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("null", "path(try .[1:2])", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("null", "path(.[1:2][0])", "[{\"start\":1,\"end\":2},0]\n", "", 0),
    ("null", "path(.[1:2]|.[0])", "[{\"start\":1,\"end\":2},0]\n", "", 0),
    ("null", "path(.[1:2]|.a)", "[{\"start\":1,\"end\":2},\"a\"]\n", "", 0),
    ("null", "path(.[0:1][])", "", "jq: error (at <stdin>:0): Cannot iterate over null (null)\n", 5),
    ("null", "path(.[0:1][]?)", "", "", 0),
    ("null", "path(.[0:1][0:1])", "[{\"start\":0,\"end\":1},{\"start\":0,\"end\":1}]\n", "", 0),
    ("null", "path(.[5:6][0])", "[{\"start\":5,\"end\":6},0]\n", "", 0),
    ("null", "path(.a[:1])", "[\"a\",{\"start\":null,\"end\":1}]\n", "", 0),
    ("null", "path(.a[:1]?)", "[\"a\",{\"start\":null,\"end\":1}]\n", "", 0),
    ("null", "path(.a[:1][0])", "[\"a\",{\"start\":null,\"end\":1},0]\n", "", 0),
    ("null", "[path(.[0],.[1:2][0]?,.[1])]", "[[0],[{\"start\":1,\"end\":2},0],[1]]\n", "", 0),
    ("null", "path(.[0:1]|.[0]?)", "[{\"start\":0,\"end\":1},0]\n", "", 0),
    ("null", "[path(.[0:1],.[1:2])]", "[[{\"start\":0,\"end\":1}],[{\"start\":1,\"end\":2}]]\n", "", 0),
    ("null", "first(path(.[0:1],error(\"x\")))", "[{\"start\":0,\"end\":1}]\n", "", 0),
    ("null", "[limit(1;path(.[0:1],.[1:2]))]", "[[{\"start\":0,\"end\":1}]]\n", "", 0),
    ("null", "path(.[]?|.[0:1]?)", "", "", 0),
    ("null", ".[0:1] as $x|path(.[0:1])", "[{\"start\":0,\"end\":1}]\n", "", 0),
    ("5", "path(.[1:2])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.[:1])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.[-1:])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.[1.5:2.5])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.[null:1])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.[1:2]?)", "", "", 0),
    ("5", "path(try .[1:2])", "", "", 0),
    ("5", "path(.[1:2][0])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.[1:2]|.[0])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.[0:1][])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.[0:1][]?)", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.[0:1][0:1])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.[5:6][0])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.a[:1])", "", "jq: error (at <stdin>:0): Cannot index number with string \"a\"\n", 5),
    ("5", "path(.a[:1]?)", "", "jq: error (at <stdin>:0): Cannot index number with string \"a\"\n", 5),
    ("5", "path(.a[:1][0])", "", "jq: error (at <stdin>:0): Cannot index number with string \"a\"\n", 5),
    ("5", "[path(.[0],.[1:2][0]?,.[1])]", "", "jq: error (at <stdin>:0): Cannot index number with number\n", 5),
    ("5", "path(.[0:1]|.[0]?)", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "[path(.[0:1],.[1:2])]", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "first(path(.[0:1],error(\"x\")))", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "[limit(1;path(.[0:1],.[1:2]))]", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("5", "path(.[]?|.[0:1]?)", "", "", 0),
    ("5", ".[0:1] as $x|path(.[0:1])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("true", "path(.[1:2])", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.[:1])", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.[-1:])", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.[1.5:2.5])", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.[null:1])", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.[1:2]?)", "", "", 0),
    ("true", "path(try .[1:2])", "", "", 0),
    ("true", "path(.[1:2][0])", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.[1:2]|.[0])", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.[0:1][])", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.[0:1][]?)", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.[0:1][0:1])", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.[5:6][0])", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.a[:1])", "", "jq: error (at <stdin>:0): Cannot index boolean with string \"a\"\n", 5),
    ("true", "path(.a[:1]?)", "", "jq: error (at <stdin>:0): Cannot index boolean with string \"a\"\n", 5),
    ("true", "path(.a[:1][0])", "", "jq: error (at <stdin>:0): Cannot index boolean with string \"a\"\n", 5),
    ("true", "[path(.[0],.[1:2][0]?,.[1])]", "", "jq: error (at <stdin>:0): Cannot index boolean with number\n", 5),
    ("true", "path(.[0:1]|.[0]?)", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "[path(.[0:1],.[1:2])]", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "first(path(.[0:1],error(\"x\")))", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "[limit(1;path(.[0:1],.[1:2]))]", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("true", "path(.[]?|.[0:1]?)", "", "", 0),
    ("true", ".[0:1] as $x|path(.[0:1])", "", "jq: error (at <stdin>:0): Cannot index boolean with object\n", 5),
    ("{\"a\":1}", "path(.[1:2])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.[:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.[-1:])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.[1.5:2.5])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.[null:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.[1:2]?)", "", "", 0),
    ("{\"a\":1}", "path(try .[1:2])", "", "", 0),
    ("{\"a\":1}", "path(.[1:2][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.[1:2]|.[0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.[0:1][])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.[0:1][]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.[0:1][0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.[5:6][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.a[:1])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("{\"a\":1}", "path(.a[:1]?)", "", "", 0),
    ("{\"a\":1}", "path(.a[:1][0])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("{\"a\":1}", "[path(.[0],.[1:2][0]?,.[1])]", "", "jq: error (at <stdin>:0): Cannot index object with number\n", 5),
    ("{\"a\":1}", "path(.[0:1]|.[0]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "[path(.[0:1],.[1:2])]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "first(path(.[0:1],error(\"x\")))", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "[limit(1;path(.[0:1],.[1:2]))]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":1}", "path(.[]?|.[0:1]?)", "", "", 0),
    ("{\"a\":1}", ".[0:1] as $x|path(.[0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("[[1,2],[3,4],5]", "path(.[1:2])", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[:1])", "[{\"start\":null,\"end\":1}]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[-1:])", "[{\"start\":-1,\"end\":null}]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[1.5:2.5])", "[{\"start\":1.5,\"end\":2.5}]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[null:1])", "[{\"start\":null,\"end\":1}]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[1:2]?)", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(try .[1:2])", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[1:2][0])", "[{\"start\":1,\"end\":2},0]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[1:2]|.[0])", "[{\"start\":1,\"end\":2},0]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
    ("[[1,2],[3,4],5]", "path(.[0:1][])", "[{\"start\":0,\"end\":1},0]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[0:1][]?)", "[{\"start\":0,\"end\":1},0]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[0:1][0:1])", "[{\"start\":0,\"end\":1},{\"start\":0,\"end\":1}]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[5:6][0])", "[{\"start\":5,\"end\":6},0]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.a[:1])", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
    ("[[1,2],[3,4],5]", "path(.a[:1]?)", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
    ("[[1,2],[3,4],5]", "path(.a[:1][0])", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
    ("[[1,2],[3,4],5]", "[path(.[0],.[1:2][0]?,.[1])]", "[[0],[{\"start\":1,\"end\":2},0],[1]]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[0:1]|.[0]?)", "[{\"start\":0,\"end\":1},0]\n", "", 0),
    ("[[1,2],[3,4],5]", "[path(.[0:1],.[1:2])]", "[[{\"start\":0,\"end\":1}],[{\"start\":1,\"end\":2}]]\n", "", 0),
    ("[[1,2],[3,4],5]", "first(path(.[0:1],error(\"x\")))", "[{\"start\":0,\"end\":1}]\n", "", 0),
    ("[[1,2],[3,4],5]", "[limit(1;path(.[0:1],.[1:2]))]", "[[{\"start\":0,\"end\":1}]]\n", "", 0),
    ("[[1,2],[3,4],5]", "path(.[]?|.[0:1]?)", "[0,{\"start\":0,\"end\":1}]\n[1,{\"start\":0,\"end\":1}]\n", "", 0),
    ("[[1,2],[3,4],5]", ".[0:1] as $x|path(.[0:1])", "[{\"start\":0,\"end\":1}]\n", "", 0),
    ("[]", "path(.[1:2])", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("[]", "path(.[:1])", "[{\"start\":null,\"end\":1}]\n", "", 0),
    ("[]", "path(.[-1:])", "[{\"start\":-1,\"end\":null}]\n", "", 0),
    ("[]", "path(.[1.5:2.5])", "[{\"start\":1.5,\"end\":2.5}]\n", "", 0),
    ("[]", "path(.[null:1])", "[{\"start\":null,\"end\":1}]\n", "", 0),
    ("[]", "path(.[1:2]?)", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("[]", "path(try .[1:2])", "[{\"start\":1,\"end\":2}]\n", "", 0),
    ("[]", "path(.[1:2][0])", "[{\"start\":1,\"end\":2},0]\n", "", 0),
    ("[]", "path(.[1:2]|.[0])", "[{\"start\":1,\"end\":2},0]\n", "", 0),
    ("[]", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
    ("[]", "path(.[0:1][])", "", "", 0),
    ("[]", "path(.[0:1][]?)", "", "", 0),
    ("[]", "path(.[0:1][0:1])", "[{\"start\":0,\"end\":1},{\"start\":0,\"end\":1}]\n", "", 0),
    ("[]", "path(.[5:6][0])", "[{\"start\":5,\"end\":6},0]\n", "", 0),
    ("[]", "path(.a[:1])", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
    ("[]", "path(.a[:1]?)", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
    ("[]", "path(.a[:1][0])", "", "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n", 5),
    ("[]", "[path(.[0],.[1:2][0]?,.[1])]", "[[0],[{\"start\":1,\"end\":2},0],[1]]\n", "", 0),
    ("[]", "path(.[0:1]|.[0]?)", "[{\"start\":0,\"end\":1},0]\n", "", 0),
    ("[]", "[path(.[0:1],.[1:2])]", "[[{\"start\":0,\"end\":1}],[{\"start\":1,\"end\":2}]]\n", "", 0),
    ("[]", "first(path(.[0:1],error(\"x\")))", "[{\"start\":0,\"end\":1}]\n", "", 0),
    ("[]", "[limit(1;path(.[0:1],.[1:2]))]", "[[{\"start\":0,\"end\":1}]]\n", "", 0),
    ("[]", "path(.[]?|.[0:1]?)", "", "", 0),
    ("[]", ".[0:1] as $x|path(.[0:1])", "[{\"start\":0,\"end\":1}]\n", "", 0),
    ("{\"a\":[1,2,3]}", "path(.[1:2])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[-1:])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[1.5:2.5])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[null:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[1:2]?)", "", "", 0),
    ("{\"a\":[1,2,3]}", "path(try .[1:2])", "", "", 0),
    ("{\"a\":[1,2,3]}", "path(.[1:2][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[1:2]|.[0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[0:1][])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[0:1][]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[0:1][0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[5:6][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.a[:1])", "[\"a\",{\"start\":null,\"end\":1}]\n", "", 0),
    ("{\"a\":[1,2,3]}", "path(.a[:1]?)", "[\"a\",{\"start\":null,\"end\":1}]\n", "", 0),
    ("{\"a\":[1,2,3]}", "path(.a[:1][0])", "[\"a\",{\"start\":null,\"end\":1},0]\n", "", 0),
    ("{\"a\":[1,2,3]}", "[path(.[0],.[1:2][0]?,.[1])]", "", "jq: error (at <stdin>:0): Cannot index object with number\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[0:1]|.[0]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "[path(.[0:1],.[1:2])]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "first(path(.[0:1],error(\"x\")))", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "[limit(1;path(.[0:1],.[1:2]))]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":[1,2,3]}", "path(.[]?|.[0:1]?)", "[\"a\",{\"start\":0,\"end\":1}]\n", "", 0),
    ("{\"a\":[1,2,3]}", ".[0:1] as $x|path(.[0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[1:2])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[-1:])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[1.5:2.5])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[null:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[1:2]?)", "", "", 0),
    ("{\"a\":\"abc\"}", "path(try .[1:2])", "", "", 0),
    ("{\"a\":\"abc\"}", "path(.[1:2][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[1:2]|.[0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[0:1][])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[0:1][]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[0:1][0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[5:6][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.a[:1])", "[\"a\",{\"start\":null,\"end\":1}]\n", "", 0),
    ("{\"a\":\"abc\"}", "path(.a[:1]?)", "[\"a\",{\"start\":null,\"end\":1}]\n", "", 0),
    ("{\"a\":\"abc\"}", "path(.a[:1][0])", "", "jq: error (at <stdin>:0): Cannot index string with number\n", 5),
    ("{\"a\":\"abc\"}", "[path(.[0],.[1:2][0]?,.[1])]", "", "jq: error (at <stdin>:0): Cannot index object with number\n", 5),
    ("{\"a\":\"abc\"}", "path(.[0:1]|.[0]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "[path(.[0:1],.[1:2])]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "first(path(.[0:1],error(\"x\")))", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "[limit(1;path(.[0:1],.[1:2]))]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":\"abc\"}", "path(.[]?|.[0:1]?)", "[\"a\",{\"start\":0,\"end\":1}]\n", "", 0),
    ("{\"a\":\"abc\"}", ".[0:1] as $x|path(.[0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[1:2])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[-1:])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[1.5:2.5])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[null:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[1:2]?)", "", "", 0),
    ("{\"a\":null}", "path(try .[1:2])", "", "", 0),
    ("{\"a\":null}", "path(.[1:2][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[1:2]|.[0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[0:1][])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[0:1][]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[0:1][0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[5:6][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.a[:1])", "[\"a\",{\"start\":null,\"end\":1}]\n", "", 0),
    ("{\"a\":null}", "path(.a[:1]?)", "[\"a\",{\"start\":null,\"end\":1}]\n", "", 0),
    ("{\"a\":null}", "path(.a[:1][0])", "[\"a\",{\"start\":null,\"end\":1},0]\n", "", 0),
    ("{\"a\":null}", "[path(.[0],.[1:2][0]?,.[1])]", "", "jq: error (at <stdin>:0): Cannot index object with number\n", 5),
    ("{\"a\":null}", "path(.[0:1]|.[0]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "[path(.[0:1],.[1:2])]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "first(path(.[0:1],error(\"x\")))", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "[limit(1;path(.[0:1],.[1:2]))]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":null}", "path(.[]?|.[0:1]?)", "[\"a\",{\"start\":0,\"end\":1}]\n", "", 0),
    ("{\"a\":null}", ".[0:1] as $x|path(.[0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[1:2])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[-1:])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[1.5:2.5])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[null:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[1:2]?)", "", "", 0),
    ("{}", "path(try .[1:2])", "", "", 0),
    ("{}", "path(.[1:2][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[1:2]|.[0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[0:1][])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[0:1][]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[0:1][0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[5:6][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.a[:1])", "[\"a\",{\"start\":null,\"end\":1}]\n", "", 0),
    ("{}", "path(.a[:1]?)", "[\"a\",{\"start\":null,\"end\":1}]\n", "", 0),
    ("{}", "path(.a[:1][0])", "[\"a\",{\"start\":null,\"end\":1},0]\n", "", 0),
    ("{}", "[path(.[0],.[1:2][0]?,.[1])]", "", "jq: error (at <stdin>:0): Cannot index object with number\n", 5),
    ("{}", "path(.[0:1]|.[0]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "[path(.[0:1],.[1:2])]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "first(path(.[0:1],error(\"x\")))", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "[limit(1;path(.[0:1],.[1:2]))]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{}", "path(.[]?|.[0:1]?)", "", "", 0),
    ("{}", ".[0:1] as $x|path(.[0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[1:2])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[-1:])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[1.5:2.5])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[null:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[1:2]?)", "", "", 0),
    ("{\"a\":5}", "path(try .[1:2])", "", "", 0),
    ("{\"a\":5}", "path(.[1:2][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[1:2]|.[0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[0:1][])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[0:1][]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[0:1][0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[5:6][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.a[:1])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("{\"a\":5}", "path(.a[:1]?)", "", "", 0),
    ("{\"a\":5}", "path(.a[:1][0])", "", "jq: error (at <stdin>:0): Cannot index number with object\n", 5),
    ("{\"a\":5}", "[path(.[0],.[1:2][0]?,.[1])]", "", "jq: error (at <stdin>:0): Cannot index object with number\n", 5),
    ("{\"a\":5}", "path(.[0:1]|.[0]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "[path(.[0:1],.[1:2])]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "first(path(.[0:1],error(\"x\")))", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "[limit(1;path(.[0:1],.[1:2]))]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":5}", "path(.[]?|.[0:1]?)", "", "", 0),
    ("{\"a\":5}", ".[0:1] as $x|path(.[0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[1:2])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[-1:])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[1.5:2.5])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[null:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[1:2]?)", "", "", 0),
    ("{\"a\":{\"b\":1}}", "path(try .[1:2])", "", "", 0),
    ("{\"a\":{\"b\":1}}", "path(.[1:2][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[1:2]|.[0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[1:2]|.a)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[0:1][])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[0:1][]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[0:1][0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[5:6][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.a[:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.a[:1]?)", "", "", 0),
    ("{\"a\":{\"b\":1}}", "path(.a[:1][0])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "[path(.[0],.[1:2][0]?,.[1])]", "", "jq: error (at <stdin>:0): Cannot index object with number\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[0:1]|.[0]?)", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "[path(.[0:1],.[1:2])]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "first(path(.[0:1],error(\"x\")))", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "[limit(1;path(.[0:1],.[1:2]))]", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
    ("{\"a\":{\"b\":1}}", "path(.[]?|.[0:1]?)", "", "", 0),
    ("{\"a\":{\"b\":1}}", ".[0:1] as $x|path(.[0:1])", "", "jq: error (at <stdin>:0): Cannot index object with object\n", 5),
];

/// #6265: `path()` of a narrow literal slice walks with cursors. Its answers, its errors and their
/// order with what came before them must be jq's, over a target of any kind.
#[test]
fn path_of_a_narrow_literal_slice_answers_as_jq_does_6265() -> Result<()> {
    for (input, filter, stdout, stderr, code) in PATH_OF_A_NARROW_SLICE {
        let got = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*stdout, *stderr, *code),
            "{filter} on {input}"
        );
    }
    Ok(())
}

/// The walk and the materializing route agree on either side of the width the gate turns on.
#[test]
fn the_width_cap_does_not_show_in_the_answer_6265() -> Result<()> {
    let input = format!("{:?}", vec![1; 5000]);
    for (filter, want) in [
        ("path(.[:4096]) | length", "1\n"),
        ("path(.[:4097]) | length", "1\n"),
        (
            "path(.[:4096][4095])",
            "[{\"start\":null,\"end\":4096},4095]\n",
        ),
        (
            "path(.[:4097][4096])",
            "[{\"start\":null,\"end\":4097},4096]\n",
        ),
    ] {
        let got = run_jq_full(&["-c", filter], Some(&input))?;
        assert_eq!((got.0.as_str(), got.2), (want, 0), "{filter}: {}", got.1);
    }
    Ok(())
}

/// #6265 is the #2168 boundary moving, recorded in `docs/compliance/jq/limitations.md`: a narrow
/// slice validates what `.a[s:e]` validates, so a fault outside it no longer raises, where a wide
/// slice materializes the document and still does. jq 1.7.1 rejects all of these at parse.
#[test]
fn a_narrow_slice_validates_what_the_slice_itself_does_6265() -> Result<()> {
    let answered = [
        "{\"a\":[1,2],\"b\":\"bad\\x\"}",
        "{\"b\":[,],\"a\":[1,2]}",
        "{\"a\":[1,\"\\q\"]}",
    ];
    for input in answered {
        let got = run_jq_full(&["-c", "path(.a[:1])"], Some(input))?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            ("[\"a\",{\"start\":null,\"end\":1}]\n", "", 0),
            "{input}"
        );
        // A wide slice takes the materializing route, which reads the whole document.
        let got = run_jq_full(&["-c", "path(.a[1:])"], Some(input))?;
        assert_eq!((got.0.as_str(), got.2), ("", 5), "{input}");
    }
    // What the slice reads still raises, and `?` does not absorb it.
    for input in [
        "{\"a\":[\"\\q\",2]}",
        "{\"a\":[1,]}",
        "{\"a\":[1,,2]}",
        "{\"a\":[1 2]}",
        "{\"a\":[tru,2]}",
        "{\"a\":\"\\q\"}",
        // Under a full-range slice too, which is the element walk's own arm.
        "{\"a\":[[,]]}",
    ] {
        for filter in ["path(.a[:1])", "path(.a[:1]?)", ".a[:1]"] {
            let got = run_jq_full(&["-c", filter], Some(input))?;
            assert_eq!(
                (got.0.as_str(), got.2),
                ("", 5),
                "{filter} on {input}: {}",
                got.1
            );
        }
    }
    Ok(())
}

/// A full-range slice validates through the cursor: `[.[0:3]] | length` and `path(.[0:3])` answered
/// on a nested `[,]` that jq 1.7.1 rejects at parse, because a copy of a bare value does not see it.
#[test]
fn a_full_range_slice_rejects_a_nested_empty_gap_as_jq_does_6265() -> Result<()> {
    for input in ["[[,],2,3]", "[1,[,],3]", "[1,2,[,]]"] {
        for filter in [
            "path(.[0:3])",
            "path(.[0:3]?)",
            "[.[0:3]] | length",
            ".[0:3]",
        ] {
            let got = run_jq_full(&["-c", filter], Some(input))?;
            assert_eq!(
                (got.0.as_str(), got.2),
                ("", 5),
                "{filter} on {input}: {}",
                got.1
            );
        }
    }
    Ok(())
}
