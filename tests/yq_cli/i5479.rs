//! #5479: a write whose path selects nodes by what only the document knows about them (style,
//! anchor, comment, line, whether a node is an alias) matched nothing, because a write resolved
//! its path against the owned document, which carries values alone. The path is now resolved on
//! the document's cursor first. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_write_path_selecting_by_node_metadata_matches_yq_5479() -> Result<()> {
    for (input, filter, want) in [
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(style == \"double\")) = 5",
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: 5\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(style == \"single\")) = 5",
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 5\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(style == \"flow\")) = 5",
            "a: &x 5\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(anchor == \"y\")) = 5",
            "a: &x {p: 1}\nb: *x\nc: &y 5 # cm\nd: \"s\"\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.. | select(anchor == \"x\")) = 5",
            "a: &x 5\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(line_comment == \"cm\")) = 5",
            "a: &x {p: 1}\nb: *x\nc: &y 5 # cm\nd: \"s\"\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(kind == \"alias\")) = 5",
            "a: &x {p: 1}\nb: 5\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(line == 3)) = 5",
            "a: &x {p: 1}\nb: *x\nc: &y 5 # cm\nd: \"s\"\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.. | select(anchor == \"y\")) anchor = \"z\"",
            "a: &x {p: 1}\nb: *x\nc: &z 2 # cm\nd: \"s\"\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.. | select(style == \"double\")) style = \"single\"",
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: 's'\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(style == \"double\")) |= \"new\"",
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"new\"\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(anchor != \"\")) |= 7",
            "a: &x 7\nb: *x\nc: &y 7 # cm\nd: \"s\"\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(line_comment != \"\")) += 1",
            "a: &x {p: 1}\nb: *x\nc: &y 3 # cm\nd: \"s\"\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(style == \"double\" or anchor == \"y\")) = 5",
            "a: &x {p: 1}\nb: *x\nc: &y 5 # cm\nd: 5\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "del(.[] | select(style == \"double\"))",
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "del(.. | select(anchor == \"y\"))",
            "a: &x {p: 1}\nb: *x\nd: \"s\"\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(style == \"nothing\")) = 5",
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(style == \"double\")) = 5 | .d",
            "5\n",
        ),
        (
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: \"s\"\ne: 'q'\n",
            "(.[] | select(style == \"double\")) = 5 | .e = 1",
            "a: &x {p: 1}\nb: *x\nc: &y 2 # cm\nd: 5\ne: 1\n",
        ),
        (
            "a: \"x\"\n---\nb: \"y\"\nc: z\n",
            "(.. | select(style == \"double\")) = 1",
            "a: 1\n---\nb: 1\nc: z\n",
        ),
        (
            "a:\n  - {k: 1}\n  - \"z\"\n  - 'w'\n",
            "del(.a[] | select(style == \"double\" or style == \"single\"))",
            "a:\n  - {k: 1}\n",
        ),
    ] {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter} over {input:?}");
    }
    Ok(())
}

#[test]
fn test_metadata_write_with_no_match_leaves_the_document_alone_5479() -> Result<()> {
    let input = "a: 1\nb: [2, 3]\n";
    for filter in [
        "(.. | select(style == \"double\")) = 9",
        "(.. | select(anchor != \"\")) |= 9",
        "del(.. | select(line_comment != \"\"))",
    ] {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((out.as_str(), code), (input, 0), "{filter}");
    }
    Ok(())
}
