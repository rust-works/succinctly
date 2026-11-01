//! #5675: a `-R` line, or any owned input, is not written out as JSON and indexed to answer a
//! program that does not need the text: `(.|length) as $n | $n * 2` binds a count the string door
//! answers and then reads a literal, and an expression that never reads `.` (`1 + 2`, `$n * 2` once
//! `$n` is a literal) is one value whatever the input is. The answers were captured from jq 1.7.1
//! (`/usr/bin/jq`): bindings that shadow, nest, generate, `label` and raise included.

use anyhow::Result;

use crate::run_jq_full;

const LINES: &str = "hello\n\nhéllo\n[1,2]\n";

const RAW: &[(&str, &str)] = &[
    (r"(.|length) as $n|$n*2", "10\n0\n10\n10\n"),
    (
        r"length as $n|[$n,.]",
        "[5,\"hello\"]\n[0,\"\"]\n[5,\"héllo\"]\n[5,\"[1,2]\"]\n",
    ),
    (
        r"utf8bytelength as $b|[$b,length]",
        "[5,5]\n[0,0]\n[6,5]\n[5,5]\n",
    ),
    (
        r#"startswith("h") as $s|[$s]"#,
        "[true]\n[false]\n[true]\n[false]\n",
    ),
    (r"length as $n|(5 as $n|$n)", "5\n5\n5\n5\n"),
    (
        r"length as $n|[range($n)]",
        "[0,1,2,3,4]\n[]\n[0,1,2,3,4]\n[0,1,2,3,4]\n",
    ),
    (r"length as $n|label $out|$n,break $out", "5\n0\n5\n5\n"),
    (
        r#"length as $n|try error("x\($n)") catch ."#,
        "\"x5\"\n\"x0\"\n\"x5\"\n\"x5\"\n",
    ),
    (r"length as $n|$n as $m|$m+1", "6\n1\n6\n6\n"),
    (
        r#"length as $n|if $n>2 then "big" else "small" end"#,
        "\"big\"\n\"small\"\n\"big\"\n\"big\"\n",
    ),
    (r"1+2", "3\n3\n3\n3\n"),
    (r"[1,2]|length", "2\n2\n2\n2\n"),
    (r"5 as $x|$x*2", "10\n10\n10\n10\n"),
    (r"length*2", "10\n0\n10\n10\n"),
    (r#""a"+"b""#, "\"ab\"\n\"ab\"\n\"ab\"\n\"ab\"\n"),
];

/// (document, program, jq 1.7.1's stdout, its exit status)
const DOC_CASES: &[(&str, &str, &str, i32)] = &[
    (r"[1,2,3]", r"length as $n|[$n,.]", "[3,[1,2,3]]\n", 0),
    (r"[1,2,3]", r"(.|length) as $n|$n*2", "6\n", 0),
    (r"[1,2,3]", r"length as $n|[$n]|length", "1\n", 0),
    (r"[1,2,3]", r"1+2", "3\n", 0),
    (r"[1,2,3]", r"length as $n|{a:$n}", "{\"a\":3}\n", 0),
    (
        r"[1,2,3]",
        r#"length as $n|try error("x\($n)") catch ."#,
        "\"x3\"\n",
        0,
    ),
    (r"[1,2,3]", r"length as $n|$n as $m|$m+1", "4\n", 0),
    (
        r#"{"a":1,"b":[2]}"#,
        r"length as $n|[$n,.]",
        "[2,{\"a\":1,\"b\":[2]}]\n",
        0,
    ),
    (r#"{"a":1,"b":[2]}"#, r"(.|length) as $n|$n*2", "4\n", 0),
    (r#"{"a":1,"b":[2]}"#, r"length as $n|[$n]|length", "1\n", 0),
    (r#"{"a":1,"b":[2]}"#, r"1+2", "3\n", 0),
    (
        r#"{"a":1,"b":[2]}"#,
        r"length as $n|{a:$n}",
        "{\"a\":2}\n",
        0,
    ),
    (
        r#"{"a":1,"b":[2]}"#,
        r#"length as $n|try error("x\($n)") catch ."#,
        "\"x2\"\n",
        0,
    ),
    (
        r#"{"a":1,"b":[2]}"#,
        r"length as $n|$n as $m|$m+1",
        "3\n",
        0,
    ),
    (r#""abc""#, r"length as $n|[$n,.]", "[3,\"abc\"]\n", 0),
    (r#""abc""#, r"(.|length) as $n|$n*2", "6\n", 0),
    (r#""abc""#, r"length as $n|[$n]|length", "1\n", 0),
    (r#""abc""#, r"1+2", "3\n", 0),
    (r#""abc""#, r"length as $n|{a:$n}", "{\"a\":3}\n", 0),
    (
        r#""abc""#,
        r#"length as $n|try error("x\($n)") catch ."#,
        "\"x3\"\n",
        0,
    ),
    (r#""abc""#, r"length as $n|$n as $m|$m+1", "4\n", 0),
    (r"null", r"length as $n|[$n,.]", "[0,null]\n", 0),
    (r"null", r"(.|length) as $n|$n*2", "0\n", 0),
    (r"null", r"length as $n|[$n]|length", "1\n", 0),
    (r"null", r"1+2", "3\n", 0),
    (r"null", r"length as $n|{a:$n}", "{\"a\":0}\n", 0),
    (
        r"null",
        r#"length as $n|try error("x\($n)") catch ."#,
        "\"x0\"\n",
        0,
    ),
    (r"null", r"length as $n|$n as $m|$m+1", "1\n", 0),
    (r"7", r"length as $n|[$n,.]", "[7,7]\n", 0),
    (r"7", r"(.|length) as $n|$n*2", "14\n", 0),
    (r"7", r"length as $n|[$n]|length", "1\n", 0),
    (r"7", r"1+2", "3\n", 0),
    (r"7", r"length as $n|{a:$n}", "{\"a\":7}\n", 0),
    (
        r"7",
        r#"length as $n|try error("x\($n)") catch ."#,
        "\"x7\"\n",
        0,
    ),
    (r"7", r"length as $n|$n as $m|$m+1", "8\n", 0),
];

#[test]
fn raw_input_bound_counts_and_closed_bodies_answer_as_jq_does_5675() -> Result<()> {
    for (program, expected) in RAW {
        let (stdout, stderr, code) = run_jq_full(&["-R", "-c", program], Some(LINES))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, 0),
            "{program}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn documents_bound_counts_and_closed_bodies_answer_as_jq_does_5675() -> Result<()> {
    for (doc, program, expected, status) in DOC_CASES {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(doc))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*expected, *status),
            "{program} over {doc}: {stderr}"
        );
    }
    Ok(())
}
