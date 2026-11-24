//! #7887 (second item): look-behinds jq accepts were refused, under `x` since #7865 and without it
//! before.
//!
//! The look-behind checks size a body with a `regex-syntax` parse of the pattern, which reads
//! three things differently from jq's Oniguruma:
//!
//! - A `[` inside a class is a member in jq, which has no nested classes: `[[ab]]` is the class
//!   `[[ab]` and a `]`, two characters, so `(?<=[[ab]]|ab)` is a body of one length. Read as one
//!   class it was `invalid pattern in look-behind`, and `(?<=[[ab]]|a)`, which jq refuses, was
//!   answered.
//! - A brace that opens no interval is the characters it is: `a{0 }` is five characters. Read as
//!   `a{0}` it made `(?<=a{0 })` a look-behind that matches nothing, which answered `true`.
//! - An option run in a branch (`(?<=(?i)a{0}|(a))`) is one option node around the alternation
//!   to Oniguruma, and a branch of it that is sure to match nothing makes the body no
//!   look-behind at all.
//!
//! Where the whole pattern is matched without regard to case, a class that holds a character
//! whose folding is longer than itself (`[[:alpha:]]` holds `ß`) has no one length, and jq refuses
//! the look-behind it stands in; so does succinctly now.
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on every row.

use anyhow::Result;

use crate::run_jq_full;

