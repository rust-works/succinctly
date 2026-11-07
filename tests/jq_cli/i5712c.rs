//! #5712: an `if`, an array, an object, an arithmetic or a `//` built from a string's `length` is
//! answered from the string (the string door), not through the reindex bridge. The door answers
//! only in a value position; these rows put the same shapes where they must not change an answer
//! (under `path()`, `try`, `?`, `limit`, `first`, `reduce`, a bound variable, a computed key, a
//! raising division) against what jq 1.7.1 (`/usr/bin/jq`) says: `(program, stdout, stderr,
//! exit status)`.

use anyhow::Result;

use crate::run_jq_full;

const ROWS: &[(&str, &str, &str, i32)] = &[
    (
        "\"héllo\" | ascii_downcase | path([length])",
        "",
        "jq: error (at <unknown>): Invalid path expression with result [5]\n",
        5,
    ),
    (
        "\"héllo\" | ascii_downcase | path(if length > 1 then 1 else 2 end)",
        "",
        "jq: error (at <unknown>): Invalid path expression with result 1\n",
        5,
    ),
    (
        "\"héllo\" | ascii_downcase | path(length + 1)",
        "",
        "jq: error (at <unknown>): Invalid path expression with result 6\n",
        5,
    ),
    (
        "\"héllo\" | ascii_downcase | [paths(. == 1)]",
        "[]\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | try path([length]) catch \"e\"",
        "\"e\"\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | [limit(1; [length])]",
        "[[5]]\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | first([length])",
        "[5]\n",
        "",
        0,
    ),
    ("\"héllo\" | ascii_downcase | [length]?", "[5]\n", "", 0),
    (
        "\"héllo\" | ascii_downcase | try (length / 0) catch \"e\"",
        "\"e\"\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | (length / 0)? // \"z\"",
        "\"z\"\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | reduce range(3) as $i (0; . + ((\"x\"|length) + 1))",
        "6\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | . as $s | [range(3) | ($s | [length])]",
        "[[5],[5],[5]]\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | {a: [length]} | .a[0]",
        "5\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | if length > 3 then [length] else {a: length} end | tojson",
        "\"[5]\"\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | [length + 1, length * 2]",
        "[6,10]\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | (length > 99) // [length]",
        "[5]\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | {a: length, b: ((length > 3) // 7)} | tojson",
        "\"{\\\"a\\\":5,\\\"b\\\":true}\"\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | {(length|tostring): 1} | keys",
        "[\"5\"]\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | if length > 99 then 1 end",
        "\"héllo\"\n",
        "",
        0,
    ),
    (
        "\"héllo\" | ascii_downcase | length | [., . + 1]",
        "[5,6]\n",
        "",
        0,
    ),
];

#[test]
fn composed_length_shapes_in_other_positions_answer_as_jq_does_5712() -> Result<()> {
    for (program, stdout, stderr, status) in ROWS {
        let (got_out, got_err, got_status) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(got_out, *stdout, "stdout of {program}");
        assert_eq!(got_err, *stderr, "stderr of {program}");
        assert_eq!(got_status, *status, "exit status of {program}");
    }
    Ok(())
}
