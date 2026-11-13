//! #7158: `succinctly yq -o kyaml` writes comments as yq's KYAML encoder does: the root's head comment above
//! the `{`, a mapping key's and its value's head comment above the entry, a line comment after the `,` (the
//! value's, else the key's, its first line only), a foot comment after the entry, the root's foot comment
//! after the document, and the first document's leading block (the comments and blank lines before it, a
//! `---` as written unless `-N`). Every row was captured from yq v4.53.3 (stdout, the first stderr line, the
//! exit status) and kept only when both tools agree: random comment positions over the document shapes of
//! the `-o xml` sweep (#7117), read as they are and through a write, and `-N`, `-I` and `-r`.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// (flags after `-o kyaml`, filter, input, yq's stdout, yq's first stderr line, yq's exit status).
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
    (&[], "del(.a)", "a: #  l  \n  b: 1 # l\n  #f\n# h1\n# h2\n  d: 2\n  c: 3\n  # f\n", "{}\n", "", 0),
    (&[], "del(.a)", "#h\na: 5 #  l  \n# f1\n# f2\nb:\n  c: 6\n", "#h\n{\n  # f1\n  # f2\n  b: {\n    c: 6,\n  },\n}\n", "", 0),
    (&[], ".zz = \"n\"", "a: 5\nb: #l\n  c: 6\n    #f\n", "{\n  a: 5,\n  b: {\n    c: 6,\n    #f\n  }, #l\n  zz: \"n\",\n}\n", "", 0),
    (&[], ".zz = \"n\"", "a: 1\n  #h\n+p_xml: version=\"1.0\" #l\nb: 2\n", "{\n  a: 1,\n  #h\n  \"+p_xml\": \"version=\\\"1.0\\\"\", #l\n  b: 2,\n  zz: \"n\",\n}\n", "", 0),
    (&[], ".zz.yy = 1", "# x # y\n\na: #l\n  +@id: 7\n# x # y\n  +content: t\n  c: 1\n", "# x # y\n\n{\n  a: {\n    \"+@id\": 7,\n    # x # y\n    \"+content\": \"t\",\n    c: 1,\n  }, #l\n  zz: {\n    yy: 1,\n  },\n}\n", "", 0),
    (&[], ".a = \"v\"", "# a b c\na: 1\nb: 2\n# a b c\nc: 3\n", "# a b c\n{\n  a: \"v\",\n  b: 2,\n  # a b c\n  c: 3,\n}\n", "", 0),
    (&[], "del(.a)", "# h\n\n---\na: 1\nb: 2\n#h\nc: 3\n", "# h\n\n---\n{\n  b: 2,\n  #h\n  c: 3,\n}\n", "", 0),
    (&[], ".zz = \"n\"", "# h1\n# h2\na:\n  b: 1\n#h\n  c: 2 #l\nd: 3\n# f\n", "# h1\n# h2\n{\n  a: {\n    b: 1,\n    #h\n    c: 2, #l\n  },\n  d: 3,\n  # f\n  zz: \"n\",\n}\n", "", 0),
    (&[], ".zz.yy = 1", "#\na: # a # b\n  - x: 1\n#  spaced\n  - x: 2\nb: 3\n", "#\n{\n  a: [\n    {\n      x: 1,\n      #  spaced\n    },\n    {\n      x: 2,\n    },\n  ], # a # b\n  b: 3,\n  zz: {\n    yy: 1,\n  },\n}\n", "", 0),
    (&[], "del(.a)", "r:\n  # h1\n  # h2\n  s:\n  #  spaced\n    t: 1 # a # b\n    u: 2 # l\n  v: 3\nw: 4 # a # b\n", "{\n  r: {\n    # h1\n    # h2\n    s: {\n      #  spaced\n      t: 1, # a # b\n      u: 2, # l\n    },\n    v: 3,\n  },\n  w: 4, # a # b\n}\n", "", 0),
    (&[], "del(.a)", "# a b c\na:\n# f\n  - - 1\n    - 2\n  - 3\n", "# a b c\n{}\n", "", 0),
    (&[], "sort_keys(.)", "a: 1\nb: 2\n#f\n# a b c\nc: 3\n", "{\n  a: 1,\n  b: 2,\n  #f\n  # a b c\n  c: 3,\n}\n", "", 0),
    (&[], ".a = \"v\"", "a: 1\nb: 2 #l\n", "{\n  a: \"v\",\n  b: 2, #l\n}\n", "", 0),
    (&[], ".zz.yy = 1", "#\n\na: #l\n  # f1\n  # f2\n    #\n  b: 1\n  c: 2 # l\nd: 3\n  #f\n", "#\n\n{\n  a: {\n    # f1\n    # f2\n    #\n    b: 1,\n    c: 2, # l\n  }, #l\n  d: 3,\n  #f\n  zz: {\n    yy: 1,\n  },\n}\n", "", 0),
    (&[], ".zz = \"n\"", "#h\n\na: 1\nb: 2\nc: 3\n  #f\n", "#h\n\n{\n  a: 1,\n  b: 2,\n  c: 3,\n  #f\n  zz: \"n\",\n}\n", "", 0),
    (&[], "sort_keys(.)", "  ## two\na:\n  # h1\n  # h2\n  b: 1\n  d: 2 #  l  \n  c: 3\n", "  ## two\n{\n  a: {\n    # h1\n    # h2\n    b: 1,\n    d: 2, #  l\n    c: 3,\n  },\n}\n", "", 0),
    (&[], ".zz = \"n\"", "#  spaced\na: 5\n# f\n# a b c\nb: # l\n    #h\n  c: 6\n  # f\n", "#  spaced\n{\n  a: 5,\n  # f\n  # a b c\n  b: {\n    #h\n    c: 6,\n    # f\n  }, # l\n  zz: \"n\",\n}\n", "", 0),
    (&[], "sort_keys(.)", "# x # y\nr:\n  s:\n    # h1\n    # h2\n    t: 1 # l\n    u: 2\n    # x # y\n  v: 3\nw: 4 #  l  \n  #f\n", "# x # y\n{\n  r: {\n    s: {\n      # h1\n      # h2\n      t: 1, # l\n      u: 2,\n      # x # y\n    },\n    v: 3,\n  },\n  w: 4, #  l\n  #f\n}\n", "", 0),
    (&[], ".a = \"v\"", "# x # y\na: # l\n  b: 1 #l\n  d: 2 # l\n  #h\n  c: 3 #  l  \n  #f\n", "# x # y\n{\n  a: \"v\", # l\n}\n", "", 0),
    (&[], ".zz.yy = 1", "#h\na:\n#f\n  - x: 1 #l\n  - x: 2\n#\nb: 3\n", "#h\n{\n  a: [\n    #f\n    {\n      x: 1, #l\n    },\n    {\n      x: 2,\n    },\n  ],\n  #\n  b: 3,\n  zz: {\n    yy: 1,\n  },\n}\n", "", 0),
    (&[], ".a = \"v\"", "#  spaced\n---\n# a b c\na:\n# f\n    # a b c\n  +@id: 7 #  l  \n  # f\n  +content: t\n    # a b c\n  c: 1\n#f\n", "#  spaced\n---\n# a b c\n{\n  a: \"v\",\n  #f\n}\n", "", 0),
    (&[], ".a = \"v\"", "a: #  l  \n  #\n  - x: 1\n  - x: 2\nb: 3\n", "{\n  a: \"v\", #  l\n  b: 3,\n}\n", "", 0),
    (&[], ".zz.yy = 1", "r: # l\n#f\n  s: #l\n    t: 1\n    u: 2\n  # a b c\n  v: 3\nw: 4 #  l  \n  #f\n", "{\n  r: {\n    #f\n    s: {\n      t: 1,\n      u: 2,\n    }, #l\n    # a b c\n    v: 3,\n  }, # l\n  w: 4, #  l\n  #f\n  zz: {\n    yy: 1,\n  },\n}\n", "", 0),
    (&[], ".", "a: 1\n# f1\n# f2\n+p_xml: version=\"1.0\"\n# h\nb: 2\n", "{\n  a: 1,\n  # f1\n  # f2\n  \"+p_xml\": \"version=\\\"1.0\\\"\",\n  # h\n  b: 2,\n}\n", "", 0),
    (&[], "sort_keys(.)", "#  spaced\na:\n#f\n  b: 1\n  c: 2\n#h\nd: 3\n", "#  spaced\n{\n  a: {\n    #f\n    b: 1,\n    c: 2,\n  },\n  #h\n  d: 3,\n}\n", "", 0),
    (&[], ".", "a: # l\n  - x: 1\n  - x: 2\nb: 3\n# f\n", "{\n  a: [\n    {\n      x: 1,\n    },\n    {\n      x: 2,\n    },\n  ], # l\n  b: 3,\n  # f\n}\n", "", 0),
    (&[], ".", "a: # a # b\n  # a b c\n  b: 1 # l\n#f\n  d: 2\n  c: 3\n", "{\n  a: {\n    # a b c\n    b: 1, # l\n    #f\n    d: 2,\n    c: 3,\n  }, # a # b\n}\n", "", 0),
    (&[], ".", "a: 1\n+p_xml: version=\"1.0\"\n# f1\n# f2\n#\nb: 2\n", "{\n  a: 1,\n  \"+p_xml\": \"version=\\\"1.0\\\"\",\n  # f1\n  # f2\n  #\n  b: 2,\n}\n", "", 0),
    (&[], ".", "#h\na:\n  - x: 1 # l\n# f\n  - x: 2\n  #  spaced\nb: 3\n", "#h\n{\n  a: [\n    {\n      x: 1, # l\n      # f\n    },\n    {\n      x: 2,\n      #  spaced\n    },\n  ],\n  b: 3,\n}\n", "", 0),
    (&[], "sort_keys(.)", "  # h1\n  # h2\nx:\n#f\n    # a b c\n  +p_target: data\n  y: 1\n", "  # h1\n  # h2\n{\n  x: {\n    #f\n    # a b c\n    \"+p_target\": \"data\",\n    y: 1,\n  },\n}\n", "", 0),
    (&["-N"], ".", "# h\n\na: 1 # l\n# f\nb:\n  - x # lx\n# tail\n", "# h\n\n{\n  a: 1, # l\n  # f\n  b: [\n    \"x\", # lx\n  ],\n  # tail\n}\n", "", 0),
    (&["-N"], ".", "---\n# first\na: 1 # l\n", "# first\n{\n  a: 1, # l\n}\n", "", 0),
    (&[], ".", "---\n# first\na: 1 # l\n", "---\n# first\n{\n  a: 1, # l\n}\n", "", 0),
    (&["-I", "4"], ".", "a: 1 # l\nb:\n  c: 2 # lc\n", "{\n    a: 1, # l\n    b: {\n        c: 2, # lc\n    },\n}\n", "", 0),
    (&["-I", "0"], ".", "a: 1 # l\nb: 2\n", "{\na: 1, # l\nb: 2,\n}\n", "", 0),
    (&["-r"], ".a", "a: 1 # l\nb: 2\n", "1\n", "", 0),
    (&[], ".", "a: # k\n  - 1 # i\n  - 2\n# foot\n", "{\n  a: [\n    1, # i\n    2,\n  ], # k\n  # foot\n}\n", "", 0),
];

#[test]
fn kyaml_output_writes_comments_as_yq_does_7158() -> Result<()> {
    for &(flags, filter, input, stdout, stderr, code) in ROWS {
        let mut args = vec!["-o", "kyaml"];
        args.extend_from_slice(flags);
        let (out, err, status) = run_yq_stdin_with_stderr(filter, input, &args)?;
        let row = format!("-o kyaml {flags:?} {filter:?} on {input:?}");
        assert_eq!(out, stdout, "stdout of {row}");
        assert_eq!(err.lines().next().unwrap_or(""), stderr, "stderr of {row}");
        assert_eq!(status, code, "exit status of {row}");
    }
    Ok(())
}