/// `(program, jq's answer)`: rows that differed from jq before the fix.
static FIXED: &[(&str, &str)] = &[
    (r#""KkK" | test("(?x)(?<=[[ab]]|ab)")"#, r"false"),
    (r#""KkK" | test("(?<=[[ab]]|ab)")"#, r"false"),
    (r#""x" | capture("(?<=(?i)a{0}|(a))"; "x")"#, r"{}"),
    (r#""x" | capture("(?<=(?i)a{0}|(a))")"#, r"{}"),
    (r#""a b" | test("(?<=[[ ]]|a b)"; "x")"#, r"false"),
    (
        r#""b]" | test("(?<=[[ab]]|a)$")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abc" | test("(?<=[[ab]c]|abc)")"#, r"true"),
    (
        r#""abc" | test("(?<=[a[b]|ab)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=[[]|ab)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abc" | test("(?<![[ab]]|ab)c"; "x")"#, r"false"),
    (
        r#""abc" | test("(?x)(?<= [[ab]] | a )")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abc" | test("(?<=[[ab] ]|ab)"; "x")"#, r"true"),
    (r#""abc" | test("(?<=a{0 }|bcdef)")"#, r"false"),
    (
        r#""abc" | test("(?<=a{0 }|bc)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abc" | test("(?<=(?i)|ab)")"#, r"true"),
    (r#""abc" | test("(?<=(?i)a{0}|ab)")"#, r"true"),
    (r#""abc" | test("(?<=x|(?i)|bc)")"#, r"true"),
    (r#""abc" | test("(?<=(?i)(?:)|bc)")"#, r"true"),
    (r#""abc" | test("(?<=xy|(?i))")"#, r"true"),
    (r#""abc" | test("(?<=xy|(?i)a{0})")"#, r"true"),
    (r#""abc" | test("(?<=(?i)|(1))")"#, r"true"),
    (r#""abc" | test("(?<=xy|(?x-i))"; "x")"#, r"true"),
    (
        r#""abc" | test("(?<=(?<=(?-i)|a{,2}[a-c]{1}))"; "x")"#,
        r"true",
    ),
    (
        r#""aa" | test("(?<=[[:alpha:]])"; "i")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""aa" | test("(?i)(?<=[[:alpha:]])")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""aa" | test("(?<=[[:^digit:]b]x|ab)"; "i")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""aa" | test("(?<=[\\D])"; "i")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""aa" | test("(?<=[\\w]x)"; "i")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""aa" | test("(?<=[\\S])"; "i")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""aa" | test("(?<![[:upper:]])"; "i")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=a\\z)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<!\\Z)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=(a|b\\z))")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=a|b\\z)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abc" | test("(?<=a{0 })")"#, r"false"),
    (r#""abc" | test("(?<=a{0 })c")"#, r"false"),
    (
        r#""abc" | test("(?<=a{ 0}|bc)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abc" | test("(?<=(a)|(?i))")"#, r"true"),
    (
        r#""abc" | test("a(?<!ab|)+")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (
        r#""abc" | test("(?<=a|)+")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (
        r#""abc" | test("(?<=){0}?")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (r#""ab" | test("(?<=[^a]\\w{1, 2})b")"#, r"false"),
    (
        r#""ab" | test("(?<=a{x}|bcd)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abc" | test("(?<=(?<=a)b|(?i))")"#, r"true"),
    (r#""abcd" | test("(?<=\\p{L}{ 1}|bcdef)")"#, r"false"),
];

/// Rows that agreed with jq before the fix and must not change.
static KEPT: &[(&str, &str)] = &[
    (
        r#""abc" | test("(?<=[[ab]c]|ab)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abc" | test("(?<=[[]|a)")"#, r"true"),
    (r#""abc" | test("(?<=[[:alpha:]]|a)")"#, r"true"),
    (
        r#""abc" | test("(?<=[a[:alpha:]]|ab)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abc" | test("(?<=[[:alpha:]]]|ab)")"#, r"true"),
    (r#""abc" | test("(a)(?<![[]+?x|a b*+| )b"; "x")"#, r"false"),
    (
        r#""abc" | test("a(?<![[ab]c]|a b[ab]{0 })")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abc" | test("(?<=k*[[]{ 0}|[[:alpha:]]{0})$")"#, r"true"),
    (r#""abc" | test("(?<=a{,2}|bcdef)")"#, r"false"),
    (r#""abc" | test("(?<=a}|bc)")"#, r"true"),
    (
        r#""abc" | test("(?<=a{1,2}|bc)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=a(?i)|bc)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=(?i)a|bc)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=(?i)\\b|bc)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=1\\Z|(?i))")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=1\\z|(?i))")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=(\\w|(?i-x))\\z|a{1})\\1"; "x")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=(?<=a|bc))")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abc" | test("(?<=(?<=a|b)c)")"#, r"true"),
    (r#""aa" | test("(?<=[[:alpha:]])")"#, r"true"),
    (
        r#""aa" | test("(?i)(?<=[[ ]]|[[:^digit:]b]x)(b)"; "x")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""aa" | test("(?<=[[:^alpha:]])"; "i")"#, r"false"),
    (r#""aa" | test("(?<=[^[:alpha:]])"; "i")"#, r"false"),
    (r#""aa" | test("(?<=[[:digit:]])"; "i")"#, r"false"),
    (r#""aa" | test("(?<=[\\d])"; "i")"#, r"false"),
    (r#""aa" | test("(?<=[\\W])"; "i")"#, r"false"),
    (r#""aa" | test("(?<=[a-z])"; "i")"#, r"true"),
    (r#""aa" | test("(?<=\\w)"; "i")"#, r"true"),
    (r#""aa" | test("(?=[[:alpha:]])"; "i")"#, r"true"),
    (r#""aa" | test("(?<=[[:alpha:]]|)"; "i")"#, r"true"),
    (r#""aa" | test("(?<=[[:alpha:]]?)"; "i")"#, r"true"),
    (r#""aa" | test("(?<=[[:alpha:]]{0})"; "i")"#, r"true"),
    (
        r#""aa" | test("(?i)(?<=[[:alpha:]]{0}?bc)$"; "x")"#,
        r"false",
    ),
    (r#""aa" | test("(?i)(?-i)(?<=[[:alpha:]])"; "i")"#, r"true"),
    (r#""aa" | test("(?i:a)(?<=[[:alpha:]])")"#, r"true"),
    (
        r#""abc" | test("(?<=[a[b]c]|x(?<m>\\zc{2}+))")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abc" | test("(?<=[\\z])")"#, r"false"),
    (r#""abc" | test("(?<=\\\\z)")"#, r"false"),
    (r#""abc" | test("(?<=a)\\z")"#, r"false"),
    (r#""abc" | test("(?=a\\z)")"#, r"false"),
    (r#""a{0 }c" | test("(?<=a{0 })c")"#, r"true"),
    (r#""abc" | test("(?<=a{0})c")"#, r"true"),
    (
        r#""abc" | test("(?<=(a)|(?i))\\1")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=a{a}|)+")"#,
        r"Regex failure: target of repeat operator is invalid",
    ),
    (r#""abc" | test("(?<=a|)b")"#, r"true"),
    (
        r#""abc" | [match("(?<=(a)|)b").captures[0].string]"#,
        r"[null]",
    ),
    (r#""ab" | test("(?<=a{x}|bcde)")"#, r"false"),
    (
        r#""abc" | test("(?<=K?+|(?<=(?-i)|\\W)\\Z\\B)")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=(?=a)|(?i))")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abc" | test("(?<=(?<=(?<=b)|(?#c)]b)|a|(?i-x))")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abcd" | test("(?<=\\p{L}|bc)d")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r#""abcd" | test("(?<=\\x{61}|bc)d")"#,
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r#""abcd" | test("(?<=\\p{L}|b)d")"#, r"true"),
    (r#""abcd" | test("(?<=\\Q{a\\E|bc)d")"#, r"true"),
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
fn a_look_behind_is_sized_with_jqs_classes_and_braces_7887() -> Result<()> {
    check(FIXED)
}

#[test]
fn look_behinds_that_agreed_with_jq_keep_their_answer_7887() -> Result<()> {
    check(KEPT)
}

/// The refusals and answers of the issue, uncaught.
#[test]
fn the_issues_rows_answer_as_jq_does_7887() -> Result<()> {
    for (program, want) in [
        (r#""KkK" | test("(?x)(?<=[[ab]]|ab)")"#, "false\n"),
        (r#""x" | capture("(?<=(?i)a{0}|(a))"; "x")"#, "{}\n"),
        (r#""a b" | test("(?<=[[ ]]|a b)"; "x")"#, "false\n"),
    ] {
        let got = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (want, "", 0),
            "{program}"
        );
    }
    let got = run_jq_full(&["-nc", r#""b]" | test("(?<=[[ab]]|a)$")"#], None)?;
    assert_eq!(
        (got.0.as_str(), got.1.as_str(), got.2),
        (
            "",
            "jq: error (at <unknown>): Regex failure: invalid pattern in look-behind\n",
            5
        )
    );
    Ok(())
}
