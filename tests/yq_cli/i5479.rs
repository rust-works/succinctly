//! #5479: a write whose path selects nodes by what only the document knows about them (style,
//! anchor, comment, line, whether a node is an alias) matched nothing, because a write resolved
//! its path against the owned document, which carries values alone. The path is now resolved on
//! the document's cursor first. Every row was captured from yq v4.53.3, except the `//=` row,
//! which pins succinctly's own operator (yq has none).

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
            "a: 1\nbb: 2\nccc: 3\n",
            "(.[] | select(column == 5)) = 0",
            "a: 1\nbb: 0\nccc: 3\n",
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
            "a: 1\nb: [2, x]\n",
            "(.. | select(kind == \"scalar\")) = 0",
            "a: 0\nb: [0, 0]\n",
        ),
                (
            "a: 1\nb: 2\n",
            "(.[] | select(file_index == 0)) = 0",
            "a: 0\nb: 0\n",
        ),
                (
            "a: 1\nb: 2\n",
            "(.[] | select(filename == \"\")) = 0",
            "a: 1\nb: 2\n",
        ),
        (
            "# h\na: 1\n\n# h2\nb: 2\n",
            "(.[] | select(head_comment != \"\")) = 0",
            "# h\na: 1\n# h2\nb: 2\n",
        ),
        (
            "a: 1\nb: 2\n# f\n",
            "(.. | select(foot_comment != \"\")) = 0",
            "a: 1\nb: 2\n# f\n",
        ),
        (
            "a: \"x\"\n---\nb: \"y\"\nc: z\n",
            "(.. | select(style == \"double\")) = 1",
            "a: 1\n---\nb: 1\nc: z\n",
        ),
        (
            "a: 1\n---\na: 2\n",
            "(.. | select(document_index == 1)) = 0",
            "a: 1\n---\n0\n",
        ),
        (
            "a:\n  - {k: 1}\n  - \"z\"\n  - 'w'\n",
            "del(.a[] | select(style == \"double\" or style == \"single\"))",
            "a:\n  - {k: 1}\n",
        ),
        (
            "\"a b\": \"x\"\n\"c.d\": \"y\"\n\"1\": \"w\"\n\"\": \"v\"\n\"g/h\": \"u\"\nplain: p\n\"q\\\"r\": \"z\"\n",
            "(.[] | select(style == \"double\")) = 9",
            "\"a b\": 9\n\"c.d\": 9\n\"1\": 9\n\"\": 9\n\"g/h\": 9\nplain: p\n\"q\\\"r\": 9\n",
        ),
        (
            "\"a b\": \"x\"\n\"c.d\": \"y\"\n\"1\": \"w\"\n\"\": \"v\"\n\"g/h\": \"u\"\nplain: p\n\"q\\\"r\": \"z\"\n",
            "del(.[] | select(style == \"double\"))",
            "plain: p\n",
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

#[test]
fn test_alternative_assignment_selecting_by_metadata_5479() -> Result<()> {
    // yq v4.53.3 has no `//=`; this is succinctly's operator reaching the same resolution.
    let (out, code) = run_yq_stdin("(.[] | select(line > 1)) //= 7", "a: 1\nb:\nc: 3\n", &[])?;
    assert_eq!((out.as_str(), code), ("a: 1\nb: 7\nc: 3\n", 0));
    Ok(())
}

#[test]
fn test_metadata_write_scales_linearly_with_its_matches_5479() -> Result<()> {
    // The rewrite once cost time quadratic in the number of matches (4,000 took over a second in
    // release and 11 s in debug), so a regression shows as a timeout.
    let input: String = (0..4000).map(|i| format!("k{i}: \"v{i}\"\n")).collect();
    let started = std::time::Instant::now();
    let (out, code) = run_yq_stdin("(.[] | select(style == \"double\")) = 1", &input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(out.lines().count(), 4000);
    assert!(out.lines().all(|line| line.ends_with(": 1")));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "took {:?}",
        started.elapsed()
    );
    Ok(())
}
