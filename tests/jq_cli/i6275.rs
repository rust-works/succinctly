//! #6275: a computed integer (`1+1`, `length`, `floor`, a negation) is a double in jq, so it compares as
//! one against a decimal literal whose exact value is not its double (`2.00000000000000000001`, `1e-400`);
//! two literals still compare exactly. `Int` was read as a literal, so a computed integer was compared
//! by its exact decimal digits. Each row was run through jq 1.7.1 (`/usr/bin/jq -nc`). The second test is
//! the same rule where the value crosses the reindex bridge (a multi-output stage, `map`, `unique`, `sort`,
//! `group_by`, `min`/`max`, `index`, `IN`), which re-baked a bare `Int` as a literal.

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

#[test]
fn a_computed_integer_stays_computed_across_the_reindex_bridge_6275() -> Result<()> {
    for (filter, expected) in [
        (
            r"[2.00000000000000000001,(1+1)] | unique",
            r"[2.00000000000000000001]",
        ),
        (r"[1e-400,(0+0)] | sort", r"[1E-400,0]"),
        (r"[1e-400,(0+0)] | min", r"1E-400"),
        (r"[1e-400,(0+0)] | max", r"0"),
        (r"[(0,0)+0,1e-400] | unique", r"[0]"),
        (r"[(1+1),2.00000000000000000001] | group_by(.)|length", r"1"),
        (
            r"[(1+1),2.00000000000000000001] | unique_by(.)|length",
            r"1",
        ),
        (r"[(0+0),1e-400]|map(.==1e-400)", r"[true,true]"),
        (
            r"[[1,2],[3]]|map(length)|map(.==2.00000000000000000001)",
            r"[true,false]",
        ),
        (
            r"[range(0;3)]|map(.==1.00000000000000000001)",
            r"[false,true,false]",
        ),
        (
            r"[1e-400,(0+0)]|(.[0] < .[1]), (.[0] > .[1])",
            r"false
false",
        ),
        (
            r"[2,2.00000000000000000001,(1+1)]|sort",
            r"[2,2.00000000000000000001,2]",
        ),
        (
            r"[9007199254740993, (9007199254740992+0)]|unique",
            r"[9007199254740993]",
        ),
        (
            r"[9007199254740993, (9007199254740992+0)]|sort",
            r"[9007199254740993,9007199254740992]",
        ),
        (r"[(1+1)]|first == 2.00000000000000000001", r"true"),
        (r"[[1,2]|length]|index(2.00000000000000000001)", r"0"),
        (r"[[1,2]|length]|any(.==2.00000000000000000001)", r"true"),
        (
            r"[[1,2]|length]|.[]|select(.==2.00000000000000000001)",
            r"2",
        ),
        (
            r"[[1,2]|length]|to_entries[0].value==2.00000000000000000001",
            r"true",
        ),
        (r"(1+1)|IN(2.00000000000000000001)", r"true"),
        (r"[(1+1)]|IN([2.00000000000000000001])", r"true"),
        (
            r"[[1,2][]|.+0]|map(.==2.00000000000000000001)",
            r"[false,true]",
        ),
        (
            r#"{"a":(1+1)}|to_entries|map(.value==2.00000000000000000001)"#,
            r"[true]",
        ),
        (
            r"[1,2]|map(.+0)|.[1]|[.]|inside([2.00000000000000000001])",
            r"true",
        ),
        (r"[(1+1)]|map(.)|.[0]|tojson", r#""2""#),
        (r"[range(3)]|map(.)", r"[0,1,2]"),
        (r"[-5+0]|map(.)|.[0]|-.", r"5"),
        (r"[(2-7)]|map(.)|.[0]|tojson", r#""-5""#),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: {stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}
