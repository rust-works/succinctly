//! #6217: Oniguruma reads `\p{..}` property names its own way. `XDigit` is the ASCII hex digits (the
//! `regex` crate has no such name), and a name with `=` or `:` (`Age=1.1`, `Script=Latin`, `gc=L`)
//! and the properties newer than the Unicode tables jq 1.7.1 links (`Bidi_Mirrored`,
//! `IDS_Unary_Operator`, `ID_Compat_Math_Start`, `ID_Compat_Math_Continue`, `Modifier_Combining_Mark`
//! and their abbreviations) are `invalid character property name {NAME}`, the name as written
//! without a leading `^`. Case, blanks, `_` and `-` do not matter in a name. Every row was captured
//! from jq 1.7.1 (`/usr/bin/jq -n`).

use anyhow::Result;

use crate::run_jq_full;

fn filter(pattern: &str) -> String {
    format!(
        r#""aA5fG_ -\u00e9\u0663" | [explode[] | [.] | implode | test("{}")]"#,
        pattern.replace('\\', "\\\\")
    )
}

#[test]
fn test_xdigit_is_the_ascii_hex_digits_6217() -> Result<()> {
    for (pattern, expected) in [
        (
            r"\p{XDigit}",
            "[true,true,true,true,false,false,false,false,false,false]",
        ),
        (
            r"\P{XDigit}",
            "[false,false,false,false,true,true,true,true,true,true]",
        ),
        (
            r"\p{^XDigit}",
            "[false,false,false,false,true,true,true,true,true,true]",
        ),
        (
            r"\P{^XDigit}",
            "[true,true,true,true,false,false,false,false,false,false]",
        ),
        (
            r"[\p{XDigit}]",
            "[true,true,true,true,false,false,false,false,false,false]",
        ),
        (
            r"[^\p{XDigit}]",
            "[false,false,false,false,true,true,true,true,true,true]",
        ),
        (
            r"[\P{XDigit}]",
            "[false,false,false,false,true,true,true,true,true,true]",
        ),
        (
            r"[a\p{XDigit}z]",
            "[true,true,true,true,false,false,false,false,false,false]",
        ),
        (
            r"\p{xdigit}",
            "[true,true,true,true,false,false,false,false,false,false]",
        ),
        (
            r"\p{X_Digit}",
            "[true,true,true,true,false,false,false,false,false,false]",
        ),
        (
            r"\p{x digit}",
            "[true,true,true,true,false,false,false,false,false,false]",
        ),
        (
            r"\p{ X-Digit }",
            "[true,true,true,true,false,false,false,false,false,false]",
        ),
        (
            r"\p{XDigit}+",
            "[true,true,true,true,false,false,false,false,false,false]",
        ),
        (
            r"(?i)\p{XDigit}",
            "[true,true,true,true,false,false,false,false,false,false]",
        ),
        (
            r"\p{ASCII_Hex_Digit}",
            "[true,true,true,true,false,false,false,false,false,false]",
        ),
        (
            r"\p{Lu}",
            "[false,true,false,false,true,false,false,false,false,false]",
        ),
        (
            r"\p{L u}",
            "[false,true,false,false,true,false,false,false,false,false]",
        ),
        (
            r"\pL",
            "[false,false,false,false,false,false,false,false,false,false]",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter(pattern)], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{pattern}: stderr={stderr:?}"
        );
    }
    Ok(())
}

// The rows are pattern text, whose `{gc:L}` is not a format argument.
#[allow(clippy::literal_string_with_formatting_args)]
#[test]
fn test_a_property_name_oniguruma_refuses_is_refused_6217() -> Result<()> {
    for (pattern, name) in [
        (r"\p{Age=1.1}", "{Age=1.1}"),
        (r"\p{Script=Latin}", "{Script=Latin}"),
        (r"\p{sc=Greek}", "{sc=Greek}"),
        (r"\p{gc=L}", "{gc=L}"),
        (r"\p{gc:L}", "{gc:L}"),
        (r"\p{Script_Extensions=Latin}", "{Script_Extensions=Latin}"),
        (r"\P{Age=1}", "{Age=1}"),
        (r"\p{^Age=1}", "{Age=1}"),
        (r"[\p{Age=1}]", "{Age=1}"),
        (r"\p{Bidi_Mirrored}", "{Bidi_Mirrored}"),
        (r"\p{bidi mirrored}", "{bidi mirrored}"),
        (r"\p{IDS_Unary_Operator}", "{IDS_Unary_Operator}"),
        (r"\p{ID_Compat_Math_Continue}", "{ID_Compat_Math_Continue}"),
        (r"\p{ID_Compat_Math_Start}", "{ID_Compat_Math_Start}"),
        (r"\p{Modifier_Combining_Mark}", "{Modifier_Combining_Mark}"),
        (r"\p{mcm}", "{mcm}"),
        (r"\p{^mcm}", "{mcm}"),
        (r"\p{Modifier-Combining Mark}", "{Modifier-Combining Mark}"),
        (r"\p{ IDSU }", "{ IDSU }"),
        (r"\p{L=}", "{L=}"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter(pattern)], None)?;
        assert_eq!(code, 5, "{pattern}: stdout={stdout:?} stderr={stderr:?}");
        assert_eq!(stdout, "", "{pattern}");
        assert!(
            stderr.contains(&format!(
                "invalid regex: invalid character property name {name}"
            )),
            "{pattern}: {stderr:?}"
        );
    }
    Ok(())
}
