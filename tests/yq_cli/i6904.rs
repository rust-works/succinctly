//! #6904: an arithmetic operator over two kinds that do not combine words its error as yq does, by
//! tag (`!!float cannot be added to !!null`), where `succinctly yq` raised jq's type-and-value
//! wording (`number (1.5) and null (null) cannot be added`). Every row below was captured from
//! yq v4.53.3 over all seven kinds (`!!int`, `!!float`, `!!str`, `!!bool`, `!!null`, `!!seq`,
//! `!!map`), both operand orders, for `+ - * / %`; a row is kept only when both agree.
//!
//! `*` names the operands left to right whichever side a float is on (`.f * .s` is `cannot multiply
//! !!float with !!str`; it used to name the string first). A container, or a `null` on the left of
//! `/` and `%`, makes yq write each operand's key after its tag (`!!seq (l) cannot be added to a
//! !!float (f)`); the evaluator does not know a node's key, so `succinctly yq` writes `()` there,
//! as it already did for `-` on a string (#5863) and for a timestamp (#5930). That is the one
//! difference left, and it is pinned in [`KEYED_ROWS`] so a change to it is deliberate.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOCUMENT: &str = "f: 1.5\ni: 2\nn: null\ns: x\nl: [1]\nm: {k: 1}\nt: true\n";

