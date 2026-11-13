//! #5439: `load("f.yaml")` prints the loaded document with what the file said about its nodes: comments,
//! `&anchor`s and `!tag`s, quote and flow style. The evaluator's result is still the one printed; a replay
//! with the file's comment tree (`reshape_presentation::load_document_pair`) supplies the presentation and
//! is used only when its value is the evaluator's. It covers `load` itself, `. * load(..)`, `. + load(..)`,
//! `. *= load(..)`, `. += load(..)`, `.a.b = load(..)` and navigation after it, with a file input and
//! under `-n`. Every row was captured from yq v4.53.3 (rows where this build still differs are not here:
//! a file whose `---` line has a comment or whose header a blank line detaches keeps the old output, and so
//! do a construction around the `load`, an anchored flow collection under a merge, and a blank line
//! before a document's foot comment).

use anyhow::Result;
use std::process::{Command, Stdio};
use tempfile::TempDir;

/// (filter, `-n`, the loaded file `f.yaml`, the input file `in.yaml`, yq's stdout).
#[rustfmt::skip]
const ROWS: &[(&str, bool, &str, &str, &str)] = &[
    ("load(\"f.yaml\")", false, "a: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n", "a: old # keep\nl: [9]\n", "a: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n"),
    ("load(\"f.yaml\")", true, "a: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n", "a: old # keep\nl: [9]\n", "a: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n"),
    (". * load(\"f.yaml\")", false, "a: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n", "x: 1 # one\n", "x: 1 # one\na: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n"),
    (". + load(\"f.yaml\")", false, "a: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n", "x: 1 # one\n", "x: 1 # one\na: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n"),
    (". *= load(\"f.yaml\")", false, "a: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n", "# top\nx: 1\ny:\n  - a\n", "# top\nx: 1\ny:\n  - a\na: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n"),
    (".y = load(\"f.yaml\")", false, "a: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n", "# top\nx: 1\ny:\n  - a\n", "# top\nx: 1\ny:\n  a: &x !Foo 1 # c\n  b: \"q\"\n  l: [1, 2]\n"),
    (".y = load(\"f.yaml\")", true, "a: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n", "# top\nx: 1\ny:\n  - a\n", "y:\n  a: &x !Foo 1 # c\n  b: \"q\"\n  l: [1, 2]\n"),
    ("load(\"f.yaml\") | .l", true, "a: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n", "# top\nx: 1\ny:\n  - a\n", "[1, 2]\n"),
    ("load(\"f.yaml\") | keys", true, "a: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n", "x: 1 # one\n", "- a\n- b\n- l\n"),
    ("load(\"f.yaml\")", false, "# head\na: 1 # one\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n", "x: 1 # one\n", "# head\na: 1 # one\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n"),
    ("load(\"f.yaml\")", true, "# head\na: 1 # one\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n", "x: 1 # one\n", "# head\na: 1 # one\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n"),
    (". * load(\"f.yaml\")", false, "# head\na: 1 # one\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n", "# top\nx: 1\ny:\n  - a\n", "# top\nx: 1\ny:\n  - a\n# head\na: 1 # one\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n"),
    (". + load(\"f.yaml\")", false, "# head\na: 1 # one\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n", "# top\nx: 1\ny:\n  - a\n", "# top\nx: 1\ny:\n  - a\n# head\na: 1 # one\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n"),
    (". *= load(\"f.yaml\")", false, "# head\na: 1 # one\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n", "a: old # keep\nl: [9]\n", "# head\na: 1 # one\nl: [9]\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n"),
    (".y = load(\"f.yaml\")", false, "# head\na: 1 # one\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n", "a: old # keep\nl: [9]\n", "a: old # keep\nl: [9]\ny:\n  # head\n  a: 1 # one\n  b:\n    # inner\n    c: 2\n    d: [3, 4] # tail\n  # foot\n"),
    (".y = load(\"f.yaml\")", true, "# head\na: 1 # one\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n", "a: old # keep\nl: [9]\n", "y:\n  # head\n  a: 1 # one\n  b:\n    # inner\n    c: 2\n    d: [3, 4] # tail\n  # foot\n"),
    ("load(\"f.yaml\") | .l", true, "# head\na: 1 # one\nb:\n  # inner\n  c: 2\n  d: [3, 4] # tail\n# foot\n", "a: old # keep\nl: [9]\n", "null\n"),
    ("load(\"f.yaml\")", false, "- a # x\n# h\n- b: 1\n  c: !!str 2\n- |\n  lit\n", "# top\nx: 1\ny:\n  - a\n", "- a # x\n# h\n- b: 1\n  c: !!str 2\n- |\n  lit\n"),
    ("load(\"f.yaml\")", true, "- a # x\n# h\n- b: 1\n  c: !!str 2\n- |\n  lit\n", "# top\nx: 1\ny:\n  - a\n", "- a # x\n# h\n- b: 1\n  c: !!str 2\n- |\n  lit\n"),
    (".y = load(\"f.yaml\")", false, "- a # x\n# h\n- b: 1\n  c: !!str 2\n- |\n  lit\n", "x: 1 # one\n", "x: 1 # one\ny:\n  - a # x\n  # h\n  - b: 1\n    c: !!str 2\n  - |\n    lit\n"),
    (".y = load(\"f.yaml\")", true, "- a # x\n# h\n- b: 1\n  c: !!str 2\n- |\n  lit\n", "x: 1 # one\n", "y:\n  - a # x\n  # h\n  - b: 1\n    c: !!str 2\n  - |\n    lit\n"),
    ("load(\"f.yaml\") | keys", true, "- a # x\n# h\n- b: 1\n  c: !!str 2\n- |\n  lit\n", "a: old # keep\nl: [9]\n", "- 0\n- 1\n- 2\n"),
    ("load(\"f.yaml\")", false, "a: &x 1\nb: *x\nc: 't'\n", "a: old # keep\nl: [9]\n", "a: &x 1\nb: *x\nc: 't'\n"),
    ("load(\"f.yaml\")", true, "a: &x 1\nb: *x\nc: 't'\n", "a: old # keep\nl: [9]\n", "a: &x 1\nb: *x\nc: 't'\n"),
    (". * load(\"f.yaml\")", false, "a: &x 1\nb: *x\nc: 't'\n", "x: 1 # one\n", "x: 1 # one\na: &x 1\nb: *x\nc: 't'\n"),
    (". + load(\"f.yaml\")", false, "a: &x 1\nb: *x\nc: 't'\n", "x: 1 # one\n", "x: 1 # one\na: &x 1\nb: *x\nc: 't'\n"),
    (". *= load(\"f.yaml\")", false, "a: &x 1\nb: *x\nc: 't'\n", "# top\nx: 1\ny:\n  - a\n", "# top\nx: 1\ny:\n  - a\na: &x 1\nb: *x\nc: 't'\n"),
    (".y = load(\"f.yaml\")", false, "a: &x 1\nb: *x\nc: 't'\n", "# top\nx: 1\ny:\n  - a\n", "# top\nx: 1\ny:\n  a: &x 1\n  b: *x\n  c: 't'\n"),
    (".y = load(\"f.yaml\")", true, "a: &x 1\nb: *x\nc: 't'\n", "# top\nx: 1\ny:\n  - a\n", "y:\n  a: &x 1\n  b: *x\n  c: 't'\n"),
    ("load(\"f.yaml\") | .l", true, "a: &x 1\nb: *x\nc: 't'\n", "# top\nx: 1\ny:\n  - a\n", "null\n"),
    ("load(\"f.yaml\") | keys", true, "a: &x 1\nb: *x\nc: 't'\n", "x: 1 # one\n", "- a\n- b\n- c\n"),
];

