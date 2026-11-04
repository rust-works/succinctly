//! #6128: a lazy repeat that must run twice or more ends at its body's empty way instead of
//! trying the later copies first, as #6106 did for the greedy ones: `"bab" |
//! match("(?:(?:b|c)||[ab]){2,3}?"; "gn")` is `"b"`, `"ab"` in jq, and was `"b"`, `"a"`, `"b"`.
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_lazy_counted_repeat_ends_at_its_empty_way_6128() -> Result<()> {
    for (filter, expected) in [
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){2,3}?"; "gn") | .string]"#,
            r#"["b","ab"]"#,
        ),
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){2}?"; "gn") | .string]"#,
            r#"["b","ab"]"#,
        ),
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){2,}?"; "gn") | .string]"#,
            r#"["b","ab"]"#,
        ),
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){0,2}?"; "gn") | .string]"#,
            r#"["b","a","b"]"#,
        ),
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){1,3}?"; "gn") | .string]"#,
            r#"["b","a","b"]"#,
        ),
        (
            r#""ba" | [match("(?:(?:a|)|(?:b|c)){2,3}?"; "gn") | .string]"#,
            r#"["ba"]"#,
        ),
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){2,3}?"; "g") | .string]"#,
            r#"["b","","b",""]"#,
        ),
        (
            r#""ba" | [match("(?:(?:a|)|[ab]){3,}?"; "gn") | .string]"#,
            r#"["ba"]"#,
        ),
        (
            r#""aabbcc" | [match("(?:(?:a|)[ab]|(?:a|)|\\w){3,}?"; "gn") | .string]"#,
            r#"["aabb","c","c"]"#,
        ),
        (
            r#""bab" | [match("(?:b*|a){2,}?"; "gn") | .string]"#,
            r#"["b","ab"]"#,
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}
