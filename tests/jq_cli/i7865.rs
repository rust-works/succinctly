//! #7865: a look-behind that jq cannot size was accepted under the `x` flag or an inline `(?x)`.
//!
//! Oniguruma sizes a look-behind's body after it has parsed the pattern, so the blanks and `#`
//! comments the extended mode ignores are no part of what it sizes: `(?x)(?<=a|bc)` and
//! `(?<= a | bc )` under `x` are `invalid pattern in look-behind`, as `(?<=a|bc)` is. succinctly
//! skipped its look-behind checks for any pattern under `x` or with an inline flag group naming
//! `x` (`(?-x)` too), because the blanks hid the body's length, and answered such a pattern.
//!
//! The checks now read the pattern without what the mode ignores, following the mode group by
//! group (`(?x)`, `(?x:...)`, `(?-x)`). A brace that opens no interval for the blank it holds is
//! the literal characters the mode leaves (`a{ 0}` is the four characters `a{0}`), and an item
//! repeated no times is no part of a branch's length (`(?<=(?i:a)b|c{0})` is accepted).
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on every row.

use anyhow::Result;

use crate::run_jq_full;

/// `(pattern, flags, jq's answer)`: the offsets and lengths of the matches in `"xabcab c\nab"` as
/// JSON text, or the error. No flags is the one-argument `match`. These rows differed from jq
/// before the fix.
static FIXED: &[(&str, &str, &str)] = &[
    (
        r"(?<=a|bc)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a|bc)",
        r"gx",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?x)(?<=a|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?-x)(?<=a|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a|bc)(?x)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"a(?x)(?<=a|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?x:(?<=a|bc))",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?x: (?<= a | bc ) )",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=(?x)a|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=(?x) a | b c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=(?x:a|bc))",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<= a | bc )",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a |b c)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        "(?<=a#x\n|bc)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a\ |b)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=[ ]|bc)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a*b)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<= a * b )",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=a*b*)", r"x", r"Regex failure: undefined error code"),
    (r"(?x)(?<=a*b*)", "", r"Regex failure: undefined error code"),
    (
        r"(?<=bb?)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?x)(?<=b b?)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<!a|bc)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=(a)?)b",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a|bc)",
        r"ix",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?i)(?<=a|bc)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?ix)(?<=a|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?x-i)(?<=a|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?i-x)(?<=a|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?x)(?<=a*b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a|bc)",
        r"xx",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a|\ b)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?x)a(?-x)(?<=a| b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?x)a(?-x)(?<=a|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?-x:(?<=a| b))",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=(?-x) |ab)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=(?-x: )|ab)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a(?x)b|c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a(?x) b | c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a(?i)b|c)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a(?x:b)|c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=(?x)a*b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?x)(?<=a(?-x)b|c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?x)(?<=a(?-x)b|c d)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?x)(?<=a(?-x)b| )",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=(?x)a+b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a(?x)b?)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a(?x)b*c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a(?x)b+)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=(?x)a*b*)", "", r"Regex failure: undefined error code"),
    (r"(?<=a*(?x)b*)", "", r"Regex failure: undefined error code"),
    (
        r"(?<=a(?x)|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a{ 0}|bcdef)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a{ 0}|bcd)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a{0 ,1}|bcdef)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a{0 ,1}|bcdefgh)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a{ }|bc)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a{,}|bcd)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a{, 1}|bcde)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        "(?<=a{#k\n0}|bcd)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<!b{2}\ (?i:a)\{1,2\}|(?:a){0})", "", r"[]"),
    (r"(?<=(?i:a)b|c{0})", "", r"[[0,0]]"),
    (r"(?<=(?i:a)b|(?:c){0})", "", r"[[0,0]]"),
    (r"(?<=(?i:a)b|(c){0})", "", r"[[0,0]]"),
    (r"(?<=(?i:a)b|c{0}|d)", "", r"[[0,0]]"),
    (r"(?<=ab|c{0})", r"i", r"[[0,0]]"),
    (r"(?<=(?i:a)b|c{0,0})", "", r"[[0,0]]"),
    (r"(?<=(?i:a)|c{0})", "", r"[[0,0]]"),
    (r"(?<!(?i:a)b|c{0})", "", r"[]"),
    (r"(?<=(?:a|bc){0})c", r"i", r"[[3,1]]"),
    (r"(?<=x(?:a|bc){0})c", r"i", r"[]"),
    (r"(?<=x(?:a|bc){0}|y)c", r"i", r"[]"),
    (r"(?<=(?:(?:a|bc){0}|d))c", r"i", r"[[3,1]]"),
    (r"(?<=(?:a|bc){0,0})c", r"i", r"[[3,1]]"),
    (
        r"(?<=x\1{0}|yz)(c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=x(?#c){0}|y)c", r"i", r"[[3,1]]"),
];

/// The same, for rows that already agreed with jq and must not change: the look-behinds jq
/// accepts under `x`, the ones it refuses without it, and the errors that come before the
/// look-behind's.
static KEPT: &[(&str, &str, &str)] = &[
    (
        r"(?<=a|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<= a | b )", r"x", r"[[2,0]]"),
    (r"(?<= a | b )", "", r"[]"),
    (r"(?<=a |bc)", "", r"[[4,0]]"),
    (
        r"(?<=a |b c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=a b|bc)", r"x", r"[[3,0]]"),
    (
        r"(?<=a b|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=ab|b c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?x)(?<=a b|bc)c", "", r"[[3,1]]"),
    ("(?<=a#x\n|b)", r"x", r"[[2,0]]"),
    (
        "(?<=a#x\n|b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=a(?#c)|b)", r"x", r"[[2,0]]"),
    (r"(?<=a\ |bc)", r"x", r"[[4,0]]"),
    (r"(?<=[ ]|b)", r"x", r"[[3,0]]"),
    (
        r"(?<=a*b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=a*a)", r"x", r"[[2,0]]"),
    (r"(?<= a* a)", r"x", r"[[2,0]]"),
    (r"(?<=a*b*)", "", r"Regex failure: undefined error code"),
    (
        r"(?<!(a))",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<! ( a ) )",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=a|)b", r"x", r"[[2,1]]"),
    (r"(?<= a | )b", r"x", r"[[2,1]]"),
    (r"(?<=(a)|)b", r"x", r"[[2,1]]"),
    (r"(?<=(a)|)b", "", r"[[2,1]]"),
    (
        r"(?<=a|bc)",
        r"i",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=a{1, 2})", r"x", r"[]"),
    (r"(?<=a{1,2})", r"x", r"[[2,0]]"),
    (r"(?<=a{2})", r"x", r"[]"),
    (r"(?<=a {2})", r"x", r"[]"),
    (r"(?<=\Qa b\E|abc)", r"x", r"[[4,0]]"),
    (
        r"(?<=a|bc)*",
        r"x",
        r"Regex failure: target of repeat operator is invalid",
    ),
    (
        r"(?x)(?<=a|bc)\1",
        "",
        r"Regex failure: invalid backref number/name",
    ),
    (
        r"(?<=a|bc)(",
        r"x",
        r"Regex failure: end pattern with unmatched parenthesis",
    ),
    (
        r"(?<=(?=a)a)",
        r"x",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=a|bc)", r"z", r"z is not a valid modifier string"),
    (r"(?<=a|bc)", r"xz", r"xz is not a valid modifier string"),
    (r"(?x)(?<=a|bc)", r"z", r"z is not a valid modifier string"),
    (
        r"(?<=a|\ b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a| b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?x)a(?-x)(?<=a|b)", "", r"[[1,1]]"),
    (r"(?-x:(?<=a|b))", r"x", r"[[2,0]]"),
    (r"(?-x:(?<= a| b))", r"x", r"[]"),
    (r"(?<=(?-x) |a)", r"x", r"[[2,0]]"),
    (r"(?<=(?-x: )|a)", r"x", r"[[2,0]]"),
    (
        r"(?<=a(?i)b|c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a(?m)b|c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=a(?i)b|cd)", "", r"[[3,0]]"),
    (r"(?<=a(?x)b|cd)", "", r"[[3,0]]"),
    (
        r"(?<=(?i)a|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=(?m)a|bc)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=a(?x:b)|cd)", "", r"[[3,0]]"),
    (r"(?<=a(?i:b)|cd)", "", r"[[3,0]]"),
    (
        r"(?<=a(?i:b)|c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=(?x) a*a)", "", r"[[2,0]]"),
    (r"(c)(?<=b??\d*(?x:a b)*?|[ ]{1,2})b", "", r"[]"),
    (
        r"(c)(?<=b??\d*(?:ab)*?|[ ]{1,2})b",
        "",
        r"Regex failure: undefined error code",
    ),
    (r"(c)(?<=b??\d*(?i:ab)*?|[ ]{1,2})b", "", r"[]"),
    (r"(c)(?<=b??\d*(?m:ab)*?|[ ]{1,2})b", "", r"[]"),
    (r"(?<=(?x:a b)*)", "", r"[[0,0]]"),
    (r"(?<=(?:ab)*)", "", r"[[0,0]]"),
    (r"(?<=(?m:ab)*)", "", r"[[0,0]]"),
    (
        r"(?<=(?:ab)*b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=(?x:a)?)", "", r"[[0,0]]"),
    (r"(?<=(?:a)?)", "", r"[[0,0]]"),
    (r"(?<=(?m:a)?)", "", r"[[0,0]]"),
    (r"(?<=(?:a)+)", "", r"[[2,0]]"),
    (
        r"(?<=(?:a)+b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=a+b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=(?x)a*)", "", r"[[0,0]]"),
    (r"(?<=(?x)a?b)", "", r"[[3,0]]"),
    (r"(?<=a?b)", "", r"[[3,0]]"),
    (
        r"(?<=ab?)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=ab*c)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=ab+)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=a*(?x)a)", "", r"[[2,0]]"),
    (r"(?<=a*a)", "", r"[[2,0]]"),
    (r"(?<=(?x)|bc)", "", r"[[0,0]]"),
    (r"(?<=|bc)", "", r"[[0,0]]"),
    (r"(?<=(?x)a|(?x)b)", "", r"[[2,0]]"),
    (
        r"(?m:(?<=a*b))",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?:(?<=a*b))",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?m:(?<=a*b))+",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?:(?<=a*b))+",
        "",
        r"Regex failure: target of repeat operator is invalid",
    ),
    (r"(?m:(?<=a*a))", "", r"[[2,0]]"),
    (
        r"(?m:(?<=a*b*))",
        "",
        r"Regex failure: undefined error code",
    ),
    (
        r"(?m:(?<=(a)?)b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?m:c(?<=bb?))",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?-i:c(?<=bb?))",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=a{ 0})", r"x", r"[]"),
    (r"(?<=a{ 0}|bcde)", r"x", r"[]"),
    (r"(?<=a{0 ,1}|bcdefg)", r"x", r"[]"),
    (r"(?<=a{ }|bcd)", r"x", r"[]"),
    (r"(?<=a{,}|bcde)", r"x", r"[]"),
    (r"(?<=a{,}|bcde)", "", r"[]"),
    (r"(?<=a{, 1}|bcdef)", r"x", r"[]"),
    (r"(?<=\w{1, 2})b", "", r"[]"),
    (r"(?<=a{1, 2})b", "", r"[]"),
    (
        r"(?<=a{1,2}|bcdefgh)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=a\{1,2\}|bcdefg)", "", r"[]"),
    (r"(?<=a\{1,2}|bcdefg)", "", r"[]"),
    (r"(?<=a{x}|bcde)", "", r"[]"),
    (r"(?<=a{,2}|bcdef)", "", r"[]"),
    ("(?<=a{#k\n0}|bcde)", r"x", r"[]"),
    (r"(?<=a {0}|bcd)", r"x", r"[[0,0]]"),
    ("(?<=a{0\n}|bcde)", r"x", r"[]"),
    (
        r"(?<=(?i:a)b|c{0}d)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=(?i:a)b|c{0}de)", "", r"[[3,0]]"),
    (r"(?<=(?i:a)b|)", "", r"[[0,0]]"),
    (r"(?<=(?i:a)b|c?)", "", r"[[0,0]]"),
    (r"(?<=(?i:a)b|c*)", "", r"[[0,0]]"),
    (
        r"(?<=(?i:a)b|\b)",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=(?i:a)b|(?:))", "", r"[[0,0]]"),
    (r"(?<=ab|c{0})", "", r"[[0,0]]"),
    (r"(?<=(?#c)ab|c{0})", "", r"[[0,0]]"),
    (
        r"(?<=(?=a)ab|c{0})",
        "",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (r"(?<=(?:a|bc){0})c", "", r"[[3,1]]"),
    (
        r"(?<=x(?:a|bc){0}|yz)c",
        r"i",
        r"Regex failure: invalid pattern in look-behind",
    ),
    (
        r"(?<=(?:a|bc){1})c",
        r"i",
        r"Regex failure: invalid pattern in look-behind",
    ),
    ("(?<=(?:a|b\\tc){0}\n)c", r"ix", r"[[3,1]]"),
    ("(?i)(?x)x(?<=(?:a\\t| bc ){0}#k\n)", "", r"[[0,1]]"),
    (r"(?<=x\1{0}|y)(c)", "", r"[]"),
    (
        r"(?<=x(?=a){0}|yz)c",
        r"i",
        r"Regex failure: target of repeat operator is invalid",
    ),
    (
        r"(?<=x(?=a){0}|y)c",
        r"i",
        r"Regex failure: target of repeat operator is invalid",
    ),
];

