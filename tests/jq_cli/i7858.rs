//! #7858: a reference to a name or group the pattern does not have (`\k<n>`, `\g<1>`) after or
//! inside a look-around, an atomic group, an absent operator or a conditional is jq's `undefined
//! name <n> reference` (or `undefined group <1> reference`). succinctly answered in the `regex`
//! crate's words (`look-around, including look-ahead and look-behind, is not supported`, `unrecognized
//! flag`), because that parse stopped at the look-around before it reached the reference.
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on every row.

use anyhow::Result;

use crate::run_jq_full;

/// `(pattern, jq's message)`.
static ROWS: &[(&str, &str)] = &[
    (r"(?=a)\k<n>", "undefined name <n> reference"),
    (r"(?=\k<n>)", "undefined name <n> reference"),
    (r"(?!\k<n>)", "undefined name <n> reference"),
    (r"(?<=a+)\k<x>", "undefined name <x> reference"),
    (r"(?=a)\g<1>", "undefined group <1> reference"),
    (r"(?>a)\k<n>", "undefined name <n> reference"),
    (r"(?>\g<n>)", "undefined name <n> reference"),
    (r"(?~\k<n>)", "undefined name <n> reference"),
    (r"(?~a)\k<n>", "undefined name <n> reference"),
    (r"(?~a)\g<2>", "undefined group <2> reference"),
    (r"(a)(?(1)b)\k<n>", "undefined name <n> reference"),
    (r"(?x)(?=a)\k<n>", "undefined name <n> reference"),
    (r"(?#c)\g<1>", "undefined group <1> reference"),
    (r"a(?#c)\g<n>(", "end pattern with unmatched parenthesis"),
    (r"(?=a{3,2})", "upper is smaller than lower in repeat range"),
    // A look-behind's refusal waits for them too (#7844).
    (r"(?<=\k<n>(?=a))", "undefined name <n> reference"),
    (r"(?<=(?=a))(?<n>a)\k<m>", "undefined name <m> reference"),
    (r"(?<=(?=a))\g<1>", "undefined group <1> reference"),
];

#[test]
fn an_undefined_reference_beside_a_look_around_is_jqs_refusal_7858() -> Result<()> {
    for &(pattern, message) in ROWS {
        let program = format!(
            "\"a\" | try test({}) catch .",
            serde_json::to_string(pattern)?
        );
        let got = run_jq_full(&["-nc", &program], None)?;
        let want = format!("\"Regex failure: {message}\"\n");
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (want.as_str(), "", 0),
            "{pattern}"
        );
    }
    Ok(())
}

#[test]
fn a_reference_to_a_name_the_pattern_has_beside_a_look_around_still_matches_7858() -> Result<()> {
    let got = run_jq_full(
        &[
            "-nc",
            r#""aab abb" | [match("(?<n>a)(?=a)\\k<n>"; "g") | .string]"#,
        ],
        None,
    )?;
    assert_eq!(
        (got.0.as_str(), got.1.as_str(), got.2),
        ("[\"aa\"]\n", "", 0)
    );
    Ok(())
}
