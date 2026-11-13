//! #4380: a test of where a document came from (`select(fi == 1)`, `select(di == 0)`, joined by
//! `and`/`or`) in front of a navigation or a literal write keeps or drops each document on its own, so
//! `--eval-all` answers it with the per-document route that keeps the comments, anchors and style a
//! document says (the owned-value route prints none). Every row was captured from yq v4.53.3 over two files
//! and is kept where both tools agree. A first key's head comment in a later file that a `del` takes away is
//! the one difference (limitations.md).

use anyhow::Result;
use tempfile::TempDir;

use crate::run_yq_files;

/// (filter, first file, second file, yq's stdout).
#[rustfmt::skip]
const ROWS: &[(&str, &str, &str, &str)] = &[
    ("select(fi == 0)", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n"),
    ("select(fi == 0)", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n"),
    ("select(fi == 0)", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n"),
    ("select(fi == 1)", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n"),
    ("select(fi == 1)", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n"),
    ("select(fi == 1)", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n"),
    ("select(fi == 1) | .b", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "null\n"),
    ("select(fi == 1) | .b", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n", "null\n"),
    ("select(fi == 0) | .a", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "null\n"),
    ("select(fi == 0) | .a", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "null\n"),
    ("select(fi == 0) | .a", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "null\n"),
    ("select(fi != 0) | .d", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "null\n"),
    ("select(fi == 0) | .a = \"x\"", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\na: x\n"),
    ("select(fi == 0) | .a = \"x\"", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\na: x\n"),
    ("select(fi == 0) | .b.c = 9", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "# base head\na: 1 # one\nb:\n  # inner\n  c: 9\n# foot\n"),
    ("select(fi == 1) | .zz = 5", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\nzz: 5\n"),
    ("select(fi == 1) | .zz = 5", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n\nzz: 5\n"),
    ("select(fi == 1) | .zz = 5", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n\nzz: 5\n"),
    ("select(fi == 0) | del(.a)", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n"),
    ("select(di == 0)", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n---\n# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n"),
    ("select(di == 0)", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n---\n# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n"),
    ("select(di == 0)", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n---\n# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n"),
    ("select(fi == 0 or fi == 1) | .b", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "null\n---\nnull\n"),
    ("select(fi == 0 or fi == 1) | .b", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "null\n---\nc: 3 # three\n"),
    ("select(fi == 1) | .[0]", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "null\n"),
    ("select(fi == 1) | .[0]", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "null\n"),
    ("select(fi == 1) | .[0]", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "a\n"),
    ("select(fi == 0) | .. ", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n1\n# inner\nc: 2\n2\n"),
    ("select(fi == 0) | .. ", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\na\n# mid\nb: 1\nc: [x, y]\n1\n[x, y]\nx\ny\nz\n"),
    ("select(fi == 0) | .. ", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\na\n# mid\nb: 1\nc: [x, y]\n1\n[x, y]\nx\ny\nz\n"),
    ("select(fi == 1) | .l", "x: &x\n  p: 1 # p\ny: *x\nz: \"q\" # quoted\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "null\n"),
    ("select(fi < 1) | .k", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "null\n"),
    ("select(fi < 1) | .k", "k: v\n# tail\nl:\n  - 1 # a\n  - 2\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "v\n"),
    ("select(fi < 1) | .k", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "- a # one\n# mid\n- b: 1\n  c: [x, y]\n- \"z\"\n", "null\n"),
    ("select(fi == 0) | select(di == 0)", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n"),
    ("select(filename == \"{DIR}/f1.yaml\") | .b", "# base head\na: 1 # one\nb:\n  # inner\n  c: 2\n# foot\n", "# over head\nb:\n  c: 3 # three\nd: [1, 2] # list\n", "c: 3 # three\n"),
];

#[test]
fn an_origin_select_keeps_the_presentation_of_the_documents_it_keeps_4380() -> Result<()> {
    let dir = TempDir::new()?;
    let first = dir.path().join("f0.yaml");
    let second = dir.path().join("f1.yaml");
    for &(filter, a, b, expected) in ROWS {
        // A `filename` row names the second file by its place in the temporary directory.
        let filter = filter.replace("{DIR}", &dir.path().display().to_string());
        let filter = filter.as_str();
        std::fs::write(&first, a)?;
        std::fs::write(&second, b)?;
        let (stdout, stderr, status) = run_yq_files(
            filter,
            &[first.as_path(), second.as_path()],
            &["--eval-all"],
        )?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), status),
            (expected, "", 0),
            "{filter} over {a:?} and {b:?}"
        );
    }
    Ok(())
}

/// The one difference from yq (limitations.md): yq reads a later file's header comment as its first key's
/// head comment, so `del` of that key takes it away; this prints it. Pinned so a change shows.
#[test]
fn a_later_files_header_over_a_deleted_first_key_prints_here_4380() -> Result<()> {
    let dir = TempDir::new()?;
    let first = dir.path().join("f0.yaml");
    let second = dir.path().join("f1.yaml");
    std::fs::write(&first, "a: 1\n")?;
    std::fs::write(&second, "# over head\nb:\n  c: 3\nd: 4 # four\n")?;
    let (stdout, _, status) = run_yq_files(
        "select(fi == 1) | del(.b)",
        &[first.as_path(), second.as_path()],
        &["--eval-all"],
    )?;
    assert_eq!((stdout.as_str(), status), ("# over head\nd: 4 # four\n", 0));
    Ok(())
}
