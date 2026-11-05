//! #6241: Oniguruma writes `X{n,}?` out as `n` plain copies when the body's compiled length times
//! `n` is at most ten, and as a counted loop that leaves at the empty way otherwise (#6128). The
//! length of a body that holds an option group (`(?s:\w)`, `(?-i:\p{L})`, `(?i:b)?`) was not worked
//! out, and a body of unknown length was taken for a long one, so the rewrite gave such a pattern
//! the counted loop where jq writes copies. A bare `\p{L}` is one of them, since `succinctly jq`
//! pins its case sensitivity with the group `(?-i:\p{L})` (#6068). An option group counts as its
//! body (jq answers `(?s:\w)` as `\w`). Every row was captured from jq 1.7.1 (`/usr/bin/jq -n`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_option_group_in_a_lazy_counted_repeat_body_is_counted_as_its_body_6241() -> Result<()> {
    for (pattern, text, flags, expected) in [
        (r"(?:a?|\p{L}){2,}?", "ca", "gn", r#"[[0,"c"],[1,"a"]]"#),
        (
            r"(?:a?|(?-i:\p{L})){2,}?",
            "ca",
            "gn",
            r#"[[0,"c"],[1,"a"]]"#,
        ),
        (
            r"(?:a?|(?i:\p{L})){2,}?",
            "ca",
            "gn",
            r#"[[0,"c"],[1,"a"]]"#,
        ),
        (r"(?:a?|(?s:\w)){2,}?", "ca", "gn", r#"[[0,"c"],[1,"a"]]"#),
        (
            r"(?:a?|(?-i:[a-c])){2,}?",
            "ca",
            "gn",
            r#"[[0,"c"],[1,"a"]]"#,
        ),
        (r"(?:(?i:b)?|a){2,}?", "cab", "gn", r#"[[1,"a"],[2,"b"]]"#),
        (r"(?:(?-i:b)?|a){2,}?", "cab", "gn", r#"[[1,"a"],[2,"b"]]"#),
        (r"(?:(?s:b)?|a){2,}?", "cab", "gn", r#"[[1,"a"],[2,"b"]]"#),
        // A repeat of an option group inside the body is looked through.
        (r"(?:(?s:a)*|\p{L}){2,}?", "ca", "gn", r#"[[0,"ca"]]"#),
        (r"(?:(?s:a)+|b){2,}?", "cab", "gn", r#"[[1,"ab"]]"#),
        (r"(?:(?s:a?)*|\p{L}){2,}?", "ca", "gn", r#"[[0,"ca"]]"#),
        (r"(?:(?s:a){2}|\p{L}){2,}?", "caa", "gn", r#"[[0,"caa"]]"#),
        (r"(?:a?|(?m:\w)){2,}?", "ca", "gn", r#"[[0,"c"],[1,"a"]]"#),
        (r"(?:a?|\d){2,}?", "c1", "gn", r#"[[1,"1"]]"#),
        // The controls: a longer body, a bounded count and the `g`-only reading are unchanged.
        (r"(?:a?|\p{L}){3,}?", "ca", "gn", r#"[[0,"ca"]]"#),
        (r"(?:a?|\p{L}){2,3}?", "ca", "gn", r#"[[0,"ca"]]"#),
        (r"(?:a?|b?|\p{L}){2,}?", "ca", "gn", r#"[[0,"ca"]]"#),
        (r"(?:a?|\p{Lu}){2,}?", "cA", "gn", r#"[[1,"A"]]"#),
        (
            r"(?:a?|\p{L}){2,}?",
            "ca",
            "g",
            r#"[[0,""],[1,"a"],[2,""]]"#,
        ),
    ] {
        let program = format!(
            r#""{text}" | [match("{}"; "{flags}") | [.offset, .string]]"#,
            pattern.replace('\\', "\\\\")
        );
        let (stdout, stderr, code) = run_jq_full(&["-nc", &program], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{pattern} on {text:?} ({flags}): stderr={stderr:?}"
        );
    }
    Ok(())
}
