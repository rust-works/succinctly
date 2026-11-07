//! #6510: jq 1.7.1's Oniguruma searches a pattern that matches only the empty string and ends in
//! `\z`, or `$`/`\Z` on a text with no final newline, at the end of the text alone, and there a
//! look-behind reads no further than one character past the search's start. A look-behind that
//! consumes fails there once two characters follow, and a negative one holds. Every expected
//! answer was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const FILTER: &str =
    r#"$s | [[match($p; "g") | [.offset, (.captures | map(.offset))]], test($p), sub($p; "X")]"#;

fn answer(pattern: &str, subject: &str) -> Result<String> {
    let (stdout, stderr, code) = run_jq_full(
        &["-nc", "--arg", "p", pattern, "--arg", "s", subject, FILTER],
        None,
    )?;
    assert_eq!(
        (code, stderr.as_str()),
        (0, ""),
        "{pattern:?} on {subject:?}"
    );
    Ok(stdout)
}

fn check(rows: &[(&str, &str, &str)]) -> Result<()> {
    for (pattern, subject, want) in rows {
        assert_eq!(
            answer(pattern, subject)?,
            format!("{want}\n"),
            "{pattern:?} on {subject:?}"
        );
    }
    Ok(())
}

/// A look-behind that consumes fails at the end, a negative one holds, at any depth and beside
/// any anchor that reaches the pattern's right edge.
#[test]
fn a_consuming_look_behind_reads_as_the_end_search_does_6510() -> Result<()> {
    check(&[
        (r"(?<=b)\z", "ab", r#"[[],false,"ab"]"#),
        (r"(?<=b)$", "ab", r#"[[],false,"ab"]"#),
        (r"(?<=\w)\z", "ab", r#"[[],false,"ab"]"#),
        (r"(?<=b)\b\z", "ab", r#"[[],false,"ab"]"#),
        (r"\z(?<=b)", "ab", r#"[[],false,"ab"]"#),
        (r"(?<=b)\Z", "ab", r#"[[],false,"ab"]"#),
        (r"(?<=b)\z", "éb", r#"[[],false,"éb"]"#),
        (r"(?<=ab)\z", "ab", r#"[[],false,"ab"]"#),
        (r"(?<=a?b)\z", "ab", r#"[[],false,"ab"]"#),
        (r"(?<=b+)\z", "ab", r#"[[],false,"ab"]"#),
        (r"(?<!b)\z", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?<!\w)\z", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?<=(?<=a)b)\z", "ab", r#"[[],false,"ab"]"#),
        (r"(?<=(?<=b))\z", "ab", r#"[[],false,"ab"]"#),
        (r"(?!(?<=b))\z", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?=(?<=b))\z", "ab", r#"[[],false,"ab"]"#),
        (r"(?:(?<=b)|(?<=a))\z", "ab", r#"[[],false,"ab"]"#),
        (r"(?<=(b))\z", "ab", r#"[[],false,"ab"]"#),
        (r"((?<!b))\z", "ab", r#"[[[2,[2]]],true,"abX"]"#),
        (r"(?:(?<=(b))|())\z", "ab", r#"[[[2,[2,2]]],true,"abX"]"#),
        ("\\z(?<=\\n)", "a\n", r#"[[],false,"a\n"]"#),
        (r"(?m)\z(?<=b)", "a\nb", r#"[[],false,"a\nb"]"#),
        (r"(?i)(?<=B)\z", "ab", r#"[[],false,"ab"]"#),
    ])
}

/// One character after the start leaves the look-behind its text, and a final newline sends a
/// `$`/`\Z` search forward from before it.
#[test]
fn a_short_text_and_a_final_newline_read_as_written_6510() -> Result<()> {
    check(&[
        (r"(?<=b)\z", "b", r#"[[[1,[]]],true,"bX"]"#),
        (r"(?<=é)\z", "é", r#"[[[1,[]]],true,"éX"]"#),
        (r"(?<!b)\z", "b", r#"[[],false,"b"]"#),
        (r"(?<=b)$", "ab\n", r#"[[[2,[]]],true,"abX\n"]"#),
        (r"(?<=b)\Z", "ab\n", r#"[[[2,[]]],true,"abX\n"]"#),
        ("(?<=\\n)$", "ab\n", r#"[[[3,[]]],true,"ab\nX"]"#),
    ])
}

/// Where Oniguruma's search is the usual one: something consumes, the right edge has no anchor
/// every way shares, `$` is a line's end under `(?m)`, or the look-behind consumes nothing on
/// some way.
#[test]
fn the_usual_search_is_left_alone_6510() -> Result<()> {
    check(&[
        (r"(?<=b)(?=\z)", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?<!a)\z", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?<=b)\z|x", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"b(?<=b)\z", "ab", r#"[[[1,[]]],true,"aX"]"#),
        (r"(?m)(?<=b)$", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?<=b)(?:\z|\Z)", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?:(?<=b)\z|(?<=a)\Z)", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?<=\b)\z", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?<=b?)\z", "ab", r#"[[[2,[]]],true,"abX"]"#),
        // A way that matches nothing makes the look-behind an empty node in Oniguruma.
        (r"(?<=b|)\z", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?<=|b)\z", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?<=b|c?)\z", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?<=b|c*)\z", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?<=(?:b|))\z", "ab", r#"[[[2,[]]],true,"abX"]"#),
        (r"(?<=(b)|)\z", "ab", r#"[[[2,[2]]],true,"abX"]"#),
        (r"(?<!b|)\z", "ab", r#"[[],false,"ab"]"#),
        (r"(?<!b?|c)\z", "ab", r#"[[],false,"ab"]"#),
    ])
}
