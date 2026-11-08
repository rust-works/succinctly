//! #6437: `-R 'fromjson | ...'` parses each line in place instead of writing it out as JSON text,
//! indexing that and decoding the string back (a fifth of the run at 10 MB). The door takes a text
//! that parses and leaves every other line to the bridge, so each row, errors included, is what jq
//! 1.7.1 prints; every expected value below was captured from it.

use anyhow::Result;

use crate::run_jq_full;

/// `(program, input, stdout, stderr, exit status)`.
type Row<'a> = (&'a str, &'a str, &'a str, &'a str, i32);

#[test]
fn raw_lines_through_fromjson_match_jq_6437() -> Result<()> {
    let rows: &[Row] = &[
        ("fromjson|.a", "{\"a\":1}\n{\"a\":[1,2]}\n", "1\n[1,2]\n", "", 0),
        (
            "fromjson",
            "[1,2]\n  {\"b\":null}  \n\"x\"\n",
            "[1,2]\n{\"b\":null}\n\"x\"\n",
            "",
            0,
        ),
        ("fromjson|.[]?", "[1,2]\n7\n[]\n", "1\n2\n", "", 0),
        (
            "try fromjson catch \"E\"",
            "[1,2\n{\"a\":1}\n",
            "\"E\"\n{\"a\":1}\n",
            "",
            0,
        ),
        ("fromjson?", "1 2\n[3]\n", "[3]\n", "", 0),
        ("(fromjson|.a)?", "[1]\n{\"a\":3}\n", "3\n", "", 0),
        ("fromjson|path(..)", "{\"a\":[1]}\n", "[]\n[\"a\"]\n[\"a\",0]\n", "", 0),
        ("fromjson|length", "\u{feff}[1,2]\n", "2\n", "", 0),
        (
            "fromjson",
            "[1,2\n",
            "",
            "jq: error (at <stdin>:1): Unfinished JSON term at EOF at line 1, column 4 (while parsing '[1,2')\n",
            5,
        ),
    ];
    for &(program, input, stdout, stderr, status) in rows {
        let (out, err, code) = run_jq_full(&["-R", "-c", program], Some(input))?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (stdout, stderr, status),
            "-R -c {program:?} over {input:?}"
        );
    }
    Ok(())
}

/// The parse depth ceiling is jq's: a line 256 containers deep parses, one more is the bridge's
/// error, since the door declines a text it cannot parse.
#[test]
fn the_fromjson_depth_ceiling_is_jqs_6437() -> Result<()> {
    let deep = |depth: usize| format!("{}{}\n", "[".repeat(depth), "]".repeat(depth));
    let (out, _, code) = run_jq_full(&["-R", "-c", "fromjson|type"], Some(&deep(256)))?;
    assert_eq!((out.as_str(), code), ("\"array\"\n", 0));
    let (out, err, code) = run_jq_full(&["-R", "-c", "fromjson|type"], Some(&deep(257)))?;
    assert_eq!((out.as_str(), code), ("", 5));
    assert!(
        err.starts_with(
            "jq: error (at <stdin>:1): Exceeds depth limit for parsing at line 1, column 257"
        ),
        "{err}"
    );
    Ok(())
}