/// (filter, yq's error message): the wording matches yq byte for byte.
const ROWS: &[(&str, &str)] = &[
    (".f + .n", "!!float cannot be added to !!null"),
    (".f + .t", "!!float cannot be added to !!bool"),
    (".i + .n", "!!int cannot be added to !!null"),
    (".i + .t", "!!int cannot be added to !!bool"),
    (".t + .f", "!!bool cannot be added to !!float"),
    (".t + .i", "!!bool cannot be added to !!int"),
    (".t + .n", "!!bool cannot be added to !!null"),
    (".t + .t", "!!bool cannot be added to !!bool"),
    (".f - .n", "!!float cannot be added to !!null"),
    (".f - .s", "!!float cannot be added to !!str"),
    (".f - .t", "!!float cannot be added to !!bool"),
    (".i - .n", "!!int cannot be added to !!null"),
    (".i - .s", "!!int cannot be added to !!str"),
    (".i - .t", "!!int cannot be added to !!bool"),
    (".s - .f", "strings cannot be subtracted"),
    (".s - .i", "strings cannot be subtracted"),
    (".s - .n", "strings cannot be subtracted"),
    (".s - .s", "strings cannot be subtracted"),
    (".s - .t", "strings cannot be subtracted"),
    (".m - .f", "maps not yet supported for subtraction"),
    (".m - .i", "maps not yet supported for subtraction"),
    (".m - .n", "maps not yet supported for subtraction"),
    (".m - .s", "maps not yet supported for subtraction"),
    (".m - .l", "maps not yet supported for subtraction"),
    (".m - .m", "maps not yet supported for subtraction"),
    (".m - .t", "maps not yet supported for subtraction"),
    (".t - .f", "!!bool cannot be added to !!float"),
    (".t - .i", "!!bool cannot be added to !!int"),
    (".t - .n", "!!bool cannot be added to !!null"),
    (".t - .s", "!!bool cannot be added to !!str"),
    (".t - .t", "!!bool cannot be added to !!bool"),
    (".f * .s", "cannot multiply !!float with !!str"),
    (".f * .l", "cannot multiply !!float with !!seq"),
    (".f * .m", "cannot multiply !!float with !!map"),
    (".f * .t", "cannot multiply !!float with !!bool"),
    (".i * .l", "cannot multiply !!int with !!seq"),
    (".i * .m", "cannot multiply !!int with !!map"),
    (".i * .t", "cannot multiply !!int with !!bool"),
    (".n * .f", "cannot multiply !!null with !!float"),
    (".n * .i", "cannot multiply !!null with !!int"),
    (".n * .s", "cannot multiply !!null with !!str"),
    (".n * .t", "cannot multiply !!null with !!bool"),
    (".s * .f", "cannot multiply !!str with !!float"),
    (".s * .s", "cannot multiply !!str with !!str"),
    (".s * .l", "cannot multiply !!str with !!seq"),
    (".s * .m", "cannot multiply !!str with !!map"),
    (".s * .t", "cannot multiply !!str with !!bool"),
    (".l * .f", "cannot multiply !!seq with !!float"),
    (".l * .i", "cannot multiply !!seq with !!int"),
    (".l * .s", "cannot multiply !!seq with !!str"),
    (".l * .m", "cannot multiply !!seq with !!map"),
    (".l * .t", "cannot multiply !!seq with !!bool"),
    (".m * .f", "cannot multiply !!map with !!float"),
    (".m * .i", "cannot multiply !!map with !!int"),
    (".m * .s", "cannot multiply !!map with !!str"),
    (".m * .l", "cannot multiply !!map with !!seq"),
    (".m * .t", "cannot multiply !!map with !!bool"),
    (".t * .f", "cannot multiply !!bool with !!float"),
    (".t * .i", "cannot multiply !!bool with !!int"),
    (".t * .s", "cannot multiply !!bool with !!str"),
    (".t * .l", "cannot multiply !!bool with !!seq"),
    (".t * .m", "cannot multiply !!bool with !!map"),
    (".t * .t", "cannot multiply !!bool with !!bool"),
    (".f / .n", "!!float cannot be divided by !!null"),
    (".f / .s", "!!float cannot be divided by !!str"),
    (".f / .t", "!!float cannot be divided by !!bool"),
    (".i / .n", "!!int cannot be divided by !!null"),
    (".i / .s", "!!int cannot be divided by !!str"),
    (".i / .t", "!!int cannot be divided by !!bool"),
    (".s / .f", "!!str cannot be divided by !!float"),
    (".s / .i", "!!str cannot be divided by !!int"),
    (".s / .n", "!!str cannot be divided by !!null"),
    (".s / .t", "!!str cannot be divided by !!bool"),
    (".t / .f", "!!bool cannot be divided by !!float"),
    (".t / .i", "!!bool cannot be divided by !!int"),
    (".t / .n", "!!bool cannot be divided by !!null"),
    (".t / .s", "!!bool cannot be divided by !!str"),
    (".t / .t", "!!bool cannot be divided by !!bool"),
    (".f % .n", "!!float cannot modulo by !!null"),
    (".f % .s", "!!float cannot modulo by !!str"),
    (".f % .t", "!!float cannot modulo by !!bool"),
    (".i % .n", "!!int cannot modulo by !!null"),
    (".i % .s", "!!int cannot modulo by !!str"),
    (".i % .t", "!!int cannot modulo by !!bool"),
    (".s % .f", "!!str cannot modulo by !!float"),
    (".s % .i", "!!str cannot modulo by !!int"),
    (".s % .n", "!!str cannot modulo by !!null"),
    (".s % .s", "!!str cannot modulo by !!str"),
    (".s % .t", "!!str cannot modulo by !!bool"),
    (".t % .f", "!!bool cannot modulo by !!float"),
    (".t % .i", "!!bool cannot modulo by !!int"),
    (".t % .n", "!!bool cannot modulo by !!null"),
    (".t % .s", "!!bool cannot modulo by !!str"),
    (".t % .t", "!!bool cannot modulo by !!bool"),
];

