//! #7852: Oniguruma refuses a backreference or a conditional to a group the pattern does not have
//! in `check_backrefs`, once it has parsed the whole pattern, so every parse error comes first,
//! wherever it stands: `(?:\1` is `end pattern with unmatched parenthesis` and `(?=\1){2}` is
//! `target of repeat operator is invalid`. succinctly reported the backreference first, and a
//! repeated look-ahead only once the rest of the pattern had been read (`(?=a)*[`). The same order
//! holds for the parse errors themselves, each where it stands (`\k<n>(?<n>a)`), and for the repeat
//! checks where an inline `(?x)` hides what a repeat follows (`(?x)(?i)+`).
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on every row.

use anyhow::Result;

use crate::run_jq_full;

/// `(pattern, jq's message)`.
static ROWS: &[(&str, &str)] = &[
    (r"(?:\1", "end pattern with unmatched parenthesis"),
    (r"\1(?=a)*", "target of repeat operator is invalid"),
    (r"(?=\1){2}", "target of repeat operator is invalid"),
    (r"\1\k<n>", "undefined name <n> reference"),
    (r"\1(?#c", "end pattern in group"),
    (r"\2(?<=a(?=b)", "end pattern with unmatched parenthesis"),
    (r"(?(5)a)(", "end pattern with unmatched parenthesis"),
    (r"(?(5)a|b|c)(", "end pattern with unmatched parenthesis"),
    (r"(?(5)a)\k<n>", "undefined name <n> reference"),
    (r"\k<5>(", "end pattern with unmatched parenthesis"),
    (r"\k<+1>(", "end pattern with unmatched parenthesis"),
    (r"(a)\k<-1>(", "end pattern with unmatched parenthesis"),
    // A reference to a group the pattern has was refused in the `regex` crate's words.
    (r"\1(", "end pattern with unmatched parenthesis"),
    (r"\1(a", "end pattern with unmatched parenthesis"),
    (r"(?x)\1(", "end pattern with unmatched parenthesis"),
    // A repeated assertion is refused at its repeat operator, before an error after it.
    (r"(?=a)*[", "target of repeat operator is invalid"),
    // A relative call before the start of the pattern is refused where the parse reads it.
    (r"\g<-3>(", "undefined group <-3> reference"),
    (r"(a)\g<-3>", "undefined group <-3> reference"),
    // With no parse error, the later passes' refusals, in their order.
    (r"(?(5)a)", "invalid backref number/name"),
    (r"\5\g<7>", "invalid backref number/name"),
    (r"\g<7>\5", "invalid backref number/name"),
    (r"\g<-1>", "never ending recursion"),
    (r"\g<+1>", "undefined group <+1> reference"),
    // Each parse error is refused where it stands, before one after it.
    (r"\k<n>(?<n>a)", "undefined name <n> reference"),
    (r"(?(<n>)a)(?<n>b)", "undefined name <n> reference"),
    (r"(?(?=a)b|c)", "target of repeat operator is not specified"),
    (r"(?(1)*)", "target of repeat operator is not specified"),
    (r"(?~{2})", "target of repeat operator is not specified"),
    (r"(?=a)(?#c)*", "target of repeat operator is invalid"),
    (r"(?~", "end pattern in group"),
    (r"(?~|", "end pattern in group"),
    (r"(?(1)", "end pattern in group"),
    (r"(*a(", "end pattern in group"),
    (r"(*\", "end pattern in group"),
    (r"(*{2}", "invalid callout name"),
    // A call that never ends is refused after every call to a group the pattern does not have.
    (r"\g<-1>\g<n>", "undefined name <n> reference"),
    (r"\g<0>\g<5>", "undefined group <5> reference"),
    // The repeat checks see past the blanks an inline `(?x)` turns on, as they do under the `x`
    // flag.
    (r"(?x)(?i)+", "target of repeat operator is not specified"),
    (
        r"(?x)(?s){0,}",
        "target of repeat operator is not specified",
    ),
    (r"(?x)(?x)+", "target of repeat operator is not specified"),
    (r"a(?x)(?i)*", "target of repeat operator is not specified"),
    (r"(?x) (?i) +", "target of repeat operator is not specified"),
    (r"(?x)(?=a)*", "target of repeat operator is invalid"),
    (r"(?x)^*", "target of repeat operator is invalid"),
    (r"*[ (?x)", "target of repeat operator is not specified"),
    // A refusal the parse makes keeps its place.
    (r"(?(0)a)(", "invalid backref number/name"),
    (r"\k<0>(", "invalid backref number/name"),
    (r"\k<-1>(", "invalid backref number/name"),
    (r"\1[", "premature end of char-class"),
    (r"\1)", "unmatched close parenthesis"),
    (r"\1\", "end pattern at escape"),
];

#[test]
fn a_regex_parse_error_comes_before_a_missing_groups_backreference_7852() -> Result<()> {
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
    // The uncaught error, as in the issue.
    let got = run_jq_full(&["-nc", r#""a" | test("(?:\\1")"#], None)?;
    assert_eq!(
        (got.0.as_str(), got.1.as_str(), got.2),
        (
            "",
            "jq: error (at <unknown>): Regex failure: end pattern with unmatched parenthesis\n",
            5
        )
    );
    Ok(())
}

#[test]
fn a_backreference_to_a_group_the_pattern_has_still_matches_7852() -> Result<()> {
    let got = run_jq_full(
        &[
            "-nc",
            r#""aab abb" | [match("(a)(?=a)\\1"; "g") | .string]"#,
        ],
        None,
    )?;
    assert_eq!(
        (got.0.as_str(), got.1.as_str(), got.2),
        ("[\"aa\"]\n", "", 0)
    );
    Ok(())
}

#[test]
fn a_k_or_g_escape_in_a_class_is_the_letter_7852() -> Result<()> {
    let program = r#"["k","<","g","1","z"] | map(test("[\\k<n>]")), map(test("[\\g<1>]"))"#;
    let got = run_jq_full(&["-nc", program], None)?;
    assert_eq!(
        (got.0.as_str(), got.1.as_str(), got.2),
        (
            "[true,true,false,false,false]\n[false,true,true,true,false]\n",
            "",
            0
        )
    );
    Ok(())
}
