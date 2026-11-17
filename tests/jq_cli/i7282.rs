//! #7282: `.[]` over `group_by(f)`'s groups with more of the pipe after it (`[group_by(.a)[] |
//! length]`) runs that rest on each group's cursors instead of decoding every group first. The
//! answer is unchanged; every expected row below is what jq 1.7.1 prints, errors included.
//! [`INPUT`] has no final newline, so jq names line 0 in an error.

use anyhow::Result;

use crate::run_jq_full;

const INPUT: &str = r#"[{"a":2,"b":"x"},{"a":1,"b":"y"},{"a":2,"b":"z"},{"a":1.0,"b":100000000000000000001},{"a":"k","b":-0},{"a":"k","b":[1,{"c":2}]}]"#;

/// (filter, expected `-c` stdout, expected stderr, exit code), run on [`INPUT`].
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("[group_by(.a)[] | length]", "[2,2,2]\n", "", 0),
    ("[group_by(.a)[] | length] | max", "2\n", "", 0),
    ("[group_by(.a) | .[] | .[0].b]", "[\"y\",\"x\",-0]\n", "", 0),
    ("[group_by(.a)[] | map(.b) | length]", "[2,2,2]\n", "", 0),
    (
        "[group_by(.a)[][] | .b]",
        "[\"y\",100000000000000000001,\"x\",\"z\",-0,[1,{\"c\":2}]]\n",
        "",
        0,
    ),
    ("[group_by(.a)[] | .[1:] | length]", "[1,1,1]\n", "", 0),
    (
        "[group_by(.a)[] | .[0].b | tostring]",
        "[\"y\",\"x\",\"-0\"]\n",
        "",
        0,
    ),
    ("[group_by(.a)[] | path(.[0])]", "[[0],[0],[0]]\n", "", 0),
    ("[.[] | [.] | group_by(.a)[] | length] | add", "6\n", "", 0),
    ("[group_by(.a)[]] | length", "3\n", "", 0),
    // A key with an effect keeps the chain whole (#4293's routing), and one in the rest runs
    // once per group, in group order.
    (
        "[group_by(.a | debug)[] | length]",
        "[2,2,2]\n",
        "[\"DEBUG:\",2]\n[\"DEBUG:\",1]\n[\"DEBUG:\",2]\n[\"DEBUG:\",1.0]\n[\"DEBUG:\",\"k\"]\n[\"DEBUG:\",\"k\"]\n",
        0,
    ),
    (
        "[group_by(.a)[] | length | debug]",
        "[2,2,2]\n",
        "[\"DEBUG:\",2]\n[\"DEBUG:\",2]\n[\"DEBUG:\",2]\n",
        0,
    ),
    (
        "[group_by(.a)[] | .[] | .b + 1]",
        "",
        "jq: error (at <stdin>:0): string (\"y\") and number (1) cannot be added\n",
        5,
    ),
    (
        "try ([group_by(.a)[] | .[] | .b + 1]) catch .",
        "\"string (\\\"y\\\") and number (1) cannot be added\"\n",
        "",
        0,
    ),
];

/// A group whose second element fails `.b + 1`: a `map` read by `first` or `.[0]` still runs
/// every element and raises, as jq's eager `map` does (#7282 review).
const MAP_INPUT: &str = r#"[{"a":1,"b":1},{"a":1,"b":"x"},{"a":2,"b":3}]"#;

/// (filter, expected `-c` stdout, expected stderr, exit code), run on [`MAP_INPUT`].
const MAP_ROWS: &[(&str, &str, &str, i32)] = &[
    (
        "[group_by(.a)[] | map(.b + 1) | first]",
        "",
        "jq: error (at <stdin>:0): string (\"x\") and number (1) cannot be added\n",
        5,
    ),
    (
        "[group_by(.a) | .[] | map(.b) | map(. + 1) | .[0]]",
        "",
        "jq: error (at <stdin>:0): string (\"x\") and number (1) cannot be added\n",
        5,
    ),
    ("[group_by(.a)[] | map(.b) | first]", "[1,3]\n", "", 0),
    (
        "[group_by(.a)[] | map(.b) | .[1:] | first]",
        "[\"x\",null]\n",
        "",
        0,
    ),
    ("[group_by(.a) | (.[]) | length]", "[2,1]\n", "", 0),
    ("[(group_by(.a)[]) | length]", "[2,1]\n", "", 0),
];

/// A `group_by(f)[]` chain in a later stage over values `group_by` cuts no groups from: the
/// chain's own answer, folded through the rest as before.
const NO_GROUPS_INPUT: &str = r#"{"x":{"p":{"a":1}},"y":"s"}"#;

/// (filter, expected `-c` stdout, expected stderr, exit code), run on [`NO_GROUPS_INPUT`].
const NO_GROUPS_ROWS: &[(&str, &str, &str, i32)] = &[
    (
        "try ([.x | group_by(.a)[] | length]) catch .",
        "\"object ({\\\"p\\\":{\\\"a\\\":1}}) and array ([[1]]) cannot be sorted, as they are not both arrays\"\n",
        "",
        0,
    ),
    (
        "try ([.y | group_by(.a)[] | length]) catch .",
        "\"Cannot iterate over string (\\\"s\\\")\"\n",
        "",
        0,
    ),
];

#[test]
fn test_iterate_over_groups_answers_as_jq_7282() -> Result<()> {
    for (input, rows) in [
        (INPUT, ROWS),
        (MAP_INPUT, MAP_ROWS),
        (NO_GROUPS_INPUT, NO_GROUPS_ROWS),
    ] {
        for (filter, want_out, want_err, want_code) in rows {
            let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), code),
                (*want_out, *want_err, *want_code),
                "`{filter}`"
            );
        }
    }
    Ok(())
}
