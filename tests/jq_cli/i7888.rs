//! #7888: a regex with two errors reported the later one in three shapes since #7852.
//!
//! Oniguruma refuses an interval whose upper bound is below its lower one (`a{2,1}`) as it reads
//! the token, so that error stands where the interval does: `a{2,1}\k<m>` and `(?x)\A{2,1}` are
//! `upper is smaller than lower in repeat range`. succinctly left it to the `regex` crate's
//! parser, which runs after the scan that words jq's other parse errors, so any error of that
//! scan came first wherever it stood (the undefined name, the repeated assertion).
//!
//! `\1\g<1 >a` is `invalid backref number/name`: a call by a name that starts with a digit and
//! is no number names no group, which jq says in the last of its passes (`undefined name <1 >
//! reference`), after a missing group's backreference. succinctly worded it as a parse error
//! (`invalid group name <1 >`), and the ordering of #7852 then put it first.
//!
//! The ordering rests on scans that have to read the pattern as jq does, so the fix also makes
//! them read a class without nesting (`[[]` is a whole class), a control escape with the
//! character it names (`\c$` is no anchor, `\c(` opens no group; `(?x)\c$+` was refused since
//! #7852, the third item of #7887), a group's or a reference's name as written under `x`
//! (`(?< n>a)`), a conditional whose condition starts with `?`, and a backreference at a
//! recursion level (`\k<1+0>`), which was `invalid group name <1>` wherever it stood.
//!
//! The same finding listed seven more shapes, in which a name was cut short: jq reads the name of
//! a call (`\g<n\k<n>`) to its `>`, pattern syntax and all, and refuses a property whose braces
//! never close (`\p{a)`) where it stands; succinctly read on inside them.
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on every row.

use anyhow::Result;

use crate::run_jq_full;

