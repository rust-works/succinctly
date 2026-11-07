//! #6502: a `^` under `(?m)` inside a look-around is written as Oniguruma reads it too (#6456: it
//! does not hold at the end of a text after a final newline), since the pattern is read through
//! its plain copy, which holds the `^` at the same offset. Every row was captured from jq 1.7.1
//! (`jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_multiline_caret_inside_a_lookaround_does_not_hold_at_the_end_6502() -> Result<()> {
    for (pattern, subject, expected) in [
        ("(?m)(?<!^)", "a\n", "[1,2]"),
        ("(?m)(?<!^)", "\n", "[1]"),
        ("(?m)a\\n(?!^)", "a\n", "[0]"),
        ("(?m)(?<=\\n)(?!^)", "a\n", "[2]"),
        ("(?<!(?m:^))", "a\n", "[1,2]"),
        ("(?m)(?!b|^)", "a\n\n", "[1,3]"),
        ("(?m)(?<!^)(?#c)", "ab\n", "[1,2,3]"),
        // `(?-m)` inside the look-around: the `^` there is the start of the text only.
        ("(?m)(?<!(?-m:^))", "a\n", "[1,2]"),
        // A `^` beside the look-around was already read (#6456), and still is.
        ("(?m)(?=(?:^|x))", "a\n", "[0]"),
    ] {
        let (stdout, stderr, status) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "p",
                pattern,
                "--arg",
                "s",
                subject,
                r#"[$s | match($p; "g") | .offset]"#,
            ],
            None,
        )?;
        assert_eq!(
            (stdout.trim_end(), stderr.as_str(), status),
            (expected, "", 0),
            "{pattern} on {subject:?}"
        );
    }
    Ok(())
}
