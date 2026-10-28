//! #2512: a `del` at an array index renumbers every later slot, so a static index written in
//! another stage of the same pipe names one node before the `del` and another after it. The
//! reconcile used to decline the pipe (and the positional walk then gave each element the comment
//! of the slot it now stands in); it now reads each stage's indices in the coordinates they were
//! written in. Each row is a live yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

const FLAT: &str = "arr:\n  - a # ca\n  - \"b\" # cb\n  - c # cc\n  - d # cd\n  - e # ce\n";
const OBJS: &str =
    "arr:\n  - x: 1 # c0\n    y: 2 # y0\n  - x: 3 # c1\n    y: 4\n  - x: 5 # c2\n    y: 6 # y2\n";
const NEST2: &str = "arr:\n  - b:\n      - p # p0\n      - q # q0\n  - b:\n      - r # r0\n      - s # s0\n  - b:\n      - t # t0\n";
const NEST: &str = "arr:\n  - - a0 # a0\n  - - b0 # b0\n";

/// (document, filter, expected output).
const ROWS: &[(&str, &str, &str)] = &[
    (
        FLAT,
        r#".arr[3] = "Z" | del(.arr[0])"#,
        "arr:\n  - \"b\" # cb\n  - c # cc\n  - Z # cd\n  - e # ce\n",
    ),
    (
        FLAT,
        r#"del(.arr[0]) | .arr[2] = "Z""#,
        "arr:\n  - \"b\" # cb\n  - c # cc\n  - Z # cd\n  - e # ce\n",
    ),
    (
        FLAT,
        r#".arr[0] = "Z" | del(.arr[1])"#,
        "arr:\n  - Z # ca\n  - c # cc\n  - d # cd\n  - e # ce\n",
    ),
    (
        FLAT,
        r#".arr[1] |= "Z" | del(.arr[1])"#,
        "arr:\n  - a # ca\n  - c # cc\n  - d # cd\n  - e # ce\n",
    ),
    (
        FLAT,
        r#".arr[2] += "q" | del(.arr[0])"#,
        "arr:\n  - \"b\" # cb\n  - cq # cc\n  - d # cd\n  - e # ce\n",
    ),
    (
        FLAT,
        "del(.arr[0]) | del(.arr[0])",
        "arr:\n  - c # cc\n  - d # cd\n  - e # ce\n",
    ),
    (
        FLAT,
        "del(.arr[1]) | del(.arr[1])",
        "arr:\n  - a # ca\n  - d # cd\n  - e # ce\n",
    ),
    (
        FLAT,
        "del(.arr[3]) | del(.arr[0])",
        "arr:\n  - \"b\" # cb\n  - c # cc\n  - e # ce\n",
    ),
    (
        FLAT,
        "del(.arr[0]) | del(.arr[3])",
        "arr:\n  - \"b\" # cb\n  - c # cc\n  - d # cd\n",
    ),
    (
        FLAT,
        "del(.arr[0], .arr[2]) | del(.arr[0])",
        "arr:\n  - d # cd\n  - e # ce\n",
    ),
    (
        FLAT,
        r#".arr[4] = "Z" | del(.arr[0]) | .arr[2] = "Y""#,
        "arr:\n  - \"b\" # cb\n  - c # cc\n  - Y # cd\n  - Z # ce\n",
    ),
    (
        OBJS,
        ".arr[1].x = 9 | del(.arr[0])",
        "arr:\n  - x: 9 # c1\n    y: 4\n  - x: 5 # c2\n    y: 6 # y2\n",
    ),
    (
        OBJS,
        "del(.arr[0]) | .arr[1].x = 9",
        "arr:\n  - x: 3 # c1\n    y: 4\n  - x: 9 # c2\n    y: 6 # y2\n",
    ),
    (
        OBJS,
        "del(.arr[0]) | del(.arr[0].y)",
        "arr:\n  - x: 3 # c1\n  - x: 5 # c2\n    y: 6 # y2\n",
    ),
    (
        OBJS,
        "del(.arr[1].y) | del(.arr[0])",
        "arr:\n  - x: 3 # c1\n  - x: 5 # c2\n    y: 6 # y2\n",
    ),
    (
        NEST,
        r#".arr[1] = (["new"] + .arr[1]) | del(.arr[0])"#,
        "arr:\n  - - new\n    - b0 # b0\n",
    ),
    (
        NEST,
        r#"del(.arr[0]) | .arr[0] = (["new"] + .arr[0])"#,
        "arr:\n  - - new\n    - b0 # b0\n",
    ),
    (
        NEST2,
        r#".arr[1].b[1] = "Z" | del(.arr[0]) | del(.arr[0].b[0])"#,
        "arr:\n  - b:\n      - Z # s0\n  - b:\n      - t # t0\n",
    ),
    (
        NEST2,
        "del(.arr[0]) | del(.arr[0].b[0])",
        "arr:\n  - b:\n      - s # s0\n  - b:\n      - t # t0\n",
    ),
    (
        NEST2,
        r#"del(.arr[1].b[0]) | del(.arr[0]) | .arr[0].b[0] = "Y""#,
        "arr:\n  - b:\n      - Y # s0\n  - b:\n      - t # t0\n",
    ),
    (
        FLAT,
        r#".arr[1] = "A" | del(.arr[0]) | .arr[2] = "B" | del(.arr[0])"#,
        "arr:\n  - c # cc\n  - B # cd\n  - e # ce\n",
    ),
    (
        FLAT,
        r#"setpath(["arr", 3]; "Z") | delpaths([["arr", 0]])"#,
        "arr:\n  - \"b\" # cb\n  - c # cc\n  - Z # cd\n  - e # ce\n",
    ),
    (
        FLAT,
        r#"delpaths([["arr", 0]]) | setpath(["arr", 1]; "Z")"#,
        "arr:\n  - \"b\" # cb\n  - Z # cc\n  - d # cd\n  - e # ce\n",
    ),
];

