//! #7497: a postfix after a negated number literal is `-(1[0])`. jq's grammar reads the `-` as
//! the unary `'-' Exp`, so `-1[0]` negates the Term `1[0]`: a run-time `Cannot index number with
//! number` (exit 5), or nothing under `?`. `succinctly jq` folded `-1` into one literal and refused
//! the `[` as `unexpected character` (exit 3). Expected outputs were captured from jq 1.7.1
//! (`/usr/bin/jq`); jq 1.8.2 agrees except that its index errors name the index
//! (`Cannot index number with number (0)`), a separate wording change, so the messages are matched
//! up to that suffix.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_postfix_after_a_negated_number_literal_negates_the_postfixed_term_7497() -> Result<()> {
    // (program, stdout, exit status, text of the run-time error)
    for (program, out, code, err) in [
        ("-1[0]", "", 5, "Cannot index number with number"),
        ("-1.5[0]", "", 5, "Cannot index number with number"),
        ("-1e2[0]", "", 5, "Cannot index number with number"),
        ("-.5[0]", "", 5, "Cannot index number with number"),
        ("-1.e1[0]", "", 5, "Cannot index number with number"),
        ("- 1[0]", "", 5, "Cannot index number with number"),
        ("-(1)[0]", "", 5, "Cannot index number with number"),
        ("-1 .[0]", "", 5, "Cannot index number with number"),
        ("-1 [0]", "", 5, "Cannot index number with number"),
        ("-1[0][1]", "", 5, "Cannot index number with number"),
        ("-1[0] as $x|$x", "", 5, "Cannot index number with number"),
        ("-1[0]//2", "", 5, "Cannot index number with number"),
        ("-1[0] * 2", "", 5, "Cannot index number with number"),
        ("-1[0]|tostring", "", 5, "Cannot index number with number"),
        ("-1.[0] | 3", "", 5, "Cannot index number with number"),
        ("{a:-1[0]}", "", 5, "Cannot index number with number"),
        ("-1[]", "", 5, "Cannot iterate over number (1)"),
        ("-1[0:1]", "", 5, "Cannot index number with object"),
        ("-1[\"a\"]", "", 5, "Cannot index number with string \"a\""),
        (
            "-1[0]=3",
            "",
            5,
            "Invalid path expression near attempt to access element 0 of 1",
        ),
        (
            "path(-1[0])",
            "",
            5,
            "Invalid path expression near attempt to access element 0 of 1",
        ),
        // Under `?` the error is dropped.
        ("-1[]?", "", 0, ""),
        ("-1[0]?", "", 0, ""),
        ("-1.[0]?", "", 0, ""),
        ("-1 [0]?", "", 0, ""),
        ("-1.[]?", "", 0, ""),
        ("-1.5[1:2]?", "", 0, ""),
        ("-1.[\"a\"]?", "", 0, ""),
        ("-1[\"a\"]?", "", 0, ""),
        ("-1[(0,1)]?", "", 0, ""),
        ("-1[null]?", "", 0, ""),
        ("-1[[1]]?", "", 0, ""),
        ("-0[0]?", "", 0, ""),
        ("-9007199254740993[0]?", "", 0, ""),
        ("-1[]?|.+1", "", 0, ""),
        ("1 - -1[0]?", "", 0, ""),
        ("1 as $x | -1[$x]?", "", 0, ""),
        ("-1[0]?  as $x | 5", "", 0, ""),
        ("{a: -1[0]?}", "", 0, ""),
        ("{a: -1 [0]?}", "", 0, ""),
        ("[-1.[]?]", "[]\n", 0, ""),
        ("[-1[]?]", "[]\n", 0, ""),
        ("[-1[0]?]", "[]\n", 0, ""),
        ("[-1[0]?, 7]", "[7]\n", 0, ""),
        ("[{a: -1[]?}]", "[]\n", 0, ""),
        (
            "try (-1[0]) catch .",
            "\"Cannot index number with number\"\n",
            0,
            "",
        ),
        // Controls: a negated literal with no postfix, and a postfix that was always fine.
        ("[1,2]|-.[0]", "-1\n", 0, ""),
        ("[1,[2]]|-.[1][0]", "-2\n", 0, ""),
        ("-1|.[0]?", "", 0, ""),
        ("{a: -1 | 2}", "{\"a\":2}\n", 0, ""),
        ("{a: -1}", "{\"a\":-1}\n", 0, ""),
        ("{a: -1.5}", "{\"a\":-1.5}\n", 0, ""),
        ("[-1]", "[-1]\n", 0, ""),
        ("[-1 , 2]", "[-1,2]\n", 0, ""),
        ("[-1,-2]|.[-1]", "-2\n", 0, ""),
        ("[1,2,3]|.[-1]", "3\n", 0, ""),
        ("[1,2,3]|.[-1:]", "[3]\n", 0, ""),
        ("[1,2,3]|.[:-1]", "[1,2]\n", 0, ""),
        ("[1,2,3]|.[-2:-1]", "[2]\n", 0, ""),
        ("-1 as $x|$x", "-1\n", 0, ""),
        ("-1 * 3", "-3\n", 0, ""),
        ("-1 + 3", "2\n", 0, ""),
    ] {
        let (stdout, stderr, status) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (stdout.as_str(), status),
            (out, code),
            "{program}: {stderr}"
        );
        assert!(stderr.contains(err), "{program}: {stderr}");
    }
    Ok(())
}

#[test]
fn test_postfix_after_a_negated_number_literal_still_refuses_a_field_name_7497() -> Result<()> {
    // jq: `-1.a` is a syntax error (the lexer takes `1.`, then an identifier): exit 3.
    // An object value takes no `Exp '?'`, `*` or `as` either.
    for program in [
        "-1.a",
        "{a: -1.a}",
        "{a: -1?}",
        "{a: -1 * 2}",
        "{a: -1 as $x|$x}",
    ] {
        let (out, _err, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!((out.as_str(), code), ("", 3), "{program}");
    }
    Ok(())
}
