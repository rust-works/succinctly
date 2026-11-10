//! #6840: `with(PATH; update)` applies a `del(...)` or `delpaths(...)` in the update as it does an
//! assignment: the delete changes the node `PATH` names, in place. Every row is captured from yq
//! v4.53.3 (`-o=json -I=0`) over `a: {x: 1, y: 2}` / `b: [1, 2, 3]`; before the change a delete was
//! taken for a value-producing stage, evaluated and discarded, and the assignments piped after it
//! were dropped with it.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "a: {x: 1, y: 2}\nb: [1, 2, 3]\n";
const ARGS: &[&str] = &["-o=json", "-I=0"];

/// `(filter, output)`.
const ROWS: &[(&str, &str)] = &[
    // The delete alone, on a map, an array, by `delpaths`, and of several keys.
    ("with(.a; del(.x))", "{\"a\":{\"y\":2},\"b\":[1,2,3]}\n"),
    (
        "with(.b; del(.[0]))",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":[2,3]}\n",
    ),
    (
        "with(.a; delpaths([[\"x\"]]))",
        "{\"a\":{\"y\":2},\"b\":[1,2,3]}\n",
    ),
    ("with(.a; del(.x, .y))", "{\"a\":{},\"b\":[1,2,3]}\n"),
    (
        "with(.b; del(.[] | select(. > 1)))",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":[1]}\n",
    ),
    // A delete in a comma, a pipe of deletes, and a delete with an assignment on either side.
    ("with(.a; del(.x), del(.y))", "{\"a\":{},\"b\":[1,2,3]}\n"),
    ("with(.a; del(.x) | del(.y))", "{\"a\":{},\"b\":[1,2,3]}\n"),
    (
        "with(.a; (del(.x), .q = 1))",
        "{\"a\":{\"y\":2,\"q\":1},\"b\":[1,2,3]}\n",
    ),
    (
        "with(.a; del(.x) | .z = 1)",
        "{\"a\":{\"y\":2,\"z\":1},\"b\":[1,2,3]}\n",
    ),
    (
        "with(.a; .z = 1 | del(.x))",
        "{\"a\":{\"y\":2,\"z\":1},\"b\":[1,2,3]}\n",
    ),
    (
        "with(.a; .x |= . + 1 | del(.y))",
        "{\"a\":{\"x\":2},\"b\":[1,2,3]}\n",
    ),
    (
        "with(.b; del(.[0]) | .[0] = 9)",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":[9,3]}\n",
    ),
    // A stage after the delete that yields a value is discarded; a `select` guards the delete.
    (
        "with(.a; del(.x) | length)",
        "{\"a\":{\"y\":2},\"b\":[1,2,3]}\n",
    ),
    (
        "with(.a; select(.x == 1) | del(.x))",
        "{\"a\":{\"y\":2},\"b\":[1,2,3]}\n",
    ),
    (
        "with(.a; select(.x == 2) | del(.x))",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":[1,2,3]}\n",
    ),
    // A binding in front of the delete, and an assignment that rebuilds the node first.
    (
        "with(.a; .x as $v | del(.y))",
        "{\"a\":{\"x\":1},\"b\":[1,2,3]}\n",
    ),
    (
        "with(.a; . = {\"k\": 1} | del(.k))",
        "{\"a\":{},\"b\":[1,2,3]}\n",
    ),
    // A path that is not there is created, and a delete that finds nothing changes nothing.
    (
        "with(.a.nope; del(.x))",
        "{\"a\":{\"x\":1,\"y\":2,\"nope\":null},\"b\":[1,2,3]}\n",
    ),
    (
        "with(.a; del(.zz))",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":[1,2,3]}\n",
    ),
    (
        "with(.a; del(.x.y))",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":[1,2,3]}\n",
    ),
    // Controls that were right before: a value-producing stage, or a delete of a fresh value, is
    // not a change to the document.
    (
        "with(.a; to_entries)",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":[1,2,3]}\n",
    ),
    (
        "with(.a; ({}) | del(.x))",
        "{\"a\":{\"x\":1,\"y\":2},\"b\":[1,2,3]}\n",
    ),
];

#[test]
fn test_with_applies_a_delete_in_its_update_6840() -> Result<()> {
    for (filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, DOC, ARGS)?;
        assert_eq!(code, 0, "exit code of {filter:?}");
        assert_eq!(&stdout, want, "{filter:?}");
    }
    Ok(())
}

/// What still differs, pinned so a change to it is a decision (the yq answers are in
/// `docs/compliance/yq/limitations.md`, captured from v4.53.3): a delete of every element of a
/// fan-out path, a value-producing head that yq changes in place and hands on, and a write
/// wrapped in `//`. Each row is `(filter, what succinctly prints, what yq prints)`.
#[test]
fn test_with_delete_residuals_stay_recorded_6840() -> Result<()> {
    let rows: &[(&str, &str, &str)] = &[
        (
            "with(.b[]; del(.))",
            "{\"a\":{\"x\":1,\"y\":2},\"b\":[1,2,3]}\n",
            "{\"a\":{\"x\":1,\"y\":2},\"b\":[]}\n",
        ),
        (
            "with(.a; sort_keys(.) | .z = 1)",
            "{\"a\":{\"x\":1,\"y\":2},\"b\":[1,2,3]}\n",
            "{\"a\":{\"x\":1,\"y\":2,\"z\":1},\"b\":[1,2,3]}\n",
        ),
        (
            "with(.a; del(.x) // 1)",
            "{\"a\":{\"x\":1,\"y\":2},\"b\":[1,2,3]}\n",
            "{\"a\":{\"y\":2},\"b\":[1,2,3]}\n",
        ),
        (
            "with(.a; (.z = 1) // 2)",
            "{\"a\":{\"x\":1,\"y\":2},\"b\":[1,2,3]}\n",
            "{\"a\":{\"x\":1,\"y\":2,\"z\":1},\"b\":[1,2,3]}\n",
        ),
    ];
    for (filter, ours, yq) in rows {
        let (stdout, code) = run_yq_stdin(filter, DOC, ARGS)?;
        assert_eq!(code, 0, "exit code of {filter:?}");
        assert_eq!(&stdout, ours, "{filter:?}");
        assert_ne!(
            ours, yq,
            "{filter:?} now matches yq: drop it from the limitations"
        );
    }
    Ok(())
}
