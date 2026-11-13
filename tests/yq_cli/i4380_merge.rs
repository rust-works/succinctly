//! #4380: the file-merge idioms of `--eval-all` (`yq ea`) keep what each document says about its nodes:
//! `. as $item ireduce ({}; . * $item)` and `select(fi == 0) * select(fi == 1)` (and `+`, `*+`, with the
//! files in either order). The evaluator's own result is still the one printed; a replay of the merge with
//! the documents' comment trees (`reshape_presentation::trace_eval_all_merge`) supplies the comments, and is
//! used only when it gives the same value. Every row was captured from yq v4.53.3: a head, line or foot
//! comment of either file, the right file's replacing the left's where it has one (a head comment above a
//! key, a line comment after a value), the first file's header leading the result and a later file's header
//! heading its first key. What is not modelled falls back to the old output without comments: a later file
//! whose header a blank line detaches, an anchor or an alias, a source with several documents or that is
//! not YAML.

use anyhow::Result;
use tempfile::TempDir;

use crate::run_yq_files;

/// (filter, first file, second file, yq's stdout).
#[rustfmt::skip]
const ROWS: &[(&str, &str, &str, &str)] = &[
    (". as $i ireduce ({}; . *+ $i)", "r:\n  s:\n# f1\n# f2\n  #  spaced\n    t: 9\n  z: 8 # l\nw: 7\n", "a: 1\nb: 2 # a # b\n", "r:\n  s:\n    # f1\n    # f2\n    #  spaced\n    t: 9\n  z: 8 # l\nw: 7\na: 1\nb: 2 # a # b\n"),
    (". as $i ireduce ({}; . *+ $i)", "a:\n  # h1\n  # h2\n  b: 1\n# h\n  c: 2 # a # b\n#  spaced\nd: 3 #  l  \n# f1\n# f2\n", "# h\na: x\nb: y #l\n", "# h\na: x\n#  spaced\nd: 3 #  l  \n# f1\n# f2\n\nb: y #l\n"),
    ("select(fi == 1) * select(fi == 0)", "# h\n\n# a b c\na: 1 #  l  \n# a b c\nc: 3\n# f1\n# f2\n", "## two\na:\n  b: 1 #  l  \n  c: 2\nd: 3 #  l  \n# f\n", "# h\n\n# a b c\n## two\na: 1 #  l  \nd: 3 #  l  \n# f\n\n# a b c\nc: 3\n# f1\n# f2\n"),
    ("select(fi == 0) * select(fi == 1)", "# h1\n# h2\n\n# h1\n# h2\na: 1\n## two\nc: 3 #  l  \n", "# h\nr:\n  s:\n  # h1\n  # h2\n    t: 9\n  z: 8\nw: 7 #l\n", "# h1\n# h2\n\n# h1\n# h2\na: 1\n## two\nc: 3 #  l  \n# h\nr:\n  s:\n    # h1\n    # h2\n    t: 9\n  z: 8\nw: 7 #l\n"),
    ("select(fi == 1) * select(fi == 0)", "a: 1\nb: 2\n# f\n", "#  spaced\n# h1\n# h2\na:\n# h1\n# h2\n  b: 9\n  e: 5\n#  spaced\nd: 4\n", "#  spaced\n# h1\n# h2\na: 1\n#  spaced\nd: 4\nb: 2\n# f\n"),
    (". as $item ireduce ({}; . * $item )", "a: #  l  \n# f\n  b: 1\n  c: 2\nd: 3\n", "r:\n  s:\n    t: 9\n  z: 8\nw: 7 #l\n", "a: #  l  \n  # f\n  b: 1\n  c: 2\nd: 3\nr:\n  s:\n    t: 9\n  z: 8\nw: 7 #l\n"),
    ("select(fi == 0) + select(fi == 1)", "r:\n  s:\n    # a b c\n    t: 1 # a # b\n    u: 2\n    # f1\n    # f2\n  # h1\n  # h2\n  v: 3\n## two\nw: 4 # l\n", "a: #l\n  - 7\nc: 3\n", "r:\n  s:\n    # a b c\n    t: 1 # a # b\n    u: 2\n    # f1\n    # f2\n  # h1\n  # h2\n  v: 3\n## two\nw: 4 # l\na: #l\n  - 7\nc: 3\n"),
    (". as $i ireduce ({}; . *+ $i)", "a: x\nb: y\n", "r: #  l  \n  s:\n    t: 1\n    u: 2\n  # h1\n  # h2\n  v: 3\nw: 4\n# f1\n# f2\n", "a: x\nb: y\nr: #  l  \n  s:\n    t: 1\n    u: 2\n  # h1\n  # h2\n  v: 3\nw: 4\n# f1\n# f2\n"),
    ("select(fi == 0) + select(fi == 1)", "# a b c\n# a b c\na:\n#h\n  - 7\nc: 3\n", "# h1\n# h2\na:\n  ## two\n  b: 1 #  l  \n  #h\n  c: 2\n#  spaced\nd: 3\n", "# a b c\n# a b c\na:\n  ## two\n  b: 1 #  l  \n  #h\n  c: 2\nc: 3\n#  spaced\nd: 3\n"),
    ("select(fi == 0) + select(fi == 1)", "a: #l\n  # a b c\n  - x: 1 #l\n  - x: 2 #  l  \nb: 3\n# f\n", "a: #  l  \n# f\n  - 1 #l\n  - 2 #  l  \nb: 3\n# f1\n# f2\n", "a: #l\n  # f\n  - 1 #l\n  - 2 #  l  \nb: 3\n# f\n"),
    ("select(fi == 0) * select(fi == 1)", "a: x\nb: y\n", "a: 1 #l\nc: 3\n# f1\n# f2\n", "a: 1 #l\nb: y\nc: 3\n# f1\n# f2\n"),
    ("select(fi == 0) + select(fi == 1)", "# h1\n# h2\n# h1\n# h2\na:\n  # a b c\n  b: 1\n# h1\n# h2\n  c: 2 #l\nd: 3\n# f\n", "a: #  l  \n  b: 9 # l\n# h1\n# h2\n  e: 5\n## two\nd: 4\n", "# h1\n# h2\n# h1\n# h2\na:\n  b: 9 # l\n  # h1\n  # h2\n  e: 5\nd: 4\n# f\n"),
    ("select(fi == 0) + select(fi == 1)", "a:\n  b: 1\n  c: 2\n  # f1\n  # f2\nd: 3\n# f1\n# f2\n# f\n", "a: 1\nc: 3 #l\n", "a: 1\nd: 3\n# f1\n# f2\n# f\n\nc: 3 #l\n"),
    ("select(fi == 0) * select(fi == 1)", "a:\n  #  spaced\n  b: 9\n  e: 5 # a # b\nd: 4 # a # b\n", "#  spaced\na: 1 #l\nc: 3\n# f1\n# f2\n", "#  spaced\na: 1 #l\nd: 4 # a # b\nc: 3\n# f1\n# f2\n"),
    ("select(fi == 1) * select(fi == 0)", "# h\na: # l\n#h\n  - x: 1 #  l  \n  - x: 2\nb: 3 # a # b\n", "a:\n  - 1\n  - 2\nb: 3\n# f\n", "# h\na: # l\n  #h\n  - x: 1 #  l  \n  - x: 2\nb: 3 # a # b\n# f\n"),
    ("select(fi == 0) * select(fi == 1)", "a:\n  - 1\n  #h\n  - 2\nb: 3\n# f1\n# f2\n", "r:\n  s:\n    t: 1\n  #h\n    u: 2 #  l  \n  #  spaced\n  v: 3\n# a b c\nw: 4\n", "a:\n  - 1\n  #h\n  - 2\nb: 3\n# f1\n# f2\n\nr:\n  s:\n    t: 1\n    #h\n    u: 2 #  l  \n  #  spaced\n  v: 3\n# a b c\nw: 4\n"),
    (". as $i ireduce ({}; . + $i)", "#  spaced\n\n# h1\n# h2\na: 1\nc: 3\n", "a: x #  l  \nb: y # l\n# f1\n# f2\n", "a: x #  l  \nc: 3\nb: y # l\n# f1\n# f2\n"),
    ("select(fi == 0) + select(fi == 1)", "a: x #  l  \n## two\nb: y # a # b\n", "#h\nr:\n  s: #l\n    #  spaced\n    t: 9\n  z: 8\nw: 7 # l\n# f\n", "a: x #  l  \n## two\nb: y # a # b\n#h\nr:\n  s: #l\n    #  spaced\n    t: 9\n  z: 8\nw: 7 # l\n# f\n"),
    ("select(fi == 0) * select(fi == 1)", "## two\na: x\nb: y\n# f1\n# f2\n", "a: 1 # l\n# h\nb: 2\n", "## two\na: 1 # l\n# h\nb: 2\n# f1\n# f2\n"),
    ("select(fi == 1) * select(fi == 0)", "r:\n  s:\n    t: 1\n  # h\n    u: 2\n    # f\n  v: 3\nw: 4\n# f1\n# f2\n", "#  spaced\nr: # l\n  s:\n    t: 1\n    # h\n    u: 2\n  v: 3\n# a b c\nw: 4 # a # b\n", "#  spaced\nr: # l\n  s:\n    t: 1\n    # h\n    u: 2\n    # f\n  v: 3\n# a b c\nw: 4 # a # b\n# f1\n# f2\n"),
    (". as $i ireduce ({}; . + $i)", "a:\n  # a b c\n  b: 1\n# h\n  c: 2 #l\n#  spaced\nd: 3 # l\n", "## two\na:\n  - 7 #  l  \n  # f\nc: 3\n# f\n", "a:\n  - 7 #  l  \n  # f\n#  spaced\nd: 3 # l\nc: 3\n# f\n"),
    ("select(fi == 1) * select(fi == 0)", "a:\n  # h\n  - x: 1\n  # h1\n  # h2\n  - x: 2\n# h\nb: 3\n# f1\n# f2\n", "#  spaced\na: #l\n  b: 9\n  e: 5 # a # b\n# f1\n# f2\nd: 4\n# f1\n# f2\n", "#  spaced\na: #l\n  # h\n  - x: 1\n  # h1\n  # h2\n  - x: 2\n# f1\n# f2\nd: 4\n# f1\n# f2\n\n# h\nb: 3\n# f1\n# f2\n"),
    ("select(fi == 1) * select(fi == 0)", "#h\na: 1\nb: 2\n# f\n", "#h\na: 1 #l\nc: 3\n", "#h\n#h\na: 1 #l\nc: 3\nb: 2\n# f\n"),
    (". as $i ireduce ({}; . *+ $i)", "#  spaced\n\na:\n  - x: 1\n  - x: 2\nb: 3\n# f\n", "# h1\n# h2\nr:\n  s:\n  # a b c\n    t: 1 #  l  \n    u: 2\n  v: 3\nw: 4\n", "#  spaced\n\na:\n  - x: 1\n  - x: 2\nb: 3\n# f\n\n# h1\n# h2\nr:\n  s:\n    # a b c\n    t: 1 #  l  \n    u: 2\n  v: 3\nw: 4\n"),
    (". as $item ireduce ({}; . * $item )", "a: 1\n#  spaced\nb: 2\n", "#  spaced\na:\n# f1\n# f2\n  - x: 1\n  - x: 2\nb: 3 # l\n# f\n", "#  spaced\na:\n  # f1\n  # f2\n  - x: 1\n  - x: 2\n#  spaced\nb: 3 # l\n# f\n"),
    ("select(fi == 0) * select(fi == 1)", "a: #l\n  - 7\nc: 3 # l\n", "a:\n# h1\n# h2\n  b: 1\n  ## two\n  c: 2\n#  spaced\nd: 3\n", "a: #l\n  # h1\n  # h2\n  b: 1\n  ## two\n  c: 2\nc: 3 # l\n#  spaced\nd: 3\n"),
    (". as $i ireduce ({}; . + $i)", "# h\n\nr:\n  s:\n# f\n    t: 9\n  z: 8 #l\nw: 7\n", "# h\na:\n#  spaced\n  b: 9 # a # b\n  e: 5 # a # b\n# f\nd: 4\n", "r:\n  s:\n    # f\n    t: 9\n  z: 8 #l\nw: 7\n# h\na:\n  #  spaced\n  b: 9 # a # b\n  e: 5 # a # b\n# f\nd: 4\n"),
    (". as $i ireduce ({}; . + $i)", "a: 1\nc: 3\n# f1\n# f2\n", "# h1\n# h2\na: 1\n# h1\n# h2\nb: 2 #l\n", "a: 1\nc: 3\n# f1\n# f2\n\n# h1\n# h2\nb: 2 #l\n"),
    (". as $item ireduce ({}; . * $item )", "r:\n  s: #  l  \n    t: 1 # l\n    # h1\n    # h2\n    u: 2\n  v: 3\nw: 4\n", "#  spaced\na:\n# f\n  - 7\nc: 3 # l\n", "r:\n  s: #  l  \n    t: 1 # l\n    # h1\n    # h2\n    u: 2\n  v: 3\nw: 4\n#  spaced\na:\n  # f\n  - 7\nc: 3 # l\n"),
    (". as $i ireduce ({}; . *+ $i)", "# a b c\na:\n#h\n  - 1\n  # f\n  #  spaced\n  - 2\n#  spaced\nb: 3\n", "a:\n  # a b c\n  b: 9\n  e: 5\nd: 4 # a # b\n", "# a b c\na:\n  # a b c\n  b: 9\n  e: 5\n#  spaced\nb: 3\nd: 4 # a # b\n"),
    (". as $i ireduce ({}; . *+ $i)", "r:\n  s:\n    t: 9\n  #  spaced\n  z: 8 #  l  \nw: 7 # a # b\n# f1\n# f2\n", "# a b c\na: 1\n# f\nb: 2\n# f1\n# f2\n", "r:\n  s:\n    t: 9\n  #  spaced\n  z: 8 #  l  \nw: 7 # a # b\n# f1\n# f2\n\n# a b c\na: 1\n# f\nb: 2\n# f1\n# f2\n"),
    (". as $item ireduce ({}; . * $item )", "#h\na: 1\n# h\nb: 2 # a # b\n", "a: 1 #l\nb: 2\n", "#h\na: 1 #l\n# h\nb: 2 # a # b\n"),
    (". as $item ireduce ({}; . * $item )", "# h1\n# h2\n\na: # l\n# f1\n# f2\n  # h\n  - 1 # a # b\n  - 2 # a # b\n# h1\n# h2\nb: 3 #l\n", "a:\n  b: 9 #l\n  #  spaced\n  e: 5\n# h\nd: 4\n", "# h1\n# h2\n\na: # l\n  b: 9 #l\n  #  spaced\n  e: 5\n# h1\n# h2\nb: 3 #l\n# h\nd: 4\n"),
    (". as $i ireduce ({}; . + $i)", "## two\na: #l\n  - 1 #l\n  - 2\n# f1\n# f2\nb: 3 #  l  \n", "a: 1\nb: 2\n", "a: 1 #l\n# f1\n# f2\nb: 2\n"),
    (". as $item ireduce ({}; . * $item )", "a:\n  # h\n  b: 1\n  c: 2\n  # f1\n  # f2\nd: 3\n", "a:\n  ## two\n  b: 9\n  e: 5\n# f1\n# f2\nd: 4\n# f1\n# f2\n", "a:\n  ## two\n  b: 9\n  c: 2\n  # f1\n  # f2\n\n  e: 5\n# f1\n# f2\nd: 4\n# f1\n# f2\n"),
    (". as $i ireduce ({}; . + $i)", "# h\nr:\n  s: # a # b\n    # a b c\n    t: 9\n  z: 8\nw: 7\n# f\n", "a: 1\n#h\nc: 3\n# f1\n# f2\n", "r:\n  s: # a # b\n    # a b c\n    t: 9\n  z: 8\nw: 7\n# f\n\na: 1\n#h\nc: 3\n# f1\n# f2\n"),
];

