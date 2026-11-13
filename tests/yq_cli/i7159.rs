//! #7159: `succinctly yq -o lua` writes a line comment as ` -- text` after the entry or item it is on, as
//! yq's Lua encoder does: after a key's `["k"] =` and a line break when the comment is on the key, after
//! the `;` of a mapping value, after the `,` of an array item. A head or foot comment is not written (yq
//! writes none). Every row was captured from yq v4.53.3 (stdout, the first stderr line, the exit status)
//! and kept only when both tools agree: random comment positions over the document shapes of the
//! `-o xml` sweep (#7117), read as they are and through a write (`.zz = "n"`, `del(.a)`, `.a = "v"`,
//! `sort_keys(.)`, a nested write), and the `--lua-*` options.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// (flags after `-o lua`, filter, input, yq's stdout, yq's first stderr line, yq's exit status).
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
    (&[], ".a = \"v\"", "## two\n\na: 5\nb: # l\n  c: 6\n  # f\n", "return {\n\t[\"a\"] = \"v\";\n\t[\"b\"] = -- l\n\t{\n\t\t[\"c\"] = 6;\n\t};\n};\n", "", 0),
    (&[], ".zz = \"n\"", "  #  spaced\na: 1 # l\nb: 2 #  l  \n", "return {\n\t[\"a\"] = 1; -- l\n\t[\"b\"] = 2; --  l  \n\t[\"zz\"] = \"n\";\n};\n", "", 0),
    (&[], "sort_keys(.)", "## two\n---\nr:\n  s: #l\n  # f\n    t: 1 #l\n    #h\n    u: 2\n  v: 3\nw: 4 # l\n", "return {\n\t[\"r\"] = {\n\t\t[\"s\"] = --l\n\t\t{\n\t\t\t[\"t\"] = 1; --l\n\t\t\t[\"u\"] = 2;\n\t\t};\n\t\t[\"v\"] = 3;\n\t};\n\t[\"w\"] = 4; -- l\n};\n", "", 0),
    (&[], ".a = \"v\"", "a:\n#f\n  - 1\n  - 2 #  l  \n# a b c\nb: 3 #l\n", "return {\n\t[\"a\"] = \"v\";\n\t[\"b\"] = 3; --l\n};\n", "", 0),
    (&[], "del(.a)", "a: 5\n# x # y\nb:\n  c: 6\n#f\n", "return {\n\t[\"b\"] = {\n\t\t[\"c\"] = 6;\n\t};\n};\n", "", 0),
    (&[], "sort_keys(.)", "# x # y\nx: #  l  \n# f1\n# f2\n  +p_target: data\n  y: 1\n", "return {\n\t[\"x\"] = --  l  \n\t{\n\t\t[\"+p_target\"] = \"data\";\n\t\t[\"y\"] = 1;\n\t};\n};\n", "", 0),
    (&[], ".zz = \"n\"", "# h1\n# h2\na:\n# f\n  - x: 1 #  l  \n  - x: 2\nb: 3\n", "return {\n\t[\"a\"] = {\n\t\t{\n\t\t\t[\"x\"] = 1; --  l  \n\t\t},\n\t\t{\n\t\t\t[\"x\"] = 2;\n\t\t},\n\t};\n\t[\"b\"] = 3;\n\t[\"zz\"] = \"n\";\n};\n", "", 0),
    (&[], ".zz = \"n\"", "  #  spaced\na: 1\n# a b c\nb: 2 #l\n# f\n# h\nc: 3 # l\n", "return {\n\t[\"a\"] = 1;\n\t[\"b\"] = 2; --l\n\t[\"c\"] = 3; -- l\n\t[\"zz\"] = \"n\";\n};\n", "", 0),
    (&[], "del(.a)", "## two\n\n---\na: 1 #  l  \nb: 2 #l\n", "return {\n\t[\"b\"] = 2; --l\n};\n", "", 0),
    (&[], ".zz.yy = 1", "# h\na:\n  ## two\n  b: 1\n  c: 2\n# a b c\nd: 3\n  # f1\n  # f2\n", "return {\n\t[\"a\"] = {\n\t\t[\"b\"] = 1;\n\t\t[\"c\"] = 2;\n\t};\n\t[\"d\"] = 3;\n\t[\"zz\"] = {\n\t\t[\"yy\"] = 1;\n\t};\n};\n", "", 0),
    (&[], ".", "a: #  l  \n  - - 1\n    - 2\n#h\n  - 3\n", "return {\n\t[\"a\"] = --  l  \n\t{\n\t\t{\n\t\t\t1,\n\t\t\t2,\n\t\t},\n\t\t3,\n\t};\n};\n", "", 0),
    (&[], ".", "# a b c\na: 1\nb: 2\nc: 3\n#f\n", "return {\n\t[\"a\"] = 1;\n\t[\"b\"] = 2;\n\t[\"c\"] = 3;\n};\n", "", 0),
    (&[], "sort_keys(.)", "a: # l\n  b: 1 # a # b\n  c: 2\nd: 3\n  #f\n", "return {\n\t[\"a\"] = -- l\n\t{\n\t\t[\"b\"] = 1; -- a # b\n\t\t[\"c\"] = 2;\n\t};\n\t[\"d\"] = 3;\n};\n", "", 0),
    (&[], ".", "# h\na:\n  - - 1\n      # h\n    - 2 # l\n#\n  - 3\n# f\n", "return {\n\t[\"a\"] = {\n\t\t{\n\t\t\t1,\n\t\t\t2, -- l\n\t\t},\n\t\t3,\n\t};\n};\n", "", 0),
    (&[], ".zz.yy = 1", "a: #l\n  +@id: 7\n# a b c\n  +content: t\n  c: 1\n", "return {\n\t[\"a\"] = --l\n\t{\n\t\t[\"+@id\"] = 7;\n\t\t[\"+content\"] = \"t\";\n\t\t[\"c\"] = 1;\n\t};\n\t[\"zz\"] = {\n\t\t[\"yy\"] = 1;\n\t};\n};\n", "", 0),
    (&[], ".", "a: 5\nb: # l\n  c: 6 # l\n", "return {\n\t[\"a\"] = 5;\n\t[\"b\"] = -- l\n\t{\n\t\t[\"c\"] = 6; -- l\n\t};\n};\n", "", 0),
    (&[], ".zz.yy = 1", "a:\n# f1\n# f2\n  b: 1 # a # b\n  c: 2\nd: 3\n", "return {\n\t[\"a\"] = {\n\t\t[\"b\"] = 1; -- a # b\n\t\t[\"c\"] = 2;\n\t};\n\t[\"d\"] = 3;\n\t[\"zz\"] = {\n\t\t[\"yy\"] = 1;\n\t};\n};\n", "", 0),
    (&[], "sort_keys(.)", "x:\n#\n  +p_target: data\n  # a b c\n  y: 1\n", "return {\n\t[\"x\"] = {\n\t\t[\"+p_target\"] = \"data\";\n\t\t[\"y\"] = 1;\n\t};\n};\n", "", 0),
    (&[], "sort_keys(.)", "# a b c\n\na: 1\nb: 2\n  # f\n", "return {\n\t[\"a\"] = 1;\n\t[\"b\"] = 2;\n};\n", "", 0),
    (&[], ".zz.yy = 1", "# h\na: 1\nb: 2 # l\n", "return {\n\t[\"a\"] = 1;\n\t[\"b\"] = 2; -- l\n\t[\"zz\"] = {\n\t\t[\"yy\"] = 1;\n\t};\n};\n", "", 0),
    (&[], ".zz.yy = 1", "a: 1\n+p_xml: version=\"1.0\" #l\nb: 2\n# f\n", "return {\n\t[\"a\"] = 1;\n\t[\"+p_xml\"] = \"version=\\\"1.0\\\"\"; --l\n\t[\"b\"] = 2;\n\t[\"zz\"] = {\n\t\t[\"yy\"] = 1;\n\t};\n};\n", "", 0),
    (&[], ".a = \"v\"", "# h\na: 1\n+p_xml: version=\"1.0\" #  l  \n#\nb: 2\n", "return {\n\t[\"a\"] = \"v\";\n\t[\"+p_xml\"] = \"version=\\\"1.0\\\"\"; --  l  \n\t[\"b\"] = 2;\n};\n", "", 0),
    (&[], "del(.a)", "r:\n  #  spaced\n  s: #  l  \n    t: 1\n    u: 2\n# f1\n# f2\n  v: 3\n## two\nw: 4 #  l  \n", "return {\n\t[\"r\"] = {\n\t\t[\"s\"] = --  l  \n\t\t{\n\t\t\t[\"t\"] = 1;\n\t\t\t[\"u\"] = 2;\n\t\t};\n\t\t[\"v\"] = 3;\n\t};\n\t[\"w\"] = 4; --  l  \n};\n", "", 0),
    (&[], ".a = \"v\"", "# x # y\n---\na: #  l  \n  #\n  b: 1\n# f1\n# f2\n  d: 2\n  c: 3 #l\n", "return {\n\t[\"a\"] = --  l  \n\t\"v\";\n};\n", "", 0),
    (&[], ".zz = \"n\"", "# x # y\n\nr:\n  s:\n    ## two\n    t: 1 # a # b\n    u: 2 # l\n  v: 3 # a # b\n  #h\nw: 4\n", "return {\n\t[\"r\"] = {\n\t\t[\"s\"] = {\n\t\t\t[\"t\"] = 1; -- a # b\n\t\t\t[\"u\"] = 2; -- l\n\t\t};\n\t\t[\"v\"] = 3; -- a # b\n\t};\n\t[\"w\"] = 4;\n\t[\"zz\"] = \"n\";\n};\n", "", 0),
    (&[], ".zz = \"n\"", "a:\n  - - 1 #  l  \n    - 2\n  - 3 # l\n", "return {\n\t[\"a\"] = {\n\t\t{\n\t\t\t1, --  l  \n\t\t\t2,\n\t\t},\n\t\t3, -- l\n\t};\n\t[\"zz\"] = \"n\";\n};\n", "", 0),
    (&[], "del(.a)", "## two\nx: # l\n  +p_target: data\n  y: 1\n", "return {\n\t[\"x\"] = -- l\n\t{\n\t\t[\"+p_target\"] = \"data\";\n\t\t[\"y\"] = 1;\n\t};\n};\n", "", 0),
    (&[], "del(.a)", "a:\n  - - 1\n    - 2 #l\n  - 3 # l\n", "return {};\n", "", 0),
    (&[], ".", "x: # a # b\n# h\n  +p_target: data #  l  \n  # a b c\n  y: 1\n", "return {\n\t[\"x\"] = -- a # b\n\t{\n\t\t[\"+p_target\"] = \"data\"; --  l  \n\t\t[\"y\"] = 1;\n\t};\n};\n", "", 0),
    (&[], ".a = \"v\"", "# h1\n# h2\n\n---\na: #  l  \n  - x: 1 #l\n  - x: 2 #  l  \nb: 3 # l\n", "return {\n\t[\"a\"] = --  l  \n\t\"v\";\n\t[\"b\"] = 3; -- l\n};\n", "", 0),
    (&["--lua-unquoted"], ".", "a: 1 # l\nb:\n  c: 2 # lc\n", "return {\n\ta = 1; -- l\n\tb = {\n\t\tc = 2; -- lc\n\t};\n};\n", "", 0),
    (&["--lua-globals"], ".", "a: 1 # l\nb:\n  c: 2 # lc\n", "a = 1; -- l\nb = {\n\t[\"c\"] = 2; -- lc\n};\n", "", 0),
    (&["--lua-prefix", "x = ", "--lua-suffix", ";"], ".", "a: 1 # l\n", "x = {\n\t[\"a\"] = 1; -- l\n};", "", 0),
    (&[], ".", "a: # k\n  - 1 # i\n  - 2\n", "return {\n\t[\"a\"] = -- k\n\t{\n\t\t1, -- i\n\t\t2,\n\t};\n};\n", "", 0),
];

#[test]
fn lua_output_writes_line_comments_as_yq_does_7159() -> Result<()> {
    for &(flags, filter, input, stdout, stderr, code) in ROWS {
        let mut args = vec!["-o", "lua"];
        args.extend_from_slice(flags);
        let (out, err, status) = run_yq_stdin_with_stderr(filter, input, &args)?;
        let row = format!("-o lua {flags:?} {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(err.lines().next().unwrap_or(""), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}