fn check(rows: &[(&str, &str, &str)]) -> Result<()> {
    for &(pattern, flags, want) in rows {
        let pattern_json = serde_json::to_string(pattern)?;
        let call = if flags.is_empty() {
            format!("match({pattern_json})")
        } else {
            format!("match({pattern_json}; {})", serde_json::to_string(flags)?)
        };
        let program =
            format!(r#""xabcab c\nab" | try ([{call} | [.offset, .length]] | tojson) catch ."#);
        let got = run_jq_full(&["-nr", &program], None)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (format!("{want}\n").as_str(), "", 0),
            "{pattern:?} with flags {flags:?}"
        );
    }
    Ok(())
}

#[test]
fn a_look_behind_jq_cannot_size_is_refused_under_the_x_flag_7865() -> Result<()> {
    check(FIXED)
}

#[test]
fn look_behinds_that_agreed_with_jq_keep_their_answer_7865() -> Result<()> {
    check(KEPT)
}

#[test]
fn the_uncaught_refusal_is_jqs_error_7865() -> Result<()> {
    for program in [
        r#""x" | test("(?<=a|bc)"; "x")"#,
        r#""x" | test("(?x)(?<=a|bc)")"#,
        r#""x" | test("(?-x)(?<=a|bc)")"#,
    ] {
        let got = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (
                "",
                "jq: error (at <unknown>): Regex failure: invalid pattern in look-behind\n",
                5
            ),
            "{program}"
        );
    }
    Ok(())
}
