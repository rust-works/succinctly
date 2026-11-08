//! #6593: a comment line between a dash alone on its line and the container below it, in the first
//! item of an explicit key's sequence, heads that container's first node and is written on the
//! dash line (`? - # c`), as it is for an ordinary item. It was written before the dash: the
//! lookahead that decides whether the dash has a value records the comment lines as pending, and
//! the routine that places them then counted them as lines above the dash, so they headed the
//! container itself. A scalar value below the comment was not affected (`? # c` / `  - x`).
//!
//! The first group is the issue's own table, captured from yq v4.53.3. The second group was
//! captured from yq v4.52.2, the one installed where this was written; each row's document is
//! the shape of a first-group row moved to another comment column, a second comment line, or a
//! nested position.

use anyhow::Result;

use crate::run_yq_stdin;

const PINNED: &[(&str, &str)] = &[
    ("? -\n# c0\n   - y\n: v\n", "? - # c0\n    - y\n: v"),
    ("? -\n  # c\n   k: y\n: v\n", "? - # c\n    k: y\n: v"),
    ("? -\n   # c\n   - y\n: v\n", "? - # c\n    - y\n: v"),
    // Controls from the issue: a scalar below the comment, and an ordinary item.
    ("? -\n   # c\n   x\n: v\n", "? # c\n  - x\n: v"),
    ("-\n# c0\n  - y\n", "- # c0\n  - y"),
];

const NEARBY: &[(&str, &str)] = &[
    (
        "? -\n# a\n  # b\n    - y\n: v\n",
        "? - # a\n    # b\n    - y\n: v",
    ),
    (
        "? -\n      # c\n   k:\n     - y\n: v\n",
        "? - # c\n    k:\n      - y\n: v",
    ),
    ("? -\n   # c\n   - - y\n: v\n", "? - # c\n    - - y\n: v"),
    (
        "- ? -\n    # c\n    - y\n  : v\n",
        "- ? -\n    # c\n    - y\n  : v",
    ),
    (
        "- ? -\n# c\n      k: y\n  : v\n",
        "- ? - # c\n      k: y\n  : v",
    ),
    ("k:\n  -\n    # c\n    - y\n", "k:\n  - # c\n    - y"),
];

#[test]
fn a_comment_below_a_lone_dash_in_a_key_sequence_heads_the_first_node_6593() -> Result<()> {
    for (doc, expected) in PINNED.iter().chain(NEARBY) {
        let (stdout, status) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.trim_end(), status), (*expected, 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn the_comment_is_the_head_of_the_first_node_not_of_the_container_6593() -> Result<()> {
    // yq: the key is `[[y]]` and `c0` heads `.[0][0]`'s `y`, not the nested sequence `.[0]`.
    let doc = "? -\n# c0\n   - y\n: v\n";
    let (stdout, status) = run_yq_stdin(
        "(keys | .[0]) | [(.[0] | head_comment), (.[0][0] | head_comment)]",
        doc,
        &["-o=json", "-I0"],
    )?;
    assert_eq!((stdout.trim_end(), status), (r#"["","c0"]"#, 0));
    Ok(())
}
