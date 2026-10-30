//! #5502: a jq pattern the `regex` crate refuses for lookaround, a backreference, an atomic group
//! or `\K`/`\G`/`\e` is compiled by `fancy-regex` instead, and answers as jq does. Every row was
//! captured from jq 1.7.1 (`/usr/bin/jq -nc`).

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
fn test_lookaround_matches_jq_5502() -> Result<()> {
    assert_rows(&[
        (r#""qux" | gsub("(?=u)"; "u")"#, r#""quux""#),
        (
            r#""qux aby ab ab ayaby" | [match("(?<=a)b"; "g").offset]"#,
            "[5,9,12,17]",
        ),
        (
            r#""qux aby ab ab ayaby" | [match("a(?!b)"; "g").offset]"#,
            "[14]",
        ),
        (
            r#""qux aby ab ab ayaby" | [match("(?<!x)y"; "g").offset]"#,
            "[6,15,18]",
        ),
        // Variable-length and nested lookbehind.
        (r#""aXbbX" | [match("(?<=a+)X"; "g").offset]"#, "[1]"),
        (r#""abc" | [match("(?<=(?<=a)b)c").offset]"#, "[2]"),
        // Offsets stay codepoints, and the flags reach the second engine.
        (r#""ÀbÀb" | [match("(?<=À)b"; "g").offset]"#, "[1,3]"),
        (r#""ab AB" | [match("(?<=a)b"; "gi").offset]"#, "[1,4]"),
        (r#""a b" | [match("(?<= ) b"; "x").offset]"#, "[2]"),
        (r#""aaa" | [match("(?=a)"; "gn").offset]"#, "[]"),
        // Every builtin goes through it.
        (r#""a1b2" | [scan("(?<=\\d)\\w")]"#, r#"["b"]"#),
        (
            r#""camelCaseName" | [splits("(?=[A-Z])")]"#,
            r#"["camel","Case","Name"]"#,
        ),
        (
            r#""1234567.89" | sub("(?<=\\d)(?=(\\d{3})+(?!\\d))"; ","; "g")"#,
            r#""1,234,567.89""#,
        ),
        (
            r#""a1b2" | capture("(?<l>[a-z])(?=(?<d>\\d))")"#,
            r#"{"l":"a","d":"1"}"#,
        ),
        // A rewritten POSIX class inside a lookbehind.
        (
            r#""ab12" | [match("(?<=[[:alpha:]])\\d"; "g").offset]"#,
            "[2]",
        ),
        // `\<`/`\>` are the literal characters in Oniguruma.
        (r#""<a>" | [match("(?<=\\<)a(?=\\>)").offset]"#, "[1]"),
    ])
}

#[test]
fn test_backreferences_atomic_groups_and_oniguruma_escapes_match_jq_5502() -> Result<()> {
    assert_rows(&[
        (r#""abab" | test("(ab)\\1")"#, "true"),
        (
            r#""abcabc" | [match("(?<x>abc)\\k<x>") | .length, .captures[0].name]"#,
            r#"[6,"x"]"#,
        ),
        (r#""aaab" | [match("(?>a+)b").string]"#, r#"["aaab"]"#),
        (r#""aaa" | test("(?>a+)a")"#, "false"),
        (
            r#""ab" | [match("a\\Kb") | .offset, .string]"#,
            r#"[1,"b"]"#,
        ),
        (r#""abc" | [match("\\Gab"; "g").offset]"#, "[0]"),
        (r#""aab" | [match("(a)(?(1)b|c)").string]"#, r#"["ab"]"#),
        (r#""a\u001bb" | [match("\\e").offset]"#, "[1]"),
    ])
}

/// jq's `$` and `\Z` are the end of the text or just before one final `\n`; with lookaround in the
/// pattern, a lookahead can see that `\n` (`a(?!\n)$` does not match `"a\n"`).
#[test]
fn test_end_anchors_with_lookaround_match_jq_5502() -> Result<()> {
    assert_rows(&[
        (r#""a\n" | [match("(?<=a)$").offset]"#, "[1]"),
        (r#""a\n" | test("a(?!\n)$")"#, "false"),
        (r#""a\n\n" | test("(?<=a)$")"#, "false"),
        (r#""a\n" | [match("(?<=a)\\Z").offset]"#, "[1]"),
        (r#""a\n\n" | test("(?<=a)\\Z")"#, "false"),
        // An escaped `$`, and one inside a class, are the character.
        (r#""a$b" | [match("(?<=a)\\$b").offset]"#, "[1]"),
        (r#""a]$" | [match("(?<=a)[]$]+").string]"#, r#"["]$"]"#),
    ])
}

/// The pattern's lexical structure decides what is rewritten: an `m` in a lookahead or an atomic
/// group is not the multi-line flag, comments are skipped (a `[` there opens no class, a `$` is no
/// anchor), `x` and `m` hold for their own group, and an interval without a lower bound is a
/// literal (all found in review).
#[test]
fn test_fancy_route_follows_the_pattern_structure_5502() -> Result<()> {
    assert_rows(&[
        (r#""am\n" | test("a(?=m)m$")"#, "true"),
        (r#""name\n" | test("(?>name)$")"#, "true"),
        (r#""ab\n" | test("(?<=a)b(?#[)$")"#, "true"),
        (r#""ab\n" | test("(?<=a)b # [x\n$"; "x")"#, "true"),
        (r#""ab" | test("(?#cost $5)ab")"#, "true"),
        (r#""ab" | test("(?<=a)b # \\h\n"; "x")"#, "true"),
        (r#""ab\n" | test("(?<=a)(?x: b # c\n)$")"#, "true"),
        (r#""a b\n" | test("(?<=a)(?x: b)#$")"#, "false"),
        (r#""a\nb" | [match("(?<=a)(?m:$)"; "g").offset]"#, "[1]"),
        (r#""ab{,2}" | test("(?<=a)b{,2}$")"#, "true"),
        (r#""abb" | test("(?<=a)b{,2}$")"#, "false"),
    ])
}

/// jq reports every group of an empty overall match as an empty match at its position, whatever
/// it captured or whether it took part: `capture`, `scan` and `sub`'s `.` are built from `match`'s
/// `captures`, so they show the same `""` (they showed `null` here before). A group inside a
/// lookahead is the case only lookaround can reach.
#[test]
fn test_empty_match_reports_every_group_empty_5502() -> Result<()> {
    assert_rows(&[
        (
            r#""abc" | [match("(?=(b))") | .captures[0] | .offset, .string]"#,
            r#"[1,""]"#,
        ),
        (r#""" | capture("(?<x>a)?")"#, r#"{"x":""}"#),
        (
            r#""b" | [capture("(?<x>a)?"; "g")]"#,
            r#"[{"x":""},{"x":""}]"#,
        ),
        (r#""b" | [scan("(a)?")]"#, r#"[[""],[""]]"#),
        (r#""" | sub("(?<x>a)?"; "[\(.x)]")"#, r#""[]""#),
        // A non-empty match keeps #5363's `null` for a group that took no part.
        (r#""b" | [scan("(a)?b")]"#, "[[null]]"),
        (
            r#""b" | capture("(?<x>a)?(?<y>b)")"#,
            r#"{"x":null,"y":"b"}"#,
        ),
    ])
}

/// Escapes and constructs `fancy-regex` reads differently from Oniguruma, or that were not checked
/// against it, keep the `regex` crate's refusal rather than answering differently from jq
/// (jq 1.7.1: `true`, `true`, `true`, `false`, an error, `false`). A `(?P...)` group never gets this
/// far: #5529 refuses it before either engine, with jq's own message (`tests/jq_cli/i5529.rs`).
#[test]
fn test_constructs_read_differently_stay_refused_5502() -> Result<()> {
    for filter in [
        r#""h" | test("\\h")"#,
        r#""abab" | test("(?<n>ab)\\g<n>")"#,
        r#""abc" | test("(?~b)")"#,
        r#""abab" | test("(ab)\\g1")"#,
        // jq errors on this one; `fancy-regex` would answer it.
        r#""aaa" | test("a{3,2}")"#,
        // Possessive in jq, a repeat of a repeat in `fancy-regex` (jq: `false`).
        r#""aaa" | test("^(?=a)a{2,3}+a$")"#,
    ] {
        let (stdout, stderr, code) = run(filter)?;
        assert_eq!((stdout.as_str(), code), ("", 5), "{filter}");
        assert!(
            stderr.contains("invalid regex: regex parse error"),
            "{filter}: {stderr}"
        );
    }
    Ok(())
}

/// A search that exhausts the backtracking budget, or the backtracking stack, is an error worded
/// as Oniguruma's for its own limits, which `?` and `try` catch like any other. jq itself answers
/// both rows `false`: its optimizer settles them without backtracking, a recorded divergence
/// (`docs/compliance/jq/limitations.md`). Spending the whole budget takes about a second in a
/// debug build, so that row runs once.
#[test]
fn test_backtrack_limit_is_a_catchable_error_5502() -> Result<()> {
    let input = format!("\"{}!\"", "a".repeat(40));
    let (stdout, _, code) = run(&format!(r#"{input} | try test("((?=a)a|aa)*b") catch ."#))?;
    assert_eq!(
        (stdout.as_str(), code),
        ("\"Regex failure: retry-limit-in-match over\"\n", 0)
    );

    // One saved state per character: a million of them fill the stack long before the budget.
    let input = format!("\"{}\"", "a".repeat(1_000_000));
    let filter = r#"test("(?:a(?=a)|a)*b")"#;
    let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(&input))?;
    assert_eq!((stdout.as_str(), code), ("", 5));
    assert!(
        stderr.contains("Regex failure: match-stack limit over"),
        "{stderr}"
    );
    let (stdout, _, code) = run_jq_full(&["-c", &format!("[{filter}?]")], Some(&input))?;
    assert_eq!((stdout.as_str(), code), ("[]\n", 0));
    Ok(())
}

/// Oniguruma refuses a lookbehind whose alternatives have different fixed lengths
/// (`invalid pattern in look-behind`); `fancy-regex` matches it. A recorded divergence
/// (`docs/compliance/jq/limitations.md`): matching would mean reproducing Oniguruma's lookbehind
/// compiler rules.
#[test]
fn test_lookbehind_oniguruma_refuses_is_answered_5502() -> Result<()> {
    assert_rows(&[(r#""aXbX" | test("(?<=a|bb)X")"#, "true")])
}
