//! #6275: a computed integer (`1+1`, `length`, `floor`, a negation) is a double in jq, so it compares as
//! one against a decimal literal whose exact value is not its double (`2.00000000000000000001`, `1e-400`);
//! two literals still compare exactly. `Int` was read as a literal, so a computed integer was compared
//! by its exact decimal digits. Each row was run through jq 1.7.1 (`/usr/bin/jq -nc`). The rows left out
//! are the ones that cross the reindex bridge (`[2.00000000000000000001,(1+1)] | unique`, a `map(.)`
//! over a computed integer), which re-bakes a bare `Int` as a literal (see
//! `docs/compliance/jq/limitations.md`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_computed_integer_compares_as_a_double_against_a_decimal_literal_6275() -> Result<()> {
    for (filter, expected) in [
        (r"(1+1) == 2.00000000000000000001", "true"),
        (r"(1+1) < 2.00000000000000000001", "false"),
        (r"(1+1) > 2.00000000000000000001", "false"),
        (r"2.00000000000000000001 == (1+1)", "true"),
        (r"2.00000000000000000001 > (1+1)", "false"),
        (r"(0+0) == 1e-400", "true"),
        (r"1e-400 > (0+0)", "false"),
        (r"1e-400 <= (0+0)", "true"),
        (r"2 == 2.00000000000000000001", "false"),
        (r"2 < 2.00000000000000000001", "true"),
        (r"0 == 1e-400", "false"),
        (r#"("2"|tonumber) == 2.00000000000000000001"#, "false"),
        (r#"({"a":2}|.a) == 2.00000000000000000001"#, "false"),
        (r"([1,2]|length) == 2.00000000000000000001", "true"),
        (r"(2.5|floor) == 2.00000000000000000001", "true"),
        (r"(1.5|ceil) == 2.00000000000000000001", "true"),
        (r"(4|sqrt|floor) == 2.00000000000000000001", "true"),
        (r"(-(-2)) == 2.00000000000000000001", "true"),
        (r"(-2|-.) == 2.00000000000000000001", "true"),
        (r"(9007199254740992+0) == 9007199254740993", "true"),
        (r"(9007199254740993+0) == 9007199254740993", "true"),
        (r"9007199254740992 == 9007199254740993", "false"),
        (r"(9007199254740992+0) < 9007199254740994", "true"),
        (r#"{"a":(1+1)} == {"a":2.00000000000000000001}"#, "true"),
        (r"[(1+1)] == [2.00000000000000000001]", "true"),
        (r"(1+1) == 2", "true"),
        (r"(1+1) == 2.0", "true"),
        (r"(1+1) < 2.5", "true"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: {stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}
