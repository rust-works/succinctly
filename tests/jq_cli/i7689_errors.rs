//! Found by the wider regex fuzz of #7689: regex compile errors that jq words or orders otherwise.
//! A reference or call with no name is `group name is empty`, a sign after `(?` that opens no group
//! is `undefined group option`, a callout by name has Oniguruma's callout errors, an empty property
//! name is refused where it stands, a `)` that closes no group is refused where Oniguruma reads it
//! (after any earlier error, before a later one or a late backreference check), and the possessive
//! scan's own refusal never hides an error jq makes. Each row is captured from jq 1.7.1
//! (`/usr/bin/jq`) and the same in jq 1.8.2: 27 of 35 fail before the change, the rest are
//! controls.

use anyhow::Result;

use crate::run_jq_full;

/// `"ab" | test(re)` as text, or the caught error.
const PROGRAM: &str = r#". as $r | "ab" | try (test($r.re) | tostring) catch ."#;

/// `(row, jq 1.7.1's output)`.
const ROWS: &[(&str, &str)] = &[
    (
        r#"{"re": "\\k<"}"#,
        r#""Regex failure: group name is empty""#,
    ),
    (
        r#"{"re": "\\k<>"}"#,
        r#""Regex failure: group name is empty""#,
    ),
    (
        r#"{"re": "\\g<>"}"#,
        r#""Regex failure: group name is empty""#,
    ),
    (
        r#"{"re": "\\k''"}"#,
        r#""Regex failure: group name is empty""#,
    ),
    (
        r#"{"re": "(?'"}"#,
        r#""Regex failure: group name is empty""#,
    ),
    (
        r#"{"re": "(?}"}"#,
        r#""Regex failure: undefined group option""#,
    ),
    (
        r#"{"re": "(??"}"#,
        r#""Regex failure: undefined group option""#,
    ),
    (
        r#"{"re": "(?|"}"#,
        r#""Regex failure: undefined group option""#,
    ),
    (
        r#"{"re": "(?["}"#,
        r#""Regex failure: undefined group option""#,
    ),
    (
        r#"{"re": "(? a)"}"#,
        r#""Regex failure: undefined group option""#,
    ),
    (
        r#"{"re": "(?$)"}"#,
        r#""Regex failure: undefined group option""#,
    ),
    (
        r#"{"re": "(*"}"#,
        r#""Regex failure: invalid callout pattern""#,
    ),
    (
        r#"{"re": "(*)"}"#,
        r#""Regex failure: invalid callout name""#,
    ),
    (
        r#"{"re": "(*a)"}"#,
        r#""Regex failure: undefined callout name""#,
    ),
    (
        r#"{"re": "(*1)"}"#,
        r#""Regex failure: invalid callout name""#,
    ),
    (
        r#"{"re": "(*a"}"#,
        r#""Regex failure: end pattern in group""#,
    ),
    (
        r#"{"re": "(*a b)"}"#,
        r#""Regex failure: invalid callout name""#,
    ),
    (
        r#"{"re": "x(*"}"#,
        r#""Regex failure: invalid callout pattern""#,
    ),
    (r#"{"re": "(*FAIL)"}"#, r#""false""#),
    (
        r#"{"re": ")\\"}"#,
        r#""Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re": "a)\\"}"#,
        r#""Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re": "\\2)"}"#,
        r#""Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re": "\\2\\"}"#,
        r#""Regex failure: end pattern at escape""#,
    ),
    (
        r#"{"re": "\\p{}"}"#,
        r#""Regex failure: invalid character property name {}""#,
    ),
    (
        r#"{"re": "\\p{})"}"#,
        r#""Regex failure: invalid character property name {}""#,
    ),
    (
        r#"{"re": "\\p{^}("}"#,
        r#""Regex failure: invalid character property name {}""#,
    ),
    (
        r#"{"re": "*+a"}"#,
        r#""Regex failure: target of repeat operator is not specified""#,
    ),
    (
        r#"{"re": "*+?"}"#,
        r#""Regex failure: target of repeat operator is not specified""#,
    ),
    (
        r#"{"re": "(?i)*+c"}"#,
        r#""Regex failure: target of repeat operator is not specified""#,
    ),
    (r#"{"re": "a*+b"}"#, r#""true""#),
    (r#"{"re": "(?:a)"}"#, r#""true""#),
    (
        r#"{"re": "\\k<n>"}"#,
        r#""Regex failure: undefined name <n> reference""#,
    ),
    (r#"{"re": "(?<n>a)\\k<n>"}"#, r#""false""#),
    (
        r#"{"re": "a)"}"#,
        r#""Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re": "(a"}"#,
        r#""Regex failure: end pattern with unmatched parenthesis""#,
    ),
];

#[test]
fn regex_compile_errors_are_worded_and_ordered_as_in_jq_7689() -> Result<()> {
    let input: String = ROWS.iter().map(|(row, _)| format!("{row}\n")).collect();
    let (stdout, stderr, code) = run_jq_full(&["-c", PROGRAM], Some(&input))?;
    assert_eq!((code, stderr.as_str()), (0, ""));
    let got: Vec<&str> = stdout.lines().collect();
    assert_eq!(got.len(), ROWS.len(), "{stdout}");
    for ((row, expected), got) in ROWS.iter().zip(got) {
        assert_eq!(got, *expected, "{row}");
    }
    Ok(())
}
