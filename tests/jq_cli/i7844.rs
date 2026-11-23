//! #7844: Oniguruma judges a look-behind's body (`invalid pattern in look-behind`) only once it has
//! parsed the whole pattern and checked its backreferences and calls, so any error those find,
//! wherever it stands, is the one jq reports: `\1(?<=(?=a))` is `invalid backref number/name`, and
//! `(?<=a(?=b)` is `end pattern with unmatched parenthesis`. succinctly refused the look-behind as
//! soon as it read it, or as soon as it had read the pattern, before the parse errors after it.
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on every row.

use anyhow::Result;

use crate::run_jq_full;

/// `(pattern, jq's message)`.
static ROWS: &[(&str, &str)] = &[
    (r"\1(?<=(?=a))", "invalid backref number/name"),
    (r"(?<=\1(?=a))", "invalid backref number/name"),
    (r"(?<=(?=a))\1", "invalid backref number/name"),
    (r"(?<=(?!a)\2)", "invalid backref number/name"),
    (r"(?<=a|bc(?=x))\1", "invalid backref number/name"),
    (r"(?<=\X)\1", "invalid backref number/name"),
    (r"(?<=a|bc)\1", "invalid backref number/name"),
    (r"(?<=a*)\1", "invalid backref number/name"),
    // An undefined `\k<name>` or `\g<n>` beside a look-around, `(?<=\k<n>(?=a))` and its kin, is
    // pinned in `i7858.rs`.
    (r"(?<=a(?=b)", "end pattern with unmatched parenthesis"),
    (r"(?<=a+)(", "end pattern with unmatched parenthesis"),
    (r"(?<=(?=a)))", "unmatched close parenthesis"),
    (r"(?<=a+))", "unmatched close parenthesis"),
    (r"(?<!a(?=b))[", "premature end of char-class"),
    (r"(?<=(?=a))\", "end pattern at escape"),
    (r"(?<=(?=a))*", "target of repeat operator is invalid"),
    // With nothing else wrong, the look-behind's own refusal.
    (r"(?<=(?=a))", "invalid pattern in look-behind"),
    (r"(a)(?<=(?=a))\1", "invalid pattern in look-behind"),
    (r"[(?<=]\1(?<=a+)", "invalid backref number/name"),
];

#[test]
fn a_regex_error_jq_meets_while_parsing_comes_before_a_look_behind_refusal_7844() -> Result<()> {
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
    let got = run_jq_full(&["-nc", r#""a" | test("\\1(?<=(?=a))")"#], None)?;
    assert_eq!(
        (got.0.as_str(), got.1.as_str(), got.2),
        (
            "",
            "jq: error (at <unknown>): Regex failure: invalid backref number/name\n",
            5
        )
    );
    Ok(())
}
