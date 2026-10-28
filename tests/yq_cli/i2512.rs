//! #2512: a `del` at an array index renumbers every later slot, so a static index written in
//! another stage of the same pipe names one node before the `del` and another after it. The
//! reconcile used to decline the pipe (and the positional walk then gave each element the comment
//! of the slot it now stands in); it now reads each stage's indices in the coordinates they were
//! written in. Each row is a live yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

const FLAT: &str = "arr:\n  - a # ca\n  - \"b\" # cb\n  - c # cc\n  - d # cd\n  - e # ce\n";
const OBJS: &str = "arr:\n  - x: 1 # c0\n    y: 2 # y0\n  - x: 3 # c1\n    y: 4\n  - x: 5 # c2\n    y: 6 # y2\n";
const NEST: &str = "arr:\n  - - a0 # a0\n  - - b0 # b0\n";

/// (document, filter, expected output).
const ROWS: &[(&str, &str, &str)] = &[
    (FLAT, r#".arr[3] = "Z" | del(.arr[0])"#,
        "arr:\n  - \"b\" # cb\n  - c # cc\n  - Z # cd\n  - e # ce\n"),
    (FLAT, r#"del(.arr[0]) | .arr[2] = "Z""#,
        "arr:\n  - \"b\" # cb\n  - c # cc\n  - Z # cd\n  - e # ce\n"),
    (FLAT, r#".arr[0] = "Z" | del(.arr[1])"#,
        "arr:\n  - Z # ca\n  - c # cc\n  - d # cd\n  - e # ce\n"),
    (FLAT, r#".arr[1] |= "Z" | del(.arr[1])"#,
        "arr:\n  - a # ca\n  - c # cc\n  - d # cd\n  - e # ce\n"),
    (FLAT, r#".arr[2] += "q" | del(.arr[0])"#,
        "arr:\n  - \"b\" # cb\n  - cq # cc\n  - d # cd\n  - e # ce\n"),
    (FLAT, "del(.arr[0]) | del(.arr[0])",
        "arr:\n  - c # cc\n  - d # cd\n  - e # ce\n"),
    (FLAT, "del(.arr[1]) | del(.arr[1])",
        "arr:\n  - a # ca\n  - d # cd\n  - e # ce\n"),
    (FLAT, "del(.arr[3]) | del(.arr[0])",
        "arr:\n  - \"b\" # cb\n  - c # cc\n  - e # ce\n"),
    (FLAT, "del(.arr[0]) | del(.arr[3])",
        "arr:\n  - \"b\" # cb\n  - c # cc\n  - d # cd\n"),
    (FLAT, "del(.arr[0], .arr[2]) | del(.arr[0])",
        "arr:\n  - d # cd\n  - e # ce\n"),
    (FLAT, r#".arr[4] = "Z" | del(.arr[0]) | .arr[2] = "Y""#,
        "arr:\n  - \"b\" # cb\n  - c # cc\n  - Y # cd\n  - Z # ce\n"),
    (OBJS, ".arr[1].x = 9 | del(.arr[0])",
        "arr:\n  - x: 9 # c1\n    y: 4\n  - x: 5 # c2\n    y: 6 # y2\n"),
    (OBJS, "del(.arr[0]) | .arr[1].x = 9",
        "arr:\n  - x: 3 # c1\n    y: 4\n  - x: 9 # c2\n    y: 6 # y2\n"),
    (OBJS, "del(.arr[0]) | del(.arr[0].y)",
        "arr:\n  - x: 3 # c1\n  - x: 5 # c2\n    y: 6 # y2\n"),
    (OBJS, "del(.arr[1].y) | del(.arr[0])",
        "arr:\n  - x: 3 # c1\n  - x: 5 # c2\n    y: 6 # y2\n"),
    (NEST, r#".arr[1] = (["new"] + .arr[1]) | del(.arr[0])"#,
        "arr:\n  - - new\n    - b0 # b0\n"),
    (NEST, r#"del(.arr[0]) | .arr[0] = (["new"] + .arr[0])"#,
        "arr:\n  - - new\n    - b0 # b0\n"),
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