/// `(program, jq's answer)`: rows that differed from jq before the fix.
static FIXED: &[(&str, &str)] = &[
    (
        r#""a" | test("a{2,1}\\k<m>")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("\\1\\g<1 >a")"#,
        r"Regex failure: invalid backref number/name",
    ),
    (
        r#""a" | test("(?x)\\A{2,1}")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("{2,1}")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("^{2,1}")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("(?i){2,1}")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("\\b{3,1}")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("(?=a){2,1}")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("a{2,1}\\")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("\\g<1 >a{2,1}")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("a{2,1}\\k<m>"; "x")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("(?x) \\A {2,1}")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("a{2,1}^*")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("\\g<1 >a")"#,
        r"Regex failure: undefined name <1 > reference",
    ),
    (
        r#""a" | test("\\g<2x>")"#,
        r"Regex failure: undefined name <2x> reference",
    ),
    (
        r#""a" | test("()\\g<1 >")"#,
        r"Regex failure: undefined name <1 > reference",
    ),
    (
        r#""a" | test("\\g'1 '")"#,
        r"Regex failure: undefined name <1 > reference",
    ),
    (
        r#""a" | test("\\1\\g<2x>")"#,
        r"Regex failure: invalid backref number/name",
    ),
    (
        r#""a" | test("\\g<1 >\\1")"#,
        r"Regex failure: invalid backref number/name",
    ),
    (
        r#""a" | test("\\g<1a>(?<n>a)\\k<n >")"#,
        r"Regex failure: invalid char in group name <n >",
    ),
    (
        r#""a" | test("(?(?i)a)")"#,
        r"Regex failure: target of repeat operator is not specified",
    ),
    (
        r#""a" | test("(?(?i)a{2,1})")"#,
        r"Regex failure: target of repeat operator is not specified",
    ),
    (r#""f" | test("(?x)\\c$+")"#, r"false"),
    (r#""f" | test("(?i)\\c$+")"#, r"false"),
    (r#""f" | test("(?x)\\c^*")"#, r"true"),
    (r#""f" | test("(?:a)\\c$?")"#, r"false"),
    (
        r#""f" | test("(?x)\\c# $*\n")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (
        r#""a" | test("[[](?<=a)*"; "x")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (
        r#""a" | test("(?:[[])(?=a)+")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (
        r#""a" | test("[[]{0,}(?<!(?-x:|ab))*+")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (
        r#""a" | test("(?< n>a)(?=b)*"; "x")"#,
        r"Regex failure: invalid char in group name < n>",
    ),
    (
        r#""a" | test("\\k< n>(?=b)*"; "x")"#,
        r"Regex failure: invalid char in group name < n>",
    ),
    (
        r#""a" | test("(?<=(?<n*[]a](?>\\c${0}?)|[ a]))+")"#,
        r"Regex failure: unmatched close parenthesis",
    ),
    (r#""aa" | test("(a)\\k<1+0>")"#, r"true"),
    (r#""aa" | test("(a)\\k<1-0>")"#, r"true"),
    (r#""aa" | test("(a)\\k<1+1>")"#, r"false"),
    (r#""aa" | test("(a)\\k<-1+0>")"#, r"true"),
    (r#""aa" | test("(a)\\k'1+0'")"#, r"true"),
    (r#""aa" | test("(?<n>a)\\k<n+0>")"#, r"true"),
    (r#""aa" | test("(?<n>a)\\k<n+1>")"#, r"false"),
    (r#""aa" | [match("(a)(?:\\k<1+0>)+").string]"#, r#"["aa"]"#),
    (
        r#""aa" | test("\\k<1+0>")"#,
        r"Regex failure: invalid backref number/name",
    ),
    (
        r#""aa" | test("\\k<2+0>(a)")"#,
        r"Regex failure: invalid backref number/name",
    ),
    (r#""aa" | test("\\k<1+0>(a)")"#, r"false"),
    (
        r#""aa" | test("\\k<0+0>")"#,
        r"Regex failure: invalid backref number/name",
    ),
    (
        r#""aa" | test("\\k<-1+0>")"#,
        r"Regex failure: invalid backref number/name",
    ),
    (
        r#""aa" | test("\\k<1+0>(")"#,
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r#""aa" | test("\\k<1+0>a{2,1}")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""aa" | test("(?<=\\k<1+0>|ab)")"#,
        r"Regex failure: invalid backref number/name",
    ),
    (
        r#""a" | test("\\p{a)")"#,
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r#""a" | test("\\p{(?x)?+")"#,
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r#""a" | test("(?~\\g<1 >")"#,
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r#""a" | test("\\g<n\\k<n>\\g<n>")"#,
        r"Regex failure: undefined name <n\k<n> reference",
    ),
    (
        r#""a" | test("\\g<++\\k<1>")"#,
        r"Regex failure: undefined name <++\k<1> reference",
    ),
    (
        r#""a" | test("\\g<n(?<n>a)\\k<n")"#,
        r"Regex failure: unmatched close parenthesis",
    ),
    (
        r#""a" | test("\\g<n?+(?<=abc\\k<n>)")"#,
        r"Regex failure: unmatched close parenthesis",
    ),
    (
        r#""a" | test("\\p{a)b")"#,
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r#""a" | test("\\p{a)}")"#,
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r#""a" | test("\\p{a{2,1}")"#,
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r#""a" | test("\\p{a\\k<m>")"#,
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r#""a" | test("(\\p{a))")"#,
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r#""a" | test("\\p{a[")"#,
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r#""a" | test("\\g<n[>")"#,
        r"Regex failure: undefined name <n[> reference",
    ),
    (
        r#""a" | test("\\g'n(?<n>a)'")"#,
        r"Regex failure: invalid group name <n(?<n>a>",
    ),
    (
        r#""aab" | test("a{2,1}(*a)")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""aab" | test("a{2,1}(?x)(b)\\1 (?<= ?)c")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""aab" | test("a{2,1}[")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("\\k< n>a{2,1}")"#,
        r"Regex failure: invalid char in group name < n>",
    ),
    (
        r#""a" | test("\\k<a b>")"#,
        r"Regex failure: invalid char in group name <a b>",
    ),
    (
        r#""a" | test("\\k<(>")"#,
        r"Regex failure: invalid char in group name <(>",
    ),
    (
        r#""a" | test("\\k<a.b>")"#,
        r"Regex failure: invalid char in group name <a.b>",
    ),
    (
        r#""a" | test("(?<n>a)\\k<n >")"#,
        r"Regex failure: invalid char in group name <n >",
    ),
    (
        r#""a" | test("\\g<0>\\p{^N}\\1")"#,
        r"Regex failure: invalid backref number/name",
    ),
    (
        r#""a" | test("\\g<0>\\p{^N}")"#,
        r"Regex failure: never ending recursion",
    ),
    (
        r#""a" | test("\\g<n>\\p{^N}")"#,
        r"Regex failure: undefined name <n> reference",
    ),
    (
        r#""a" | test("\\P{^N}\\g<n>")"#,
        r"Regex failure: undefined name <n> reference",
    ),
];

/// Rows that agreed with jq before the fix and must not change.
static KEPT: &[(&str, &str)] = &[
    (
        r#""a" | test("a{2,1}(")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("\\k<m>a{2,1}")"#,
        r"Regex failure: undefined name <m> reference",
    ),
    (
        r#""a" | test("^*a{2,1}")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (
        r#""a" | test("a{100001}{2,1}")"#,
        r"Regex failure: too big number for repeat range",
    ),
    (
        r#""a" | test("a{2,1}{100001}")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (r#""a" | test("[{2,1}]")"#, r"false"),
    (r#""a" | test("a\\{2,1}")"#, r"false"),
    (r#""a" | test("(?#{2,1})a")"#, r"true"),
    (r#""a" | test("a{2, 1}")"#, r"false"),
    (r#""a" | test("a{1,2}")"#, r"true"),
    (r#""a" | test("a{2,2}")"#, r"false"),
    (
        r#""a" | test("\\k<1 >")"#,
        r"Regex failure: invalid group name <1 >",
    ),
    (
        r#""a" | test("\\1\\k<1 >")"#,
        r"Regex failure: invalid group name <1 >",
    ),
    (
        r#""a" | test("b?\\g<n>]{100001}\\g<1 >")"#,
        r"Regex failure: too big number for repeat range",
    ),
    (
        r#""a" | test("\\k<-1>a{100001}")"#,
        r"Regex failure: invalid backref number/name",
    ),
    (
        r#""a" | test("\\1a{100001}")"#,
        r"Regex failure: too big number for repeat range",
    ),
    (
        r#""a" | test("\\k<m>a{100001}")"#,
        r"Regex failure: undefined name <m> reference",
    ),
    (
        r#""a" | test("(?(?x-i){3,1}"; "x")"#,
        r"Regex failure: target of repeat operator is not specified",
    ),
    (
        r#""a" | test("(?(?=a)b{2,1})")"#,
        r"Regex failure: target of repeat operator is not specified",
    ),
    (
        r#""a" | test("(?(a)b{2,1})")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""f" | test("(?x)\\c $*")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (r#""\u0003" | test("(?x)^\\c#$")"#, r"true"),
    (r#""\u0000" | test("(?x)^\\c $")"#, r"true"),
    (r#""f" | test("(?x)\\c(+")"#, r"false"),
    (
        r#""f" | test("(?x)\\C-$+")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (r#""f" | test("\\c$+"; "x")"#, r"false"),
    (
        r#""f" | test("(?x)$+")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (r#""]" | test("(?x)[[ ] ]")"#, r"false"),
    (
        r#""a" | test("(?x)[[:alpha:]] ^*")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (r#""a" | test("(?:[[:alpha:]]\\b)*")"#, r"true"),
    (
        r#""a" | test("(?< n>a)"; "x")"#,
        r"Regex failure: invalid char in group name < n>",
    ),
    (r#""a" | test("(?<n>a) \\k<n>"; "x")"#, r"false"),
    (
        r#""a" | test("(?<n(?x:>*?)"; "x")"#,
        r"Regex failure: target of repeat operator is not specified",
    ),
    (
        r#""a" | test("(?<=(?<n*a>b))+")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (
        r#""aa" | test("\\k<1a+0>")"#,
        r"Regex failure: invalid group name <1a>",
    ),
    (r#""aa" | test("(a)\\k<1>")"#, r"true"),
    (r#""aa" | test("(a)\\k<-1>")"#, r"true"),
    (
        r#""a" | test("\\p{a")"#,
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r#""a" | test("a{2,1}\\p{a")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (r#""a" | test("\\p{L}")"#, r"true"),
    (r#""a" | test("\\p{^L}a")"#, r"false"),
    (
        r#""a" | test("[\\p{a)]")"#,
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r#""a" | test("\\g<n)>)")"#,
        r"Regex failure: invalid group name <n>",
    ),
    (
        r#""a" | test("\\g<a{2,1}>")"#,
        r"Regex failure: undefined name <a{2,1}> reference",
    ),
    (
        r#""a" | test("\\g<a>{2,1}")"#,
        r"Regex failure: upper is smaller than lower in repeat range",
    ),
    (
        r#""a" | test("\\k<n+0>(?<n>)")"#,
        r"Regex failure: undefined name <n> reference",
    ),
    (r#""a" | test("(?<n>a)\\k<n>")"#, r"false"),
    (
        r#""a" | test("\\g<n>\\p{^Foo}")"#,
        r"Regex failure: invalid character property name {Foo}",
    ),
    (
        r#""a" | test("\\p{^Foo}")"#,
        r"Regex failure: invalid character property name {Foo}",
    ),
    (r#""a" | test("\\p{^N}")"#, r"true"),
];

/// Runs each program as `try (PROGRAM | tojson) catch .`, so an answer and a refusal are both one
/// line of text.
fn check(rows: &[(&str, &str)]) -> Result<()> {
    for &(program, want) in rows {
        let wrapped = format!("try ({program} | tojson) catch .");
        let got = run_jq_full(&["-nr", &wrapped], None)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (format!("{want}\n").as_str(), "", 0),
            "{program}"
        );
    }
    Ok(())
}

#[test]
fn a_regex_with_two_errors_reports_the_one_jq_does_7888() -> Result<()> {
    check(FIXED)
}

#[test]
fn regex_errors_that_agreed_with_jq_keep_their_words_7888() -> Result<()> {
    check(KEPT)
}

/// The three rows of the issue, uncaught: jq's message on stderr and exit status 5.
#[test]
fn the_uncaught_refusal_is_jqs_error_7888() -> Result<()> {
    for (program, message) in [
        (
            r#""a" | test("a{2,1}\\k<m>")"#,
            "upper is smaller than lower in repeat range",
        ),
        (
            r#""a" | test("\\1\\g<1 >a")"#,
            "invalid backref number/name",
        ),
        (
            r#""a" | test("(?x)\\A{2,1}")"#,
            "upper is smaller than lower in repeat range",
        ),
    ] {
        let got = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (
                "",
                format!("jq: error (at <unknown>): Regex failure: {message}\n").as_str(),
                5
            ),
            "{program}"
        );
    }
    Ok(())
}
