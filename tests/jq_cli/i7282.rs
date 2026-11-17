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

#[test]
fn test_iterate_over_groups_answers_as_jq_7282() -> Result<()> {
    for (filter, want_out, want_err, want_code) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(INPUT))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (*want_out, *want_err, *want_code),
            "`{filter}`"
        );
    }
    Ok(())
}
