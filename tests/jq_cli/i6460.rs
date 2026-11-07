//! #6460: a lazy repeat that can match nothing between a greedy `X+` or `X*` and a greedy `X*` or
//! `X+` of the same expression stays lazy on the `fancy-regex` route. `fancy-regex` 0.20 merges the
//! three as `X+(?:M+?X*)?`, which made the lazy one greedy (`"bab" | match("(?=b)b+a*?b*")` was `bab`
//! and is `b`, as in jq). Every row was captured from jq 1.7.1 (`jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_lazy_repeat_between_two_greedy_ones_answers_as_jq_does_on_the_fancy_regex_route_6460(
) -> Result<()> {
    for (pattern, input, expected) in [
        ("(?=b)b+a*?b*", "bab", r#"[[0,"b",[]],[2,"b",[]]]"#),
        ("(?=b)b+a*?b*", "baab", r#"[[0,"b",[]],[3,"b",[]]]"#),
        ("(?=b)b*a*?b*", "bab", r#"[[0,"b",[]],[2,"b",[]]]"#),
        ("(?=b)b+[ab]??b*", "bab", r#"[[0,"b",[]],[2,"b",[]]]"#),
        ("(?=b)b+a{0,3}?b*", "baab", r#"[[0,"b",[]],[3,"b",[]]]"#),
        ("(?=b)(?:b+a*?b*)*", "bab", r#"[[0,"b",[]],[2,"b",[]]]"#),
        (
            "(?!c)(b+)(?:xa)*?(b*)",
            "bxab",
            r#"[[0,"b",["b",""]],[3,"b",["b",""]]]"#,
        ),
        ("(?=b)(?:b+a*?b*)+c", "bab", "[]"),
        ("(?=b)(?:b+a*?b*)+c", "babc", r#"[[0,"babc",[]]]"#),
        ("(?>b)b*a*?b+", "bab", r#"[[0,"bab",[]]]"#),
        ("(?=b)b+a*?b*(?=b|$)", "bab", r#"[[0,"bab",[]]]"#),
    ] {
        let (stdout, _, status) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "s",
                input,
                "--arg",
                "p",
                pattern,
                "[$s | match($p; \"g\") | [.offset, .string, (.captures | map(.string))]]",
            ],
            None,
        )?;
        assert_eq!(
            (stdout.trim_end(), status),
            (expected, 0),
            "{pattern} on {input:?}"
        );
    }
    Ok(())
}

/// The same under the flags that read the text differently (`x`), and with a literal `#`, a chain of
/// the shape and a repeat merged first. Every row was captured from jq 1.7.1.
#[test]
fn the_shape_is_kept_lazy_under_the_extended_flag_and_beside_a_literal_hash_6460() -> Result<()> {
    for (pattern, input, flags, expected) in [
        ("(?=b)b+a*?b*", "bab", "gx", r#"[[0,"b",[]],[2,"b",[]]]"#),
        ("(?=b)b+ a*? b*", "bab", "gx", r#"[[0,"b",[]],[2,"b",[]]]"#),
        (
            "(?=b)b+a*?b*|#",
            "bab#",
            "g",
            r##"[[0,"b",[]],[2,"b",[]],[3,"#",[]]]"##,
        ),
        ("(?=b)b+[#]*?b*", "b#b", "g", r#"[[0,"b",[]],[2,"b",[]]]"#),
        ("(?=a)a+\\d*?a+", "aa1a", "g", r#"[[0,"aa1a",[]]]"#),
        (
            "(?=b)(?:b+)+a*?b*",
            "bab",
            "g",
            r#"[[0,"b",[]],[2,"b",[]]]"#,
        ),
    ] {
        let (stdout, _, status) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "s",
                input,
                "--arg",
                "p",
                pattern,
                "--arg",
                "f",
                flags,
                "[$s | match($p; $f) | [.offset, .string, (.captures | map(.string))]]",
            ],
            None,
        )?;
        assert_eq!(
            (stdout.trim_end(), status),
            (expected, 0),
            "{pattern} on {input:?} under {flags}"
        );
    }
    Ok(())
}
