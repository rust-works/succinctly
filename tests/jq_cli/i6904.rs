//! #6904: `succinctly jq` keeps jq's type-and-value wording for an arithmetic operator over two
//! kinds that do not combine; only `succinctly yq` names the operands by tag. Captured from
//! `/usr/bin/jq` 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_jq_arithmetic_error_keeps_type_and_value_wording_6904() -> Result<()> {
    for (filter, message) in [
        (
            "1.5 - true",
            "number (1.5) and boolean (true) cannot be subtracted",
        ),
        ("[1] * 2", "array ([1]) and number (2) cannot be multiplied"),
        (
            "1 / \"a\"",
            "number (1) and string (\"a\") cannot be divided",
        ),
        (
            "1 % null",
            "number (1) and null (null) cannot be divided (remainder)",
        ),
        ("{} - 1", "object ({}) and number (1) cannot be subtracted"),
        (
            "{} + true",
            "object ({}) and boolean (true) cannot be added",
        ),
    ] {
        let (out, err, code) = run_jq_full(&["-n", filter], None)?;
        assert_eq!(out, "", "{filter}");
        assert_eq!(code, 5, "{filter}");
        assert!(err.contains(message), "{filter}: {err}");
    }
    Ok(())
}