#[test]
fn merging_documents_keeps_their_comments_as_yq_does_4380() -> Result<()> {
    let dir = TempDir::new()?;
    let first = dir.path().join("f0.yaml");
    let second = dir.path().join("f1.yaml");
    for &(filter, a, b, expected) in ROWS {
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

/// Three files fold left to right, each merge as yq makes it.
#[test]
fn merging_three_documents_keeps_the_comments_of_each_4380() -> Result<()> {
    let dir = TempDir::new()?;
    let texts = [
        "# base\na: 1 # one\nb:\n  c: 2 # two\n",
        "# mid\nb:\n  c: 3 # three\n  d: 4 # four\n",
        "e: 5 # five\nb:\n  d: 6\n",
    ];
    let paths: Vec<_> = texts
        .iter()
        .enumerate()
        .map(|(n, text)| -> Result<_> {
            let path = dir.path().join(format!("f{n}.yaml"));
            std::fs::write(&path, text)?;
            Ok(path)
        })
        .collect::<Result<_>>()?;
    let refs: Vec<&std::path::Path> = paths.iter().map(|p| p.as_path()).collect();
    let (stdout, stderr, status) = run_yq_files(
        ". as $item ireduce ({}; . * $item )",
        &refs,
        &["--eval-all"],
    )?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), status),
        (
            "# base\na: 1 # one\n# mid\nb:\n  c: 3 # three\n  d: 6 # four\ne: 5 # five\n",
            "",
            0
        )
    );
    Ok(())
}
