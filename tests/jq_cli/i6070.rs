//! #6070: Oniguruma escapes and group syntax the `regex` crate refuses or reads another way. Every
//! row was captured from jq 1.7.1 (`/usr/bin/jq -nc`): a `\` + letter that means nothing is the
//! letter, `\N`/`\O`/`\Q..\E`, octal and control escapes, `\xHH` bytes of a UTF-8 sequence,
//! `(*FAIL)`, and the patterns jq refuses (an interval with nothing to repeat, a conditional on a
//! group the pattern lacks, a group name that starts with a digit or is empty, a short multibyte
//! escape). `"a" | test("(?(2)a|b)")` used to panic inside `fancy-regex`.

use anyhow::Result;

use crate::run_jq_full;

/// `succinctly jq -nc <filter>`: stdout, stderr and the exit code.
fn run(filter: &str) -> Result<(String, String, i32)> {
    run_jq_full(&["-nc", filter], None)
}

fn assert_rows(rows: &[(&str, &str)]) -> Result<()> {
    for (filter, expected) in rows {
        let (stdout, stderr, code) = run(filter)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (*expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_octal_and_control_escapes_match_jq_6070() -> Result<()> {
    assert_rows(&[
        (r#""A" | [match("\\101";"g") | .string]"#, r#"["A"]"#),
        (
            r#""\u0000" | [match("\\0";"g") | .string]"#,
            r#"["\u0000"]"#,
        ),
        (r#""\n" | [match("\\012";"g") | .string]"#, r#"["\n"]"#),
        (r#""A" | [match("\\101x";"g") | .string]"#, "[]"),
        (r#""\b" | [match("\\10";"g") | .string]"#, r#"["\b"]"#),
        (r#""aa" | [match("(a)\\10";"g") | .string]"#, "[]"),
        (r#""\u0001" | [match("\\18";"g") | .string]"#, "[]"),
        (
            r#""\u0001" | [match("\\cA";"g") | .string]"#,
            r#"["\u0001"]"#,
        ),
        (
            r#""\u001b" | [match("\\c[";"g") | .string]"#,
            r#"["\u001b"]"#,
        ),
        (
            r#""\u007f" | [match("\\c?";"g") | .string]"#,
            r#"["\u007f"]"#,
        ),
        (
            r#""\u0001" | [match("[\\1]";"g") | .string]"#,
            r#"["\u0001"]"#,
        ),
        (r#""A" | [match("[\\101]";"g") | .string]"#, r#"["A"]"#),
        (
            r#""\u0001" | [match("[\\cA]";"g") | .string]"#,
            r#"["\u0001"]"#,
        ),
        (r#""A" | [match("[\\x41]";"g") | .string]"#, r#"["A"]"#),
        (
            r#""ab\u0001" | [match("\\cA";"g") | .string]"#,
            r#"["\u0001"]"#,
        ),
    ])
}

#[test]
fn test_letters_with_no_meaning_are_themselves_6070() -> Result<()> {
    assert_rows(&[
        (r#""h" | [match("\\h";"g") | .string]"#, r#"["h"]"#),
        (r#""H" | [match("\\H";"g") | .string]"#, r#"["H"]"#),
        (r#""g" | [match("\\g";"g") | .string]"#, r#"["g"]"#),
        (r#""u" | [match("\\u";"g") | .string]"#, r#"["u"]"#),
        (
            r#""u0041" | [match("\\u0041";"g") | .string]"#,
            r#"["u0041"]"#,
        ),
        (r#""A" | [match("\\u0041";"g") | .string]"#, "[]"),
        (r#""E" | [match("\\E";"g") | .string]"#, r#"["E"]"#),
        (r#""aEb" | [match("a\\Eb";"g") | .string]"#, r#"["aEb"]"#),
        (r#""C-a" | [match("\\C-a";"g") | .string]"#, r#"["C-a"]"#),
        (r#""q" | [match("\\q";"g") | .string]"#, r#"["q"]"#),
        (r#""M" | [match("\\M";"g") | .string]"#, r#"["M"]"#),
        (r#""p" | [match("\\p";"g") | .string]"#, r#"["p"]"#),
        (r#""pL" | [match("\\pL";"g") | .string]"#, r#"["pL"]"#),
        (r#""o" | [match("\\o";"g") | .string]"#, r#"["o"]"#),
        (r#""k" | [match("(?<a>x)\\k";"g") | .string]"#, "[]"),
        (r#""h" | [match("[\\h]";"g") | .string]"#, r#"["h"]"#),
        (r#""N" | [match("[\\N]";"g") | .string]"#, r#"["N"]"#),
        (r#""y" | [match("[\\y]";"g") | .string]"#, r#"["y"]"#),
        (r#""X" | [match("[\\X]";"g") | .string]"#, r#"["X"]"#),
        (r#""8" | [match("[\\8]";"g") | .string]"#, r#"["8"]"#),
        (r#""Q" | [match("[\\Q]";"g") | .string]"#, r#"["Q"]"#),
        (r#""E" | [match("[\\E]";"g") | .string]"#, r#"["E"]"#),
        (r#""u" | [match("[\\u]";"g") | .string]"#, r#"["u"]"#),
    ])
}

#[test]
fn test_n_o_and_quoting_escapes_match_jq_6070() -> Result<()> {
    assert_rows(&[
        (r#""a" | [match("\\N";"g") | .string]"#, r#"["a"]"#),
        (r#""\n" | [match("\\N";"g") | .string]"#, "[]"),
        (r#""a" | [match("\\O";"g") | .string]"#, r#"["a"]"#),
        (r#""\n" | [match("\\O";"g") | .string]"#, r#"["\n"]"#),
        (r#""ab" | [match("a\\Nb";"g") | .string]"#, "[]"),
        (r#""a\nb" | [match("a\\Ob";"g") | .string]"#, r#"["a\nb"]"#),
        (
            r#""a.b" | [match("\\Qa.b\\E";"g") | .string]"#,
            r#"["a.b"]"#,
        ),
        (r#""axb" | [match("\\Qa.b\\E";"g") | .string]"#, "[]"),
        (r#""a.b" | [match("\\Qa.b";"g") | .string]"#, r#"["a.b"]"#),
        (r#""ab" | [match("\\Qa\\Eb";"g") | .string]"#, r#"["ab"]"#),
        (r#""a(" | [match("\\Qa(\\E";"g") | .string]"#, r#"["a("]"#),
        (
            r#""a b" | [match("\\Qa b\\E";"g") | .string]"#,
            r#"["a b"]"#,
        ),
    ])
}

#[test]
fn test_hex_escapes_read_utf8_bytes_6070() -> Result<()> {
    assert_rows(&[
        (
            r#""\u00e9" | [match("\\xc3\\xa9";"g") | .string]"#,
            r#"["é"]"#,
        ),
        (
            r#""\u20ac" | [match("\\xe2\\x82\\xac";"g") | .string]"#,
            r#"["€"]"#,
        ),
        (
            r#""\ud83d\ude00" | [match("\\xf0\\x9f\\x98\\x80";"g") | .string]"#,
            r#"["😀"]"#,
        ),
        (
            r#""\u00e9" | [match("\\xc3\\xa9+";"g") | .string]"#,
            r#"["é"]"#,
        ),
        (r#""a" | [match("\\xf5";"g") | .string]"#, "[]"),
        (r#""a" | [match("\\xf8";"g") | .string]"#, "[]"),
        (r#""a" | [match("\\xe0\\x80\\x80";"g") | .string]"#, "[]"),
        (r#""a" | [match("\\xed\\xa0\\x80";"g") | .string]"#, "[]"),
        (
            r#""\u0004" | [match("\\x4";"g") | .string]"#,
            r#"["\u0004"]"#,
        ),
        (
            r#""\u0000g" | [match("\\xg";"g") | .string]"#,
            r#"["\u0000g"]"#,
        ),
        (r#""A" | [match("\\x41";"g") | .string]"#, r#"["A"]"#),
        (r#""\u00e9" | [match("\\x{e9}";"g") | .string]"#, r#"["é"]"#),
        (r#""\u00c3" | [match("\\x{c3}";"g") | .string]"#, r#"["Ã"]"#),
    ])
}

#[test]
fn test_fail_verb_never_matches_6070() -> Result<()> {
    assert_rows(&[
        (r#""a" | [match("(*FAIL)";"g") | .string]"#, "[]"),
        (r#""a" | [match("x(*FAIL)|a";"g") | .string]"#, r#"["a"]"#),
        (
            r#""aa" | [match("(*FAIL)|a";"g") | .string]"#,
            r#"["a","a"]"#,
        ),
    ])
}

#[test]
fn test_valid_neighbours_do_not_change_6070() -> Result<()> {
    assert_rows(&[
        (r#""aa" | [match("a{1}{2}";"g") | .string]"#, r#"["aa"]"#),
        (r#""a{ 2}" | [match("{ 2}";"g") | .string]"#, r#"["{ 2}"]"#),
        (r#""a{,2}" | [match("{,2}";"g") | .string]"#, r#"["{,2}"]"#),
        (r#""{" | [match("x{|{";"g") | .string]"#, r#"["{"]"#),
        (r#""aa" | [match("(a){2}";"g") | .string]"#, r#"["aa"]"#),
        (r#""aa" | [match("a{2}";"g") | .string]"#, r#"["aa"]"#),
        (r#""aa" | [match("(a)\\1";"g") | .string]"#, r#"["aa"]"#),
        (
            r#""aa" | [match("(?<n>a)\\k<n>";"g") | .string]"#,
            r#"["aa"]"#,
        ),
        (
            r#""abc" | [match("(?(1)a|b)(c)";"g") | .string]"#,
            r#"["bc"]"#,
        ),
        (
            r#""ab" | [match("(a)(?(1)b|c)";"g") | .string]"#,
            r#"["ab"]"#,
        ),
        (
            r#""ab" | [match("(?<n>a)(?(<n>)b)";"g") | .string]"#,
            r#"["ab"]"#,
        ),
        (r#""a1" | [match("[a-c]\\d+";"g") | .string]"#, r#"["a1"]"#),
        (r#""a1" | [match("\\w\\d";"g") | .string]"#, r#"["a1"]"#),
        (r#""xay" | [match("x\\Ky";"g") | .string]"#, "[]"),
        (r#""aa" | [match("(a)(a)\\2";"g") | .string]"#, "[]"),
    ])
}

/// Patterns jq refuses, with its reason. The exit code is 5 and the message names the reason;
/// jq's own text is `Regex failure: <reason>`, ours `invalid regex: <reason>`.
#[test]
fn test_patterns_jq_refuses_are_refused_6070() -> Result<()> {
    for (pattern, reason) in [
        (r"\xc3", "too short multibyte code string"),
        (r"\xc3x", "too short multibyte code string"),
        (r"\xc3\xa9\xc3", "too short multibyte code string"),
        (r"\x80", "invalid code point value"),
        (r"\xc3\x28", "invalid code point value"),
        (r"\c", "end pattern at control"),
        ("{1}", "target of repeat operator is not specified"),
        ("{1,2}", "target of repeat operator is not specified"),
        ("{1,}", "target of repeat operator is not specified"),
        ("a|{2}", "target of repeat operator is not specified"),
        ("(|{2})", "target of repeat operator is not specified"),
        ("(?:{2})", "target of repeat operator is not specified"),
        ("(?<n>{2})", "target of repeat operator is not specified"),
        ("(?={2})", "target of repeat operator is not specified"),
        ("(?i:{2})", "target of repeat operator is not specified"),
        ("a(?:{2})", "target of repeat operator is not specified"),
        ("(?(2)a|b)", "invalid backref number/name"),
        ("(?(0)a|b)", "invalid backref number/name"),
        ("(?(1)a)", "invalid backref number/name"),
        ("(?(<n>)a|b)", "undefined name <n> reference"),
        ("(?('n')a|b)", "undefined name <n> reference"),
        ("(?<1a>.)", "invalid group name <1a>"),
        ("(?<>.)", "group name is empty"),
        ("(?'1a'.)", "invalid group name <1a>"),
        (r"\1", "invalid backref number/name"),
        (r"\8", "invalid backref number/name"),
        (r"(a)\2", "invalid backref number/name"),
        ("^{1}", "target of repeat operator is invalid"),
    ] {
        let quoted = format!("\"{}\"", pattern.replace('\\', "\\\\").replace('"', "\\\""));
        let filter = format!("\"a\" | test({quoted})");
        let (stdout, stderr, code) = run(&filter)?;
        assert_eq!((stdout.as_str(), code), ("", 5), "{pattern}: {stderr}");
        assert!(
            stderr.contains(&format!("invalid regex: {reason}")),
            "{pattern}: {stderr}"
        );
    }
    Ok(())
}
