//! #5595: `add` over a document array is read element by element, where it used to convert the
//! array, write it out as JSON, index that and convert every element again. The answer must not
//! move: sums keep a lone element's spelling, `null` and `[]`/`""`/`{}` fold as jq's `add`
//! does, and a mix that cannot be added raises jq's message. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -c`); an `add` that raises is caught, so the message is compared too.

use anyhow::Result;

use crate::run_jq_full;

/// `(stdin, filter, expected stdout)`.
type Row = (&'static str, &'static str, &'static str);

const ROWS: &[Row] = &[
    (r"[1,2,3]", r"add", r"6"),
    (r"[]", r"add", r"null"),
    (r"[1]", r"add", r"1"),
    (r"[1.000]", r"add", r"1.000"),
    (r"[1.50,2.25]", r"add", r"3.75"),
    (r"[100000000000000000000,1]", r"add", r"1e+20"),
    (r"[1e1000,1]", r"add", r"1.7976931348623157e+308"),
    (r"[-0,0]", r"add", r"0"),
    (r"[0.1,0.2,0.3]", r"add", r"0.6000000000000001"),
    (r"[1,null,2]", r"add", r"3"),
    (r"[null,null]", r"add", r"null"),
    (r"[null]", r"add", r"null"),
    (r#"["a","b","c"]"#, r"add", r#""abc""#),
    (r#"["a"]"#, r"add", r#""a""#),
    (r"[[1],[2,3],[]]", r"add", r"[1,2,3]"),
    (r"[[1]]", r"add", r"[1]"),
    (r#"[{"a":1},{"b":2},{"a":3}]"#, r"add", r#"{"a":3,"b":2}"#),
    (r#"[{"a":1}]"#, r"add", r#"{"a":1}"#),
    (r#"{"a":1,"b":2}"#, r"add", r"3"),
    (r#"{"a":"x","b":"y"}"#, r"add", r#""xy""#),
    (r"{}", r"add", r"null"),
    (
        r#"[1,"a"]"#,
        r"try add catch .",
        r#""number (1) and string (\"a\") cannot be added""#,
    ),
    (
        r#"["a",1]"#,
        r"try add catch .",
        r#""string (\"a\") and number (1) cannot be added""#,
    ),
    (
        r#"[1,2,"x",{}]"#,
        r"try add catch .",
        r#""number (3) and string (\"x\") cannot be added""#,
    ),
    (
        r#"[[1],{"a":1}]"#,
        r"try add catch .",
        r#""array ([1]) and object ({\"a\":1}) cannot be added""#,
    ),
    (
        r#"[{"a":1},[1]]"#,
        r"try add catch .",
        r#""object ({\"a\":1}) and array ([1]) cannot be added""#,
    ),
    (
        r"[1,true]",
        r"try add catch .",
        r#""number (1) and boolean (true) cannot be added""#,
    ),
    (r"[null,1,null]", r"add", r"1"),
    (r#"[null,"a"]"#, r"add", r#""a""#),
    (r"[null,[1]]", r"add", r"[1]"),
    (r#"[null,{"a":1}]"#, r"add", r#"{"a":1}"#),
    (
        r"5",
        r"try add catch .",
        r#""Cannot iterate over number (5)""#,
    ),
    (
        r#""abc""#,
        r"try add catch .",
        r#""Cannot iterate over string (\"abc\")""#,
    ),
    (
        r"true",
        r"try add catch .",
        r#""Cannot iterate over boolean (true)""#,
    ),
    (r#"[1,"a"]"#, r"add?", r""),
    (r"[1,2,3]", r"add?", r"6"),
    (r"[[1,2],[3,4]]", r"map(add)", r"[3,7]"),
    (r"[[1,2],[3,4]]", r"map(add)|add", r"10"),
    (
        r#"{"a":[1,2],"b":[3]}"#,
        r"map_values(add)",
        r#"{"a":3,"b":3}"#,
    ),
    (r#"[{"x":[1,2]},{"x":[3]}]"#, r"map(.x)|add", r"[1,2,3]"),
    (r#"[{"x":1},{"x":2}]"#, r"map(.x)|add", r"3"),
    (r#"[{"x":1},{"x":2}]"#, r"[.[].x]|add", r"3"),
    (r#"[{"a":1},{"a":2}]"#, r"add|.a", r"2"),
    (r"[1.0,2.0]", r"add", r"3"),
    (r"[1,2]", r"add|tojson", r#""3""#),
    (r"[3,1,2]", r"add as $s|$s", r"6"),
    (r"[]", r"add//7", r"7"),
    (r"[null]", r"add//7", r"7"),
    (r"[0.1,0.2]", r"add==0.3", r"false"),
    (r"[9007199254740993,1]", r"add", r"9007199254740992"),
    (r"[9007199254740993]", r"add", r"9007199254740993"),
    (r"[1e-5,1e-5]", r"add", r"2e-05"),
    (r"[0.00000001]", r"add", r"1E-8"),
    (r"[3.14159265358979323846,0]", r"add", r"3.141592653589793"),
    (r"[1,2,3,4,5,6,7,8,9,10]", r"add", r"55"),
    (r"[[[1]],[[2]]]", r"add", r"[[1],[2]]"),
    (r"[[],[]]", r"add", r"[]"),
    (r#"["",""]"#, r"add", r#""""#),
    (r"[{},{}]", r"add", r"{}"),
    (
        r"[1,[2]]",
        r"try add catch .",
        r#""number (1) and array ([2]) cannot be added""#,
    ),
    (r"[1,2,3]", r". as $x | add", r"6"),
    (
        r"[[1],[2]]",
        r". as $a | $a | add == ($a[0] + $a[1])",
        r"true",
    ),
    (r"[[1],[2]]", r".[0] as $x | add | length", r"2"),
    (r#"[{"a":1},{"b":2}]"#, r".[0] as $x | add | .a", r"1"),
    (
        r#"[1,"a"]"#,
        r". as $x | try add catch .",
        r#""number (1) and string (\"a\") cannot be added""#,
    ),
    (
        r"[[1,2],[3]]",
        r". as $a | [add, $a[0]]",
        r"[[1,2,3],[1,2]]",
    ),
];

const PATH_ROWS: &[Row] = &[(r"[[1],[2]]", r". as $x | add | path($x)?", r"")];

fn check(rows: &[Row]) -> Result<()> {
    for (input, filter, expected) in rows {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(
            (stdout.trim_end(), code),
            (*expected, 0),
            "{filter} on {input}: stderr {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_add_over_an_array_matches_jq_5595() -> Result<()> {
    check(ROWS)
}

/// `path($x)` after an `add` over a bound document is a path expression that does more than
/// navigate, which the unshared-containers holdout refuses by design.
#[test]
#[cfg_attr(feature = "unshared-containers", ignore = "holdout refusal (#5246)")]
fn test_add_keeps_a_bound_node_out_of_the_path_like_jq_5595() -> Result<()> {
    check(PATH_ROWS)
}