/// (filter, `succinctly yq`'s error message): yq's wording with every `(key)` written `()`.
const KEYED_ROWS: &[(&str, &str)] = &[
    (".f + .l", "!!seq () cannot be added to a !!float ()"),
    (".f + .m", "!!map () cannot be added to a !!float ()"),
    (".i + .l", "!!seq () cannot be added to a !!int ()"),
    (".i + .m", "!!map () cannot be added to a !!int ()"),
    (".s + .l", "!!seq () cannot be added to a !!str ()"),
    (".s + .m", "!!map () cannot be added to a !!str ()"),
    (".m + .f", "!!float () cannot be added to a !!map ()"),
    (".m + .i", "!!int () cannot be added to a !!map ()"),
    (".m + .n", "!!null () cannot be added to a !!map ()"),
    (".m + .s", "!!str () cannot be added to a !!map ()"),
    (".m + .l", "!!seq () cannot be added to a !!map ()"),
    (".m + .t", "!!bool () cannot be added to a !!map ()"),
    (".t + .l", "!!seq () cannot be added to a !!bool ()"),
    (".t + .m", "!!map () cannot be added to a !!bool ()"),
    (".f - .l", "!!seq () cannot be subtracted from !!float"),
    (".f - .m", "!!map () cannot be subtracted from !!float"),
    (".i - .l", "!!seq () cannot be subtracted from !!int"),
    (".i - .m", "!!map () cannot be subtracted from !!int"),
    (".s - .l", "!!seq () cannot be subtracted from !!str"),
    (".s - .m", "!!map () cannot be subtracted from !!str"),
    (".l - .f", "!!float () cannot be subtracted from !!seq"),
    (".l - .i", "!!int () cannot be subtracted from !!seq"),
    (".l - .n", "!!null () cannot be subtracted from !!seq"),
    (".l - .s", "!!str () cannot be subtracted from !!seq"),
    (".l - .m", "!!map () cannot be subtracted from !!seq"),
    (".l - .t", "!!bool () cannot be subtracted from !!seq"),
    (".t - .l", "!!seq () cannot be subtracted from !!bool"),
    (".t - .m", "!!map () cannot be subtracted from !!bool"),
    (".f / .l", "!!float () cannot be divided by !!seq ()"),
    (".f / .m", "!!float () cannot be divided by !!map ()"),
    (".i / .l", "!!int () cannot be divided by !!seq ()"),
    (".i / .m", "!!int () cannot be divided by !!map ()"),
    (".n / .f", "!!null () cannot be divided by !!float ()"),
    (".n / .i", "!!null () cannot be divided by !!int ()"),
    (".n / .n", "!!null () cannot be divided by !!null ()"),
    (".n / .s", "!!null () cannot be divided by !!str ()"),
    (".n / .l", "!!null () cannot be divided by !!seq ()"),
    (".n / .m", "!!null () cannot be divided by !!map ()"),
    (".n / .t", "!!null () cannot be divided by !!bool ()"),
    (".s / .l", "!!str () cannot be divided by !!seq ()"),
    (".s / .m", "!!str () cannot be divided by !!map ()"),
    (".l / .f", "!!seq () cannot be divided by !!float ()"),
    (".l / .i", "!!seq () cannot be divided by !!int ()"),
    (".l / .n", "!!seq () cannot be divided by !!null ()"),
    (".l / .s", "!!seq () cannot be divided by !!str ()"),
    (".l / .l", "!!seq () cannot be divided by !!seq ()"),
    (".l / .m", "!!seq () cannot be divided by !!map ()"),
    (".l / .t", "!!seq () cannot be divided by !!bool ()"),
    (".m / .f", "!!map () cannot be divided by !!float ()"),
    (".m / .i", "!!map () cannot be divided by !!int ()"),
    (".m / .n", "!!map () cannot be divided by !!null ()"),
    (".m / .s", "!!map () cannot be divided by !!str ()"),
    (".m / .l", "!!map () cannot be divided by !!seq ()"),
    (".m / .m", "!!map () cannot be divided by !!map ()"),
    (".m / .t", "!!map () cannot be divided by !!bool ()"),
    (".t / .l", "!!bool () cannot be divided by !!seq ()"),
    (".t / .m", "!!bool () cannot be divided by !!map ()"),
    (".f % .l", "!!float () cannot modulo by !!seq ()"),
    (".f % .m", "!!float () cannot modulo by !!map ()"),
    (".i % .l", "!!int () cannot modulo by !!seq ()"),
    (".i % .m", "!!int () cannot modulo by !!map ()"),
    (".n % .f", "!!null () cannot modulo by !!float ()"),
    (".n % .i", "!!null () cannot modulo by !!int ()"),
    (".n % .n", "!!null () cannot modulo by !!null ()"),
    (".n % .s", "!!null () cannot modulo by !!str ()"),
    (".n % .l", "!!null () cannot modulo by !!seq ()"),
    (".n % .m", "!!null () cannot modulo by !!map ()"),
    (".n % .t", "!!null () cannot modulo by !!bool ()"),
    (".s % .l", "!!str () cannot modulo by !!seq ()"),
    (".s % .m", "!!str () cannot modulo by !!map ()"),
    (".l % .f", "!!seq () cannot modulo by !!float ()"),
    (".l % .i", "!!seq () cannot modulo by !!int ()"),
    (".l % .n", "!!seq () cannot modulo by !!null ()"),
    (".l % .s", "!!seq () cannot modulo by !!str ()"),
    (".l % .l", "!!seq () cannot modulo by !!seq ()"),
    (".l % .m", "!!seq () cannot modulo by !!map ()"),
    (".l % .t", "!!seq () cannot modulo by !!bool ()"),
    (".m % .f", "!!map () cannot modulo by !!float ()"),
    (".m % .i", "!!map () cannot modulo by !!int ()"),
    (".m % .n", "!!map () cannot modulo by !!null ()"),
    (".m % .s", "!!map () cannot modulo by !!str ()"),
    (".m % .l", "!!map () cannot modulo by !!seq ()"),
    (".m % .m", "!!map () cannot modulo by !!map ()"),
    (".m % .t", "!!map () cannot modulo by !!bool ()"),
    (".t % .l", "!!bool () cannot modulo by !!seq ()"),
    (".t % .m", "!!bool () cannot modulo by !!map ()"),
];

