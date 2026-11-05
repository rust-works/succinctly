//! #6206: a `select` whose condition reads `key`, in the target of a write (`=`, `|=`, `+=`,
//! `del()`), is judged against the key of the node the select stands at, as yq does. It used to
//! resolve to no target, so the write did nothing and exited 0. Every row was captured from yq
//! v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

/// (input, filter, yq's `-o=json -I=0` output).
const ROWS: &[(&str, &str, &str)] = &[
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key == "x")) = 5"#,
        r#"{"a":{"x":5,"y":{"x":5}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key == "x")) |= 5"#,
        r#"{"a":{"x":5,"y":{"x":5}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key == "x")) += 5"#,
        r#"{"a":{"x":6,"y":{"x":7}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key == "x")) |= key"#,
        r#"{"a":{"x":"x","y":{"x":"x"}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key == "x" and . == 2)) = 5"#,
        r#"{"a":{"x":1,"y":{"x":5}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key != "x")) |= 7"#,
        "7",
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.a | .. | select(key == "x")) = 5"#,
        r#"{"a":{"x":5,"y":{"x":5}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.[] | select(key == "a")) = 5"#,
        r#"{"a":5}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.a[] | select(key == "x")) = 5"#,
        r#"{"a":{"x":5,"y":{"x":2}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key == "x") | key) = "q""#,
        r#"{"a":{"q":1,"y":{"q":2}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"del(.. | select(key == "x"))"#,
        r#"{"a":{"y":{}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"del(.a[] | select(key == "x"))"#,
        r#"{"a":{"y":{"x":2}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key == "nope")) = 5"#,
        r#"{"a":{"x":1,"y":{"x":2}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key == "x" or key == "y")) = 5"#,
        r#"{"a":{"x":5,"y":5}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r"(.a[] | select((key | length) == 1)) = 5",
        r#"{"a":{"x":5,"y":5}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        "(.. | select(key)) = 5",
        r#"{"a":5}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(.x == 1 or key == "x")) |= 9"#,
        r#"{"a":9}"#,
    ),
    (
        "l: [a, b, c]\n",
        r#"(.l[] | select(key == 1)) = "X""#,
        r#"{"l":["a","X","c"]}"#,
    ),
    (
        "l: [a, b, c]\n",
        r#"(.. | select(key == 0)) = "X""#,
        r#"{"l":["X","b","c"]}"#,
    ),
    (
        "l: [a, b, c]\n",
        "del(.l[] | select(key > 0))",
        r#"{"l":["a"]}"#,
    ),
    (
        "l: [a, b, c]\n",
        r#"(.l[] | select(key >= 1)) |= "Z""#,
        r#"{"l":["a","Z","Z"]}"#,
    ),
    (
        "l: [{k: 1}, {k: 2}]\n",
        r#"(.. | select(key == "k")) |= . + 10"#,
        r#"{"l":[{"k":11},{"k":12}]}"#,
    ),
    (
        "l: [{k: 1}, {k: 2}]\n",
        "(.l[] | select(key == 1) | .k) = 0",
        r#"{"l":[{"k":1},{"k":0}]}"#,
    ),
    ("x: 1\n", r#"(. | select(key == "x")) = 3"#, r#"{"x":1}"#),
    (
        "a: {x: 1}\n",
        r#"(.a | select(key == "a")) = 3"#,
        r#"{"a":3}"#,
    ),
    (
        "a: {x: 1}\n",
        r#"(.a | select(key == "a") | .x) = 3"#,
        r#"{"a":{"x":3}}"#,
    ),
    (
        "a: {x: 1, y: 2}\n",
        r#"(.a.[] | select(key == "y" // false)) = 3"#,
        r#"{"a":{"x":1,"y":3}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(.x | key == "x")) = 5"#,
        r#"{"a":5}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key == "x" and (.a | key == "a"))) = 5"#,
        r#"{"a":{"x":1,"y":{"x":2}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key | test("x"))) = 5"#,
        r#"{"a":{"x":5,"y":{"x":5}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(has("x") and key == "a")) = 5"#,
        r#"{"a":5}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key == ("x"))) = 5"#,
        r#"{"a":{"x":5,"y":{"x":5}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select([key] == ["x"])) = 5"#,
        r#"{"a":{"x":1,"y":{"x":2}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key == "x" // key == "y")) = 5"#,
        r#"{"a":{"x":1,"y":{"x":2}}}"#,
    ),
    (
        "l: [a, b, c]\n",
        r"(.l[1:] | select(key == 0)) = 9",
        r#"{"l":["a","b","c"]}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(. as $k | key == "x")) = 5"#,
        r#"{"a":{"x":5,"y":{"x":5}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(. as $k | $k == 1 and key == "x")) = 5"#,
        r#"{"a":{"x":5,"y":{"x":2}}}"#,
    ),
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(key as $k | $k == "x")) = 5"#,
        r#"{"a":{"x":5,"y":{"x":5}}}"#,
    ),
    (
        "l: [a, b, c]\n",
        r#"(.l[-1] | select(key == -1)) = "X""#,
        r#"{"l":["a","b","c"]}"#,
    ),
];

#[test]
fn a_select_over_key_in_a_write_target_reaches_the_matching_nodes_6206() -> Result<()> {
    for (input, filter, expected) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, &["-o=json", "-I=0"])?;
        assert_eq!(out.trim_end(), *expected, "{filter} on {input:?}");
        assert_eq!(code, 0, "{filter} on {input:?}");
    }
    Ok(())
}

/// Where the write is still not as yq's: (input, filter, this tool's answer, yq v4.53.3's). A `key` after
/// the head of a pipe names another node's key, and a negative index in the path is not the node's key.
const GAPS: &[(&str, &str, &str, &str)] = &[
    (
        "a: {x: 1, y: {x: 2}}\n",
        r#"(.. | select(. as $k | $k | key == "x")) = 5"#,
        r#"{"a":{"x":1,"y":{"x":2}}}"#,
        r#"{"a":{"x":5,"y":{"x":5}}}"#,
    ),
    (
        "l: [a, b, c]\n",
        r#"(.l[-1] | select(key == 2)) = "X""#,
        r#"{"l":["a","b","c"]}"#,
        r#"{"l":["a","b","X"]}"#,
    ),
];

#[test]
fn a_key_the_binding_does_not_reach_still_differs_from_yq_6206() -> Result<()> {
    for &(input, filter, ours, yq) in GAPS {
        let (out, code) = run_yq_stdin(filter, input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter} on {input:?}");
        assert_eq!(out.trim_end(), ours, "{filter} on {input:?}");
        assert_ne!(
            ours, yq,
            "{filter}: no longer a gap, drop it from the table"
        );
    }
    Ok(())
}
