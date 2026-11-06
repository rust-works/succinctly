//! #6433: regex refusals #6423 left in the `regex` crate's words, or answered. Oniguruma reads the
//! number after a `{` as it lexes, so a count past 100000 is refused even where the brace is no
//! repeat or the crate cannot read the group around it; a call `\g<n>` has its own messages; and an
//! error after a construct the crate cannot read (`(?'name'...)`, a reference to a group the pattern
//! has) is the later error's. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -c 'try test(P) catch .' <<< '"a"'`).

use anyhow::Result;

use crate::run_jq_full;

fn caught(pattern: &str) -> Result<String> {
    let filter = format!("try test({}) catch .", serde_json::to_string(pattern)?);
    let (stdout, stderr, code) = run_jq_full(&["-c", &filter], Some(r#""a""#))?;
    assert_eq!((stderr.as_str(), code), ("", 0), "{pattern}");
    Ok(stdout.trim_end().to_owned())
}

fn assert_rows(rows: &[(&str, &str)]) -> Result<()> {
    for &(pattern, answer) in rows {
        let want = match answer {
            "true" | "false" => answer.to_owned(),
            message => format!(r#""Regex failure: {message}""#),
        };
        assert_eq!(caught(pattern)?, want, "{pattern}");
    }
    Ok(())
}

const TOO_BIG: &str = "too big number for repeat range";

#[test]
fn a_count_past_the_limit_is_refused_wherever_the_brace_stands_6433() -> Result<()> {
    assert_rows(&[
        ("(?=a{100001})", TOO_BIG),
        ("(?>a{100001})", TOO_BIG),
        ("(?<=a{100001})", TOO_BIG),
        ("(?!x{4294967296})", TOO_BIG),
        ("x{100001a}", TOO_BIG),
        ("x{100001 }", TOO_BIG),
        ("x{1,100001 }", TOO_BIG),
        ("x{100001,", TOO_BIG),
        ("x{100001", TOO_BIG),
        ("{100001}", TOO_BIG),
        ("a|{100001}", TOO_BIG),
        ("x{0100001}", TOO_BIG),
        ("x{1,}{100001}", TOO_BIG),
        ("[[a]{100001}]", TOO_BIG),
        ("[[:alpha:]]{100001}", TOO_BIG),
        ("(?x)a {100001}", TOO_BIG),
        // Never an interval: in a class, quoted, in a comment, escaped, a braced escape, `{,n}`,
        // a blank before the number, leading zeros.
        ("[{100001}]", "false"),
        ("[[:alpha:]{100001}]", "true"),
        (r"\Qa{100001}\E", "false"),
        ("a(?#{100001})", "true"),
        (r"\{100001}", "false"),
        (r"\x{100001}", "false"),
        ("x{,100001}", "false"),
        ("(?x)a { 100001}", "false"),
        ("x{00100000}", "false"),
    ])
}

/// Oniguruma reports the first error in the order it reads the pattern; an error it meets only at
/// the end (an unclosed group) or after reading it (an undefined call) comes after the count.
#[test]
fn an_error_before_the_count_comes_first_6433() -> Result<()> {
    assert_rows(&[
        (
            "{2,}{4294967296",
            "target of repeat operator is not specified",
        ),
        (r"\k<n>{999999,1", "undefined name <n> reference"),
        (
            "${1,2}{4294967296, }",
            "target of repeat operator is invalid",
        ),
        (r"(\Qa{\E{0100001,}", TOO_BIG),
        (r"\g<1>{100001, }", TOO_BIG),
    ])
}

#[test]
fn a_call_has_its_own_refusals_6433() -> Result<()> {
    assert_rows(&[
        (r"\g<1>", "undefined group <1> reference"),
        (r"(a)\g<2>", "undefined group <2> reference"),
        (r"\g<+1>", "undefined group <+1> reference"),
        (r"(a)\g<+1>", "undefined group <+1> reference"),
        (r"\g<-1>", "never ending recursion"),
        (r"(a)\g<-2>", "never ending recursion"),
        (r"\g<0>", "never ending recursion"),
        // The first of two calls' refusals, and a later unclosed group before a call's.
        (r"\g<n>\g<1>", "undefined name <n> reference"),
        (
            r"|\}(?'m'b)(\g<n>",
            "end pattern with unmatched parenthesis",
        ),
    ])
}

#[test]
fn an_error_after_a_construct_the_crate_cannot_read_is_found_6433() -> Result<()> {
    assert_rows(&[
        (r"(?'x'a)\k<y>", "undefined name <y> reference"),
        ("(?'x", "invalid group name <x>"),
        ("(?'x'", "end pattern with unmatched parenthesis"),
        ("(?'m'b)(?'m'b)(", "end pattern with unmatched parenthesis"),
        ("(?<m>b)(?<m>b)(", "end pattern with unmatched parenthesis"),
        (r"(?<x>a)\k<x>(", "end pattern with unmatched parenthesis"),
        (r"(?<x>a)\k<x>)", "unmatched close parenthesis"),
        // Text inside a class names no group.
        (r"[(?<x>)]\k<x>", "undefined name <x> reference"),
        // What jq answers stays answered.
        ("(?'x'a)", "true"),
        (r"(?'x'a)\k<x>", "false"),
        ("(?<m>b)(?<m>b)", "false"),
    ])
}