#[test]
fn load_keeps_the_presentation_of_the_loaded_file_as_yq_does_5439() -> Result<()> {
    let dir = TempDir::new()?;
    for (filter, null_input, loaded, input, expected) in ROWS {
        std::fs::write(dir.path().join("f.yaml"), loaded)?;
        std::fs::write(dir.path().join("in.yaml"), input)?;
        let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
        command
            .arg("yq")
            .current_dir(dir.path())
            .stdin(Stdio::null());
        if *null_input {
            command.arg("-n");
        }
        command.arg(filter);
        if !*null_input {
            command.arg("in.yaml");
        }
        let output = command.output()?;
        assert_eq!(
            String::from_utf8(output.stdout)?,
            *expected,
            "{filter} (-n: {null_input}) over {loaded:?} and {input:?}"
        );
    }
    Ok(())
}

/// The headline of the issue: tag, anchor, comment, quote and flow style of a loaded file.
#[test]
fn load_prints_a_tagged_anchored_commented_file_unchanged_5439() -> Result<()> {
    let dir = TempDir::new()?;
    let text = "a: &x !Foo 1 # c\nb: \"q\"\nl: [1, 2]\n";
    std::fs::write(dir.path().join("l.yaml"), text)?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["yq", "-n", "load(\"l.yaml\")"])
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()?;
    assert_eq!(String::from_utf8(output.stdout)?, text);
    Ok(())
}
