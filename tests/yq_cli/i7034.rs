//! #7034: `head_comment` gives the comment after a bare `-` to the first key or element of the
//! block mapping or sequence the item holds, as yq does (`- # m` / `  a: 1` is
//! `.[0].a | key | head_comment` = `"m"`), and keeps the line break a blank line leaves between an
//! indented node and its comment (`a:` / `  #c1` / `` / `  n1: 3.5` is `"#c1\n"`), which
//! `-o=props` writes as one more `# ` line. Every answer was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

/// (input, filter, yq's stdout under `-o=json -I0`).
const HEAD_ROWS: &[(&str, &str, &str)] = &[
    ("- # m\n  - q\n", ".[0][0] | head_comment", "\"m\"\n"),
    (
        "- # m\n  a: 1\n",
        "[.[0] | head_comment, (.[0].a | key | head_comment)]",
        "[\"\",\"m\"]\n",
    ),
    (
        "- # m\n  - - q\n",
        "[.[0][0] | head_comment, (.[0][0][0] | head_comment)]",
        "[\"m\",\"\"]\n",
    ),
    (
        "- # m\n  - a: 1\n",
        "[.[0][0] | head_comment, (.[0][0].a | key | head_comment)]",
        "[\"m\",\"\"]\n",
    ),
    ("- # m\n  - q\n  - r\n", ".[0][1] | head_comment", "\"\"\n"),
    (
        "x:\n  - # m\n    a: 1\n",
        ".x[0].a | key | head_comment",
        "\"m\"\n",
    ),
    (
        "- # m\n  #1\n  - q\n",
        ".[0][0] | head_comment",
        "\"m\\n#1\"\n",
    ),
    (
        "- b: 1\n- #2\n  #1\n  c: 3.5\n",
        ".[1].c | key | head_comment",
        "\"#2\\n#1\"\n",
    ),
    (
        "- # d\n\n  # c\n  a: 1\n",
        ".[0].a | key | head_comment",
        "\"d\\n\\nc\"\n",
    ),
    (
        "- # d\n\n\n  # c\n  a: 1\n",
        ".[0].a | key | head_comment",
        "\"d\\n\\nc\"\n",
    ),
    (
        "- # c\n  {a: 1}\n",
        "[.[0] | head_comment, (.[0].a | key | head_comment)]",
        "[\"c\",\"\"]\n",
    ),
    ("- # c\n  x\n", ".[0] | head_comment", "\"c\"\n"),
    (
        "a:\n  #c1\n\n  n1: 3.5\n",
        ".a.n1 | key | head_comment",
        "\"#c1\\n\"\n",
    ),
    (
        "a:\n  #c1\n\n\n  n1: 3.5\n",
        ".a.n1 | key | head_comment",
        "\"#c1\\n\"\n",
    ),
    (
        "a:\n  #c1\n\n  #c2\n  n1: 3.5\n",
        ".a.n1 | key | head_comment",
        "\"#c1\\n\\n#c2\"\n",
    ),
    (
        "a:\n  x: 1\n  #c1\n\n  n1: 3.5\n",
        ".a.n1 | key | head_comment",
        "\"\"\n",
    ),
    (
        "a:\n  #c1\n\n  - 1\n",
        ".a[0] | head_comment",
        "\"#c1\\n\"\n",
    ),
    (
        "a:\n  b:\n    #c1\n\n    n: 1\n",
        ".a.b.n | key | head_comment",
        "\"#c1\\n\"\n",
    ),
    (
        "- #c1\n\n  n: 1\n",
        ".[0].n | key | head_comment",
        "\"#c1\\n\"\n",
    ),
    (
        "- # m\n  #\n  -\n    - 1\n",
        ".[0][0] | head_comment",
        "\"m\\n#\"\n",
    ),
    ("x: 0\n#c1\n\nk: 1\n", ".k | key | head_comment", "\"\"\n"),
];

const PROPS_ROWS: &[(&str, &str)] = &[
    ("a:\n  #c1\n\n  n1: 3.5\n", "#c1\n# \na.n1 = 3.5\n"),
    (
        "- # multi\n\n  - two words\n  - d\n",
        "# multi\n# \n0.0 = two words\n0.1 = d\n",
    ),
    (
        "- # m\n  - q\n- # e\n  - - 1\n",
        "# m\n0.0 = q\n1.0.0 = 1\n",
    ),
];

#[test]
fn head_comment_takes_the_dash_comment_and_the_blank_lines_break_7034() -> Result<()> {
    for (input, filter, expected) in HEAD_ROWS {
        let (stdout, status) = run_yq_stdin(filter, input, &["-o=json", "-I0"])?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, 0),
            "{filter} over {input:?}"
        );
    }
    Ok(())
}

#[test]
fn props_writes_the_blank_lines_break_7034() -> Result<()> {
    for (input, expected) in PROPS_ROWS {
        let (stdout, status) = run_yq_stdin(".", input, &["-o=props"])?;
        assert_eq!((stdout.as_str(), status), (*expected, 0), "{input:?}");
    }
    Ok(())
}