fn assert_rows(rows: &[(&str, &str)]) -> Result<()> {
    for (filter, message) in rows {
        let (out, err, code) = run_yq_stdin_with_stderr(filter, DOCUMENT, &[])?;
        assert_eq!(
            (code, out.as_str(), err.trim_end()),
            (1, "", format!("Error: {message}").as_str()),
            "{filter}"
        );
    }
    Ok(())
}

#[test]
fn test_arithmetic_error_names_kinds_by_tag_as_yq_does_6904() -> Result<()> {
    assert_rows(ROWS)
}

#[test]
fn test_arithmetic_error_keyed_tags_carry_empty_parentheses_6904() -> Result<()> {
    assert_rows(KEYED_ROWS)
}

/// `.f * "ab"` and `"ab" * .f` refuse a fractional count and name the operands in the order written.
#[test]
fn test_fractional_string_repeat_count_names_operands_in_written_order_6904() -> Result<()> {
    for (filter, message) in [
        ("2.5 * \"ab\"", "cannot multiply !!float with !!str"),
        ("\"ab\" * 2.5", "cannot multiply !!str with !!float"),
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(filter, "null\n", &[])?;
        assert_eq!(
            (code, out.as_str(), err.trim_end()),
            (1, "", format!("Error: {message}").as_str()),
            "{filter}"
        );
    }
    Ok(())
}

/// The compound assignments route through the same arithmetic.
#[test]
fn test_compound_assignment_error_uses_the_same_wording_6904() -> Result<()> {
    for (filter, message) in [
        (".f += .n", "!!float cannot be added to !!null"),
        (".i -= .t", "!!int cannot be added to !!bool"),
        (".f *= .s", "cannot multiply !!float with !!str"),
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(filter, DOCUMENT, &[])?;
        assert_eq!(
            (code, out.as_str(), err.trim_end()),
            (1, "", format!("Error: {message}").as_str()),
            "{filter}"
        );
    }
    Ok(())
}
