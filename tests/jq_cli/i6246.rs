//! #6246: a negated class of bytes alone (no code point, no Unicode class) whose range holds every
//! lead byte, `C2`..`F4`, matches no multibyte character in jq: Oniguruma tests such a class against
//! the first byte of the character, and the bitset names it. The class is written with the whole
//! non-ASCII range added (under `i`, checked after the class, since the range would fold into `k`
//! and `s`). Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`) over the text
//! `kKaé€𝄞ÿ ]^[`.

use anyhow::Result;

use crate::run_jq_full;

fn check(rows: &[(&str, &str)]) -> Result<()> {
    for (filter, expected) in rows {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), *expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

/// A negated class of bytes alone whose range holds the lead bytes `C2`..`F4` is tested against the first byte of a multibyte character, so it matches none; a range below them matches all (the grid of #6246).
#[test]
fn a_negated_class_of_bytes_that_holds_the_lead_bytes_matches_no_multibyte_character_6246(
) -> Result<()> {
    check(&[
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x7F-\\xF5]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x7F-\\xFF]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x7F-\\xBF]"; "g") | .string]"#,
            r#"["k","K","a","é","€","𝄞","ÿ"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xF5]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xBF]"; "g") | .string]"#,
            r#"["k","K","a","é","€","𝄞","ÿ"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x90-\\xF5]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x90-\\xFF]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x90-\\xBF]"; "g") | .string]"#,
            r#"["k","K","a","é","€","𝄞","ÿ"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\xA9-\\xF5]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\xA9-\\xFF]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\xA9-\\xBF]"; "g") | .string]"#,
            r#"["k","K","a","é","€","𝄞","ÿ"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\xBF-\\xF5]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\xBF-\\xFF]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\xBF-\\xBF]"; "g") | .string]"#,
            r#"["k","K","a","é","€","𝄞","ÿ"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x41-\\xF5]"; "g") | .string]"#,
            r#"[" "]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x41-\\xFF]"; "g") | .string]"#,
            r#"[" "]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x41-\\xBF]"; "g") | .string]"#,
            r#"["é","€","𝄞","ÿ"," "]"#,
        ),
    ])
}

/// A member that is a code point (`é`, `\x{E9}`, a `\xC3\xA9` run) or a Unicode class (`\w`, `\p{L}`, `[:alpha:]`) puts the class in the code points, where the negation is of the characters; `[:ascii:]`, `[:xdigit:]`, an ASCII escape and a `]` or `^` member do not.
#[test]
fn a_code_point_or_a_unicode_class_puts_the_class_in_the_code_points_6246() -> Result<()> {
    check(&[
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF\\xC3\\xA9]"; "g") | .string]"#,
            r#"["k","K","a","€","𝄞","ÿ"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFFé]"; "g") | .string]"#,
            r#"["k","K","a","€","𝄞","ÿ"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF\\x{E9}]"; "g") | .string]"#,
            r#"["k","K","a","€","𝄞","ÿ"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF\\w]"; "g") | .string]"#,
            r#"["€","𝄞"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF\\p{L}]"; "g") | .string]"#,
            r#"["€","𝄞"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF[:alpha:]]"; "g") | .string]"#,
            r#"["€","𝄞"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF[:ascii:]]"; "g") | .string]"#,
            r"[]",
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF[:xdigit:]]"; "g") | .string]"#,
            r#"["k","K"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF\\n]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF\\]]"; "g") | .string]"#,
            r#"["k","K","a"," ","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^]\\x80-\\xFF]"; "g") | .string]"#,
            r#"["k","K","a"," ","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF\\^]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF\\s]"; "g") | .string]"#,
            r#"["k","K","a","é","€","𝄞","ÿ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF\\d]"; "g") | .string]"#,
            r#"["k","K","a","é","€","𝄞","ÿ"," ","]","^","["]"#,
        ),
    ])
}

/// Under `i`, `k` and `s` fold to the Kelvin sign and the long s, which are code points: a class holding either (or a range that does) is not tested by the lead byte, one without is.
#[test]
fn under_the_i_option_a_class_with_a_k_or_an_s_has_code_points_6246() -> Result<()> {
    check(&[
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("(?i)[^k\\x80-\\xFF]"; "g") | .string]"#,
            r#"["a","é","€","𝄞","ÿ"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("(?i)[^s\\x80-\\xFF]"; "g") | .string]"#,
            r#"["k","K","a","é","€","𝄞","ÿ"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("(?i)[^a-z\\x80-\\xFF]"; "g") | .string]"#,
            r#"["é","€","𝄞","ÿ"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("(?i)[^i\\x80-\\xFF]"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("(?i)[^\\x80-\\xFF]k"; "g") | .string]"#,
            r#"["kK"]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("(?i)[^\\x80-\\xFF]+"; "g") | .string]"#,
            r#"["kKa"," ]^["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^\\x80-\\xFF]k"; "gi") | .string]"#,
            r#"["kK"]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^i\\x80-\\xFF]"; "gi") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("(?i:[^i\\x80-\\xFF])"; "g") | .string]"#,
            r#"["k","K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^k\\x80-\\xFF]"; "g") | .string]"#,
            r#"["K","a"," ","]","^","["]"#,
        ),
        (
            r#""kKa\u00e9\u20ac\ud834\udd1e\u00ff ]^[" | [match("[^a-z\\x80-\\xFF]"; "g") | .string]"#,
            r#"["K"," ","]","^","["]"#,
        ),
    ])
}
