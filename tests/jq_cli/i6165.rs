//! #6165: a stacked repeat (`a**`, `a+*`, `a{2}*`) on a pattern the `regex` crate refuses reached
//! `fancy-regex` through an inspection parse that read it in the crate's default dialect, which
//! refuses the second repeat, while the builder that follows takes it in Oniguruma mode. The parse
//! now uses the builder's mode. Every row was captured from jq 1.7.1 (`/usr/bin/jq -n`): the
//! `[offset, length]` of each global match over two inputs.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_stacked_repeat_on_a_fancy_routed_pattern_matches_as_jq_does_6165() -> Result<()> {
    for (pattern, on_aab, on_empty) in [
        (r"(?:\Ka)**", "[[1,1],[2,0],[3,0]]", "[[0,0]]"),
        (r"(?:\Ka)+*", "[[1,1],[2,0],[3,0]]", "[[0,0]]"),
        (r"(?=a)a**", "[[0,2]]", "[]"),
        (r"(?=a)a+*", "[[0,2]]", "[]"),
        (r"(?=a)a?*", "[[0,2]]", "[]"),
        (r"(?=a)a{2}*", "[[0,2]]", "[]"),
        (r"(?=a)a*{2}", "[[0,2]]", "[]"),
        (r"(?=a)a{1,2}{2}", "[[0,2]]", "[]"),
        (r"(?=a)(a)**", "[[0,2]]", "[]"),
        (r"(?<=a)b**", "[[1,0],[2,1]]", "[]"),
        (r"(?!c)a+*", "[[0,2],[2,0],[3,0]]", "[[0,0]]"),
        (r"(?>a)**", "[[0,2],[2,0],[3,0]]", "[[0,0]]"),
        (r"(?>a)b?*", "[[0,1],[1,2]]", "[]"),
        (r"x?(?=a)a{2}+", "[[0,2]]", "[]"),
        (r"(?:(?=a)a)**b", "[[0,3]]", "[]"),
        (r"(?=a)a??*", "[[0,0],[1,0]]", "[]"),
        (r"(?=a)a+?*", "[[0,2]]", "[]"),
        (r"(?=a)a*?{2}", "[[0,0],[1,0]]", "[]"),
    ] {
        for (text, expected) in [("aab", on_aab), ("", on_empty)] {
            let (stdout, stderr, code) = run_jq_full(
                &[
                    "-nr",
                    "--arg",
                    "p",
                    pattern,
                    "--arg",
                    "t",
                    text,
                    r#"$t | [match($p; "g") | [.offset, .length]] | tostring"#,
                ],
                None,
            )?;
            assert_eq!(
                (stdout.trim_end(), code),
                (expected, 0),
                "{pattern} on {text:?}: stderr={stderr:?}"
            );
        }
    }
    Ok(())
}
