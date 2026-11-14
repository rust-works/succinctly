//! #7160: `succinctly yq -o toml` writes the comments yq's TOML encoder writes: the root mapping's head
//! comment first, a table's head comment above its `[header]`, an attribute's head comment above it and its
//! line comment after it (two spaces, then the comment), an array's the same, and an array whose elements
//! have head comments in the multi-line form with each one above its element. A comment above a key is the
//! key's, which yq's encoder does not read, and no foot comment is written. Every row was captured from yq
//! v4.53.3 (stdout, the first stderr line, the exit status) and kept only when both tools agree: random
//! comment positions over the document shapes of the `-o xml` sweep (#7117), read as they are and through a
//! write, and tables, arrays of tables and arrays with comments. A run that read TOML is `i7170.rs`'s: the
//! `-p toml` read places each comment on the value node the encoder looks at.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// (flags after `-o toml`, filter, input, yq's stdout, yq's first stderr line, yq's exit status).
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
    (&[], "del(.a)", "# h\na: 1\nb: 2\n# f1\n# f2\n", "b = 2\n", "", 0),
    (&[], ".zz = \"n\"", "a:\n  # f\n  b: 1\n  #f\n  c: 2\nd: 3\n", "d = 3\nzz = \"n\"\n\n[a]\nb = 1\nc = 2\n", "", 0),
    (&[], ".zz = \"n\"", "a: 5 #l\nb:\n  c: 6\n", "a = 5  #l\nzz = \"n\"\n\n[b]\nc = 6\n", "", 0),
    (&[], ".", "x:\n  +p_target: data\n  y: 1\n", "[x]\n\"+p_target\" = \"data\"\ny = 1\n", "", 0),
    (&[], ".zz = \"n\"", "a: 1\nb: 2 #  l  \n  #f\nc: 3 #l\n", "a = 1\nb = 2  #  l\nc = 3  #l\nzz = \"n\"\n", "", 0),
    (&[], ".a = \"v\"", "r:\n    # h\n  s:\n    t: 1\n    u: 2\n  v: 3\n# h1\n# h2\nw: 4\n", "w = 4\na = \"v\"\n\n[r]\nv = 3\n\n[r.s]\nt = 1\nu = 2\n", "", 0),
    (&[], ".", "# a b c\na: 5\n  # h1\n  # h2\nb:\n  #  spaced\n  c: 6\n  # f\n", "a = 5\n\n[b]\nc = 6\n", "", 0),
    (&[], ".zz = \"n\"", "a:\n  b: 1 # a # b\n  c: 2\nd: 3 #  l  \n#f\n", "d = 3  #  l\nzz = \"n\"\n\n[a]\nb = 1  # a # b\nc = 2\n", "", 0),
    (&[], ".", "a:\n  - - 1\n    #  spaced\n    - 2\n  - 3\n", "a = [[1, 2], 3]\n", "", 0),
    (&[], "sort_keys(.)", "# a b c\na:\n  # f\n    #h\n  b: 1\n  # f\n  c: 2\n#f\nd: 3\n", "d = 3\n\n[a]\nb = 1\nc = 2\n", "", 0),
    (&[], ".", "a: 1\n  #f\n# h\nb: 2 # l\nc: 3\n", "a = 1\nb = 2  # l\nc = 3\n", "", 0),
    (&[], ".", "r:\n    #  spaced\n  s: # a # b\n    t: 1 #  l  \n    u: 2 # l\n  v: 3\nw: 4\n# f1\n# f2\n", "w = 4\n\n[r]\nv = 3\n\n[r.s]\nt = 1  #  l\nu = 2  # l\n", "", 0),
    (&[], "sort_keys(.)", "a: 5\nb:\n  #f\n    ## two\n  c: 6\n    #f\n# f\n", "a = 5\n\n[b]\nc = 6\n", "", 0),
    (&[], ".zz = \"n\"", "# x # y\n\n---\n#\nx: # a # b\n  # a b c\n  +p_target: data\n#f\n  y: 1\n", "zz = \"n\"\n\n[x]\n\"+p_target\" = \"data\"\ny = 1\n", "", 0),
    (&[], ".a = \"v\"", "a:\n  b: 1\n  # h\n  c: 2\n# f1\n# f2\nd: 3\n# f\n", "a = \"v\"\nd = 3\n", "", 0),
    (&[], "del(.a)", "a:\n# f\n  #\n  - x: 1\n  # a b c\n  - x: 2\nb: 3\n", "b = 3\n", "", 0),
    (&[], "sort_keys(.)", "r: #l\n  s:\n    t: 1\n    u: 2\n  v: 3\nw: 4\n#f\n", "w = 4\n\n[r]\nv = 3\n\n[r.s]\nt = 1\nu = 2\n", "", 0),
    (&[], ".zz.yy = 1", "## two\n---\na: 1\n  # a b c\n+p_xml: version=\"1.0\" #  l  \n# a b c\nb: 2\n", "a = 1\n\"+p_xml\" = \"version=\\\"1.0\\\"\"  #  l\nb = 2\n\n[zz]\nyy = 1\n", "", 0),
    (&[], "sort_keys(.)", "#\n\na: 1\n# f1\n# f2\nb: 2\n", "a = 1\nb = 2\n", "", 0),
    (&[], "sort_keys(.)", "# a b c\na:\n  - - 1\n# f\n    - 2\n  - 3\n  #f\n", "a = [[1, 2], 3]\n", "", 0),
    (&[], ".a = \"v\"", "## two\na: # a # b\n  b: 1 # l\n  c: 2\n  #h\nd: 3\n#f\n", "a = \"v\"\nd = 3\n", "", 0),
    (&[], "del(.a)", "x:\n  +p_target: data\n    # h1\n    # h2\n  y: 1 #l\n", "[x]\n\"+p_target\" = \"data\"\ny = 1  #l\n", "", 0),
    (&[], ".a = \"v\"", "  # x # y\na: 1\nb: 2 # l\n", "a = \"v\"\nb = 2  # l\n", "", 0),
    (&[], ".a = \"v\"", "r:\n  # h\n  s:\n    #f\n    t: 1\n    u: 2 # a # b\n  v: 3\n  # x # y\nw: 4\n", "w = 4\na = \"v\"\n\n[r]\nv = 3\n\n[r.s]\nt = 1\nu = 2  # a # b\n", "", 0),
    (&[], "del(.a)", "a:\n    #h\n  b: 1\n  c: 2 # a # b\n## two\nd: 3\n", "d = 3\n", "", 0),
    (&[], ".zz.yy = 1", "a: 5\nb: # a # b\n    #  spaced\n  c: 6 # a # b\n", "a = 5\n\n[b]\nc = 6  # a # b\n\n[zz]\nyy = 1\n", "", 0),
    (&[], ".zz.yy = 1", "# h\n\n#  spaced\na: 1 # l\nb: 2\n  # f1\n  # f2\n", "a = 1  # l\nb = 2\n\n[zz]\nyy = 1\n", "", 0),
    (&[], ".zz.yy = 1", "a: 1\n+p_xml: version=\"1.0\"\n# f1\n# f2\nb: 2\n", "a = 1\n\"+p_xml\" = \"version=\\\"1.0\\\"\"\nb = 2\n\n[zz]\nyy = 1\n", "", 0),
    (&[], ".zz.yy = 1", "# x # y\n\na: 1\n# h\nb: 2\n  # f\nc: 3\n", "a = 1\nb = 2\nc = 3\n\n[zz]\nyy = 1\n", "", 0),
    (&[], "del(.a)", "a:\n  - 1\n  #h\n  - 2\n#  spaced\nb: 3\n", "b = 3\n", "", 0),
    (&[], ".", "a: 1 # l\nt:\n  b: 2 # lb\n  c: [1, 2] # arr\n  d:\n    e: 3 # le\n", "a = 1  # l\n\n[t]\nb = 2  # lb\nc = [1, 2]  # arr\n\n[t.d]\ne = 3  # le\n", "", 0),
    (&[], ".", "list:\n  - x: 1 # lx\n    y: 2\n  # between\n  - x: 3 # l3\n", "[[list]]\nx = 1  # lx\ny = 2\n[[list]]\nx = 3  # l3\n", "", 0),
    (&[], ".", "arr:\n  # h1\n  - 1\n  - 2 # l2\n  # h3\n  - 3\n", "arr = [\n  # h1\n  1,\n\n  2,\n\n  # h3\n  3,\n]\n", "", 0),
    (&[], ".", "e: [] # empty\nb: 1 #nospace\nc: 3 #\n", "e = []  # empty\nb = 1  #nospace\nc = 3  #\n", "", 0),
    (&[], ".", "# root\n\na: 1\n", "a = 1\n", "", 0),
    (&[], ".zz = 1", "# root\na: 1 # l\nt:\n  b: 2 # lb\n", "a = 1  # l\nzz = 1\n\n[t]\nb = 2  # lb\n", "", 0),
    (&[], "del(.a)", "a: 1 # l\nb:\n  - 1 # x\n  # h\n  - 2\n", "b = [\n  1,\n\n  # h\n  2,\n]\n", "", 0),
    (&["-r"], ".a", "a: 1 # l\n", "1\n", "", 0),
];

#[test]
fn toml_output_writes_comments_as_yq_does_7160() -> Result<()> {
    for &(flags, filter, input, stdout, stderr, code) in ROWS {
        let mut args = vec!["-o", "toml"];
        args.extend_from_slice(flags);
        let (out, err, status) = run_yq_stdin_with_stderr(filter, input, &args)?;
        let row = format!("-o toml {flags:?} {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(err.lines().next().unwrap_or(""), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}
