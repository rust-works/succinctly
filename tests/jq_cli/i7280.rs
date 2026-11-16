//! #7280: `group_by(f) | map({k: .[0].k, n: length})` builds each group's object from the group's
//! cursors instead of decoding the group and writing it out and indexing it again. The answer
//! is unchanged; every expected row below is what jq 1.7.1 prints, errors included (a read that
//! raises leaves the group to the decoded route, which keeps its diagnostic). [`INPUT`] has no
//! final newline, so jq names line 0 in an error.

use anyhow::Result;

use crate::run_jq_full;

const INPUT: &str = r#"[{"a":2,"b":"x"},{"a":1,"b":"y"},{"a":2,"b":"z"},{"a":1.0,"b":100000000000000000001},{"a":"k","b":-0},{"a":"k","b":[1,{"c":2}]}]"#;

/// (filter, expected `-c` stdout, expected stderr, exit code), run on [`INPUT`].
const ROWS: &[(&str, &str, &str, i32)] = &[
    (
        "group_by(.a) | map({h: .[0].a, n: length})",
        "[{\"h\":1,\"n\":2},{\"h\":2,\"n\":2},{\"h\":\"k\",\"n\":2}]\n",
        "",
        0,
    ),
    (
        "group_by(.a) | map({f: first.b, l: last.b, x: .[9], lit: \"k\", in: {n: length}})",
        "[{\"f\":\"y\",\"l\":100000000000000000001,\"x\":null,\"lit\":\"k\",\"in\":{\"n\":2}},{\"f\":\"x\",\"l\":\"z\",\"x\":null,\"lit\":\"k\",\"in\":{\"n\":2}},{\"f\":-0,\"l\":[1,{\"c\":2}],\"x\":null,\"lit\":\"k\",\"in\":{\"n\":2}}]\n",
        "",
        0,
    ),
    (
        "group_by(.a) | map([.[0].b, length, [last.b]])",
        "[[\"y\",2,[100000000000000000001]],[\"x\",2,[\"z\"]],[-0,2,[[1,{\"c\":2}]]]]\n",
        "",
        0,
    ),
    (
        "group_by(.a) | map({k: .[0].a, k: length})",
        "[{\"k\":2},{\"k\":2},{\"k\":2}]\n",
        "",
        0,
    ),
    (
        "[group_by(.a) | map({n: length})[] | .n] | add",
        "6\n",
        "",
        0,
    ),
    (
        "group_by(.a) | map({c: .[0].b.c})",
        "",
        "jq: error (at <stdin>:0): Cannot index string with string \"c\"\n",
        5,
    ),
    (
        "group_by(.a) | map({c: .[-1].b[1].c})",
        "",
        "jq: error (at <stdin>:0): Cannot index number with number\n",
        5,
    ),
    (
        "try (group_by(.a) | map({c: .[0].b.c})) catch .",
        "\"Cannot index string with string \\\"c\\\"\"\n",
        "",
        0,
    ),
    (
        "group_by(.a) | map({b})",
        "",
        "jq: error (at <stdin>:0): Cannot index array with string \"b\"\n",
        5,
    ),
];

#[test]
fn test_group_by_map_construction_answers_as_jq_7280() -> Result<()> {
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