#[test]
fn a_del_renumbering_another_stages_index_keeps_each_nodes_presentation_2512() -> Result<()> {
    for (doc, filter, expected) in ROWS {
        let (output, exit_code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(exit_code, 0, "{filter}");
        assert_eq!(&output, expected, "{filter}");
    }
    Ok(())
}

/// **Known divergences** (`docs/compliance/yq/limitations.md`): shapes the stage order does not
/// resolve. Pinned with yq's own answer in a comment so a fix has to update them deliberately.
#[test]
fn what_the_stage_order_does_not_resolve_is_pinned_2512() -> Result<()> {
    // (document, filter, succinctly's output, yq's output)
    let rows: &[(&str, &str, &str, &str)] = &[
        // A negative index into an array a `del` renumbers.
        (
            FLAT,
            "del(.arr[0]) | del(.arr[-1])",
            "arr:\n  - b # ca\n  - \"c\" # cb\n  - d # cc\n",
            "arr:\n  - \"b\" # cb\n  - c # cc\n  - d # cd\n",
        ),
        // An append next to a `del`: the array is longer than its survivors.
        (
            FLAT,
            r#".arr[5] = "Z" | del(.arr[0])"#,
            "arr:\n  - b # ca\n  - \"c\" # cb\n  - d # cc\n  - e # cd\n  - Z # ce\n",
            "arr:\n  - \"b\" # cb\n  - c # cc\n  - d # cd\n  - e # ce\n  - Z\n",
        ),
        // One `del` whose comma branches renumber each other's array.
        (
            OBJS,
            "del(.arr[0], .arr[1].x)",
            "arr:\n  - y: 4 # y0\n  - x: 5 # c1\n    y: 6\n",
            "arr:\n  - y: 4\n  - x: 5 # c2\n    y: 6 # y2\n",
        ),
    ];
    for (doc, filter, ours, _yq) in rows {
        let (output, exit_code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(exit_code, 0, "{filter}");
        assert_eq!(&output, ours, "{filter}");
    }
    Ok(())
}
