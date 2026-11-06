//! #6426: a loop with capture groups beside a lookaround, an atomic group or a comment answers as
//! Oniguruma does: the loop rewrite and the capture rewrite read the pattern through the copy that writes
//! that syntax plainly, and the text they write runs on `fancy-regex`. Every row was captured from jq 1.7.1
//! (`jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_loop_with_capture_groups_beside_a_lookaround_or_atomic_group_answers_as_jq_does_6426(
) -> Result<()> {
    for (pattern, input, flags, expected) in [
        ("(?=b)((a|b)|){2,}", "ba", "g", r#"[[0,"ba",["","a"]]]"#),
        ("(?=b)((a|b)|){2,}", "bab", "g", r#"[[0,"bab",["","b"]]]"#),
        ("(?=b)((a|b)|){2,}", "abab", "g", r#"[[1,"bab",["","b"]]]"#),
        (
            "(?=b|a)((?:(b|)*|a)+)*",
            "ba",
            "g",
            r#"[[0,"b",["",""]],[1,"",["",""]]]"#,
        ),
        (
            "(?=b|a)((?:(b|)*|a)+)*",
            "bab",
            "g",
            r#"[[0,"b",["",""]],[1,"",["",""]],[2,"b",["",""]]]"#,
        ),
        (
            "(?!c)((a|b)|){2,}",
            "ba",
            "g",
            r#"[[0,"ba",["","a"]],[2,"",["",""]]]"#,
        ),
        (
            "(?!c)((a|b)|){2,}",
            "aab",
            "g",
            r#"[[0,"aab",["","b"]],[3,"",["",""]]]"#,
        ),
        ("(?<=b)((a|b)|){2,}", "ba", "g", r#"[[1,"a",["","a"]]]"#),
        (
            "(?<=b)((a|b)|){2,}",
            "bab",
            "g",
            r#"[[1,"ab",["","b"]],[3,"",["",""]]]"#,
        ),
        ("(?>b)((a|b)|){2,}", "ba", "g", r#"[[0,"ba",["","a"]]]"#),
        ("(?>b)((a|b)|){2,}", "bab", "g", r#"[[0,"bab",["","b"]]]"#),
        (
            "(?#c)((?:b*|a(\\w*))*b*|a){2,}",
            "ba",
            "g",
            r#"[[0,"b",["",null]],[1,"",["",""]],[2,"",["",""]]]"#,
        ),
        (
            "(?#c)((?:b*|a(\\w*))*b*|a){2,}",
            "bab",
            "g",
            r#"[[0,"b",["",null]],[1,"",["",""]],[2,"b",["",null]],[3,"",["",""]]]"#,
        ),
        (
            "((a|b)|){2,}(?=b)",
            "bab",
            "g",
            r#"[[0,"ba",["","a"]],[2,"",["",""]]]"#,
        ),
        ("((a|b)|){2,}(?=b)", "ba", "g", r#"[[0,"",["",""]]]"#),
        (
            "((a|b)|){2,}(?!a)",
            "bab",
            "g",
            r#"[[0,"bab",["","b"]],[3,"",["",""]]]"#,
        ),
        (
            "((a|b)|){2,}(?!a)",
            "ba",
            "g",
            r#"[[0,"ba",["","a"]],[2,"",["",""]]]"#,
        ),
        (
            "(?:(a)?(a*)|(?:(a)|b)){2,}(?!c)",
            "bab",
            "g",
            r#"[[0,"",["","",""]],[1,"a",["a","",null]],[2,"",["","",""]],[3,"",["","",""]]]"#,
        ),
        (
            "(?!c)(?:(a)?(a*)|(?:(a)|b)){2,}",
            "bab",
            "gx",
            r#"[[0,"",["","",""]],[1,"a",["a","",null]],[2,"",["","",""]],[3,"",["","",""]]]"#,
        ),
        (
            "(?=[ab])(?:(b*)|a)*",
            "bab",
            "g",
            r#"[[0,"b",[""]],[1,"",[""]],[2,"b",[""]]]"#,
        ),
        ("(?=b)(?:(a)|b*|c)+$", "bab", "g", r#"[[0,"bab",["a"]]]"#),
        ("(?=b)(?:(a)|b*|c)+$", "aab", "g", r#"[[2,"b",[null]]]"#),
        ("(?=a)(?:(a)|b*|c)+\\Z", "aab", "g", r#"[[0,"aab",["a"]]]"#),
        ("(?=b)(a)?\\1(?:(a)|b*|c)+", "bab", "g", "[]"),
        (
            "(?=(b))(?:(a)|b*|c)+",
            "bab",
            "g",
            r#"[[0,"bab",["b","a"]]]"#,
        ),
        (
            "(?!c)(?:(a)|b*|c)+(?<=b)",
            "abab",
            "g",
            r#"[[0,"abab",["a"]],[4,"",[""]]]"#,
        ),
    ] {
        assert_matches(pattern, input, flags, expected)?;
    }
    Ok(())
}

fn assert_matches(pattern: &str, input: &str, flags: &str, expected: &str) -> Result<()> {
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
    Ok(())
}

/// `test`, `capture`, `scan` and `splits` read the same engine: the captures of a loop beside a
/// lookaround. Every row was captured from jq 1.7.1.
#[test]
fn the_other_regex_builtins_read_the_rewritten_pattern_beside_a_lookaround_6426() -> Result<()> {
    for (filter, expected) in [
        (r#""ba" | test("(?=b)((a|b)|){2,}")"#, "true"),
        (
            r#""ba" | capture("(?=b)(?<x>(?<y>a|b)|){2,}")"#,
            r#"{"x":"","y":"a"}"#,
        ),
        (r#""ba" | [scan("(?=b)((a|b)|){2,}")]"#, r#"[["","a"]]"#),
        (r#""ba" | [splits("(?=b)((a|b)|){2,}")]"#, r#"["",""]"#),
    ] {
        let (stdout, _, status) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!((stdout.trim_end(), status), (expected, 0), "{filter}");
    }
    Ok(())
}
