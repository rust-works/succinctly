//! #6228: Oniguruma reads a Unicode block name, `\p{In_Basic_Latin}`, as the block's code points;
//! the `regex` crate has no block table. Case, blanks, `_` and `-` do not matter in a name, the
//! surrogate blocks hold no scalar value, and inside a class the `i` flag folds the complement of a
//! negated block, not the block. Every row was captured from jq 1.7.1 (`/usr/bin/jq -n`).

use anyhow::Result;

use crate::run_jq_full;

fn filter(text: &str, pattern: &str, flags: &str) -> String {
    format!(
        r#"{text} | [explode[] | [.] | implode | test("{}"; "{flags}")]"#,
        pattern.replace('\\', "\\\\")
    )
}

#[test]
fn test_block_names_read_as_their_code_points_6228() -> Result<()> {
    for (text, pattern, flags, expected) in [
        (r#""a\u00e9""#, r"\p{In_Basic_Latin}", "", "[true,false]"),
        (r#""a\u00e9""#, r"\P{In_Basic_Latin}", "", "[false,true]"),
        (r#""a\u00e9""#, r"\p{^In_Basic_Latin}", "", "[false,true]"),
        (r#""a\u00e9""#, r"\P{^In_Basic_Latin}", "", "[true,false]"),
        (r#""a\u00e9""#, r"\p{In basic-LATIN}", "", "[true,false]"),
        (r#""a\u00e9""#, r"\p{IN_BASICLATIN}", "", "[true,false]"),
        (r#""a\u00e9""#, r"\p{in latin_1 SUPPLEMENT}", "", "[false,true]"),
        (
            r#""λxж""#,
            r"[\p{In_Greek_and_Coptic}x]",
            "",
            "[true,true,false]",
        ),
        (
            r#""λxж""#,
            r"[^\p{In_Greek_and_Coptic}]",
            "",
            "[false,true,true]",
        ),
        (
            r#""λxж""#,
            r"[\P{In_Greek_and_Coptic}]",
            "",
            "[false,true,true]",
        ),
        (
            r#""λxж""#,
            r"[^\P{In_Greek_and_Coptic}]",
            "",
            "[true,false,false]",
        ),
        (r#""λxж""#, r"[\p{^In_Cyrillic}]", "", "[true,true,false]"),
        // The surrogate blocks hold no scalar value.
        (r#""a""#, r"\p{In_High_Surrogates}", "", "[false]"),
        (r#""a""#, r"\P{In_High_Surrogates}", "", "[true]"),
        (r#""a""#, r"[\p{In_Low_Surrogates}]", "", "[false]"),
        (r#""a""#, r"[\P{In_Low_Surrogates}]", "", "[true]"),
        // The code points no block holds.
        (r#""\u2fe0a""#, r"\p{In_No_Block}", "", "[true,false]"),
        // Outside a class a property is not case-folded, inside one it is.
        (r#""\u212a""#, r"\p{In_Basic_Latin}", "i", "[false]"),
        (r#""K""#, r"[\p{In_Letterlike_Symbols}]", "i", "[true]"),
        (r#""\u00ff""#, r"[^\P{In_Latin_1_Supplement}]", "i", "[false]"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter(text, pattern, flags)], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{pattern} {flags}: stderr={stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_unknown_block_name_is_still_refused_6228() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-nc", r#""a" | test("\\p{In_Foo}")"#], None)?;
    assert_eq!((stdout.as_str(), code), ("", 5), "stderr={stderr:?}");
    Ok(())
}
