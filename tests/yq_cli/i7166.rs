//! #7166: `succinctly yq -o props` keeps the comments of a write and of a rebuilt value, as yq's properties
//! encoder writes them: the filter runs without its encoder stage on the DOM route, and each result is
//! encoded with the presentation tree the YAML printer prints it with (a read took them off the cursor since
//! #6922, `tests/yq_cli/i6922.rs`). Every row was captured from yq v4.53.3 (stdout, the first stderr line,
//! the exit status) and kept only when both tools agree: random comment positions over the document shapes
//! of the `-o xml` sweep (#7117) through `.zz = "n"`, `del(.a)`, `.a = "v"`, `sort_keys(.)`, `.a |= .`, a
//! nested write and a mapping built around the document, and the property options.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// (flags after `-o props`, filter, input, yq's stdout, yq's first stderr line, yq's exit status).
type Row = (
    &'static [&'static str],
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i32,
);

#[rustfmt::skip]
const ROWS: &[Row] = &[
    (&[], ".zz.yy = 1", "## two\na: 5\nb:\n  c: 6\n", "## two\na = 5\nb.c = 6\nzz.yy = 1\n", "", 0),
    (&[], "{\"x\": .}", "a: #l\n  - 1\n  - 2 #l\n# h\nb: 3\n", "x.a.0 = 1\n\n#l\nx.a.1 = 2\n\n# h\nx.b = 3\n", "", 0),
    (&[], ".zz.yy = 1", "r: #l\n# x # y\n  s: #l\n  # x # y\n    t: 1 # l\n      #h\n    u: 2 # l\n  v: 3\n#\nw: 4\n# f1\n# f2\n", "# x # y l\nr.s.t = 1\n\n#h l\nr.s.u = 2\nr.v = 3\nw = 4\nzz.yy = 1\n", "", 0),
    (&[], ".zz = \"n\"", "a: 1\nb: 2 # a # b\n# f\n", "a = 1\n\n# a # b\nb = 2\nzz = n\n", "", 0),
    (&[], "sort_keys(.)", "a:\n  #h\n  - 1\n# x # y\n  - 2\nb: 3\n", "#h\na.0 = 1\n\n# x # y\na.1 = 2\nb = 3\n", "", 0),
    (&[], ".a |= .", "## two\na:\n  b: 1 # l\n#f\n  c: 2\n  # f\n# x # y\nd: 3\n#f\n", "## two\n# l\na.b = 1\n\n#f\na.c = 2\n\n# x # y\nd = 3\n", "", 0),
    (&[], ".a = \"v\"", "  # a b c\nx:\n  +p_target: data\n    #f\n  y: 1\n# f1\n# f2\n", "  # a b c\nx.+p_target = data\n\n#f\nx.y = 1\na = v\n", "", 0),
    (&[], "{\"x\": .}", "# a b c\nx:\n  +p_target: data\n#\n  y: 1\n", "x.x.+p_target = data\nx.x.y = 1\n", "", 0),
    (&[], ".zz = \"n\"", "# h1\n# h2\na:\n  - x: 1 # l\n  # f1\n  # f2\n  - x: 2 # l\n    # f1\n    # f2\nb: 3\n", "# h1\n# h2\n# l\na.0.x = 1\n\n# l\na.1.x = 2\nb = 3\nzz = n\n", "", 0),
    (&[], "sort_keys(.)", "a: 1\nb: 2\n", "a = 1\nb = 2\n", "", 0),
    (&[], ".a = \"v\"", "#  spaced\na: 1 # l\n  #\nb: 2\n  # f\nc: 3\n", "#  spaced\n# l\na = v\nb = 2\n\n# f\nc = 3\n", "", 0),
    (&[], ".zz.yy = 1", "# a b c\nr:\n  s:\n    t: 1\n    u: 2\n  v: 3\n    # f1\n    # f2\n## two\nw: 4 # l\n  # f1\n  # f2\n", "# a b c\nr.s.t = 1\nr.s.u = 2\nr.v = 3\n\n## two l\nw = 4\nzz.yy = 1\n", "", 0),
    (&[], "{\"x\": .}", "# x # y\nr: # l\n  s:\n      # h1\n      # h2\n    t: 1\n    u: 2\n#  spaced\n  v: 3\n  # h\nw: 4\n# f\n", "# h1\n# # h2\nx.r.s.t = 1\nx.r.s.u = 2\nx.r.v = 3\nx.w = 4\n", "", 0),
    (&[], "{\"x\": .}", "  ## two\na: 1 #l\nb: 2 #l\nc: 3\n", "#l\nx.a = 1\n\n#l\nx.b = 2\nx.c = 3\n", "", 0),
    (&[], ".a |= .", "#h\n  #h\na: 5 #l\n#f\nb:\n# f1\n# f2\n  c: 6\n#f\n", "#h\n  #h\n#l\na = 5\n\n# f1\n# # f2\nb.c = 6\n", "", 0),
    (&[], "del(.a)", "a: 5 #  l  \nb:\n  # a b c\n  c: 6 # a # b\n#f\n", "# a b c a # b\nb.c = 6\n", "", 0),
    (&[], "del(.a)", "#h\na:\n    #\n  - - 1\n    - 2 #l\n  - 3 # a # b\n  # f1\n  # f2\n", "#h\n", "", 0),
    (&[], ".zz = \"n\"", "  # x # y\na: 5 # l\nb:\n  c: 6 #l\n", "  # x # y\n# l\na = 5\n\n#l\nb.c = 6\nzz = n\n", "", 0),
    (&[], ".zz = \"n\"", "r:\n  s:\n    #\n    t: 1 #  l  \n    u: 2 #  l  \n  v: 3 # a # b\n  #  spaced\nw: 4 #  l  \n", "#  l  \nr.s.t = 1\n\n#  l  \nr.s.u = 2\n\n# a # b\nr.v = 3\n\n#  l  \nw = 4\nzz = n\n", "", 0),
    (&[], "sort_keys(.)", "a: 1\nb: 2\n# f\n#  spaced\nc: 3\n", "a = 1\nb = 2\n\n# f\n# #  spaced\nc = 3\n", "", 0),
    (&[], ".zz.yy = 1", "r:\n  ## two\n  s: #l\n    t: 1 #l\n    u: 2\n  # f\n  v: 3\n    # f\n# h1\n# h2\nw: 4\n", "#l\nr.s.t = 1\nr.s.u = 2\n\n# f\nr.v = 3\n\n# h1\n# # h2\nw = 4\nzz.yy = 1\n", "", 0),
    (&[], "{\"x\": .}", "a:\n#f\n  +@id: 7\n  +content: t\n  c: 1\n", "#f\nx.a.+@id = 7\nx.a.+content = t\nx.a.c = 1\n", "", 0),
    (&[], ".zz.yy = 1", "a:\n# h\n  b: 1\n## two\n  d: 2\n  #  spaced\n  c: 3\n", "# h\na.b = 1\n\n## two\na.d = 2\n\n#  spaced\na.c = 3\nzz.yy = 1\n", "", 0),
    (&[], ".zz = \"n\"", "#h\na: # a # b\n  b: 1\n  c: 2\nd: 3\n# f1\n# f2\n", "#h\na.b = 1\na.c = 2\nd = 3\nzz = n\n", "", 0),
    (&[], "sort_keys(.)", "a:\n  +@id: 7\n  +content: t #  l  \n  c: 1\n", "a.+@id = 7\n\n#  l  \na.+content = t\na.c = 1\n", "", 0),
    (&[], "sort_keys(.)", "#h\n  # x # y\na: 5\nb:\n    ## two\n  c: 6\n", "#h\n  # x # y\na = 5\n\n## two\nb.c = 6\n", "", 0),
    (&[], ".a |= .", "r:\n  # h1\n  # h2\n  s:\n# f1\n# f2\n      #  spaced\n    t: 1 #  l  \n    u: 2\n  v: 3 # a # b\nw: 4\n# f1\n# f2\n  # f1\n  # f2\n", "# f1\n# # f2\n# #  spaced  l  \nr.s.t = 1\nr.s.u = 2\n\n# a # b\nr.v = 3\nw = 4\na = null\n", "", 0),
    (&[], ".a |= .", "a: 1\n+p_xml: version=\"1.0\"\n#  spaced\nb: 2\n  #f\n", "a = 1\n+p_xml = version=\"1.0\"\n\n#  spaced\nb = 2\n", "", 0),
    (&[], "del(.a)", "# x # y\n\n---\na:\n  - 1\n  - 2 # a # b\nb: 3\n", "# x # y\n\nb = 3\n", "", 0),
    (&[], ".a |= .", "a: 1 # a # b\n#f\n# h\nb: 2\nc: 3 #l\n", "# a # b\na = 1\n\n#f\n# # h\nb = 2\n\n#l\nc = 3\n", "", 0),
    (&[], "del(.a)", "x: #l\n  +p_target: data\n#\n  y: 1 #  l  \n", "x.+p_target = data\n\n#  l  \nx.y = 1\n", "", 0),
    (&[], ".a = \"v\"", "#h\na: 1\nb: 2\nc: 3\n  # f1\n  # f2\n", "#h\na = v\nb = 2\nc = 3\n", "", 0),
    (&[], ".a = \"v\"", "#  spaced\na: 1\n+p_xml: version=\"1.0\"\n# h1\n# h2\nb: 2\n", "#  spaced\na = v\n+p_xml = version=\"1.0\"\n\n# h1\n# # h2\nb = 2\n", "", 0),
    (&[], ".a = \"v\"", "a: 1 #  l  \n#\nb: 2\nc: 3 # l\n", "#  l  \na = v\nb = 2\n\n# l\nc = 3\n", "", 0),
    (&[], "del(.a)", "#\n\n  #h\na: 1\nb: 2\nc: 3\n  #f\n", "#\n\n  #h\nb = 2\nc = 3\n", "", 0),
    (&[], ".a = 5", "# h\na: 1 # l\nb:\n  # hc\n  c: 2\nd:\n  - x # lx\n", "# h\n# l\na = 5\n\n# hc\nb.c = 2\n\n# lx\nd.0 = x\n", "", 0),
    (&[], ".zz = 1", "# top\n\na: 1 # l\n", "# top\n\n# l\na = 1\nzz = 1\n", "", 0),
    (&[], "del(.b)", "a: 1 # l\nb: 2 # m\n", "# l\na = 1\n", "", 0),
    (&["--properties-array-brackets"], ".a = 5", "a: 1 # l\nb:\n  - 1 # i\n", "# l\na = 5\n\n# i\nb[0] = 1\n", "", 0),
    (&["--properties-separator", ": "], ".a = 5", "a: 1 # l\nb: 2\n", "# l\na: 5\nb: 2\n", "", 0),
    (&["--unwrapScalar=false"], ".a = 5", "a: x y # l\n", "# l\na = 5\n", "", 0),
];

#[test]
fn props_output_keeps_the_comments_through_a_write_7166() -> Result<()> {
    for &(flags, filter, input, stdout, stderr, code) in ROWS {
        let mut args = vec!["-o", "props"];
        args.extend_from_slice(flags);
        let (out, err, status) = run_yq_stdin_with_stderr(filter, input, &args)?;
        let row = format!("-o props {flags:?} {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(err.lines().next().unwrap_or(""), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}
