//! #6580: a line left over after a plain scalar that a comment ended is refused inside a block
//! container that an explicit key opens (`? - b` / `   # c` / `   x`), as it is anywhere else, and a
//! dash alone on its line in an explicit key's sequence (`? -` / `   &x` / `   x`) takes the node on
//! the lines below as its value, as an ordinary item does. The refusal was skipped whenever an
//! explicit key was pending, meant for the key's own plain scalar, so `x` was read as a further
//! item (or dropped); the dash was closed as a null item with the value read as a second one, so
//! the key held `[null, x]` where yq's holds `[x]`. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const REFUSED: &[&str] = &[
    "? - b\n   # c\n   x\n",
    "? - b\n   # c\n   x\n: v\n",
    "? - b\n   # c\n   x\nb: 1\n",
    "? - b # c\n    x\n: v\n",
    "? - &a b\n     # c\n     x\n: v\n",
    "? - a\n  - b\n    # c\n    x\n",
    "? - a\n  - b\n    # c\n    x\n: v\n",
    "? - a\n  - b # c\n    x\n: v\n",
    "? - - b\n      # c\n      x\n: v\n",
    "? - k: b\n      # c\n      x\n: v\n",
    "- ? - b\n       # c\n       x\n  : v\n",
    "a:\n  ? - b\n       # c\n       x\n  : v\n",
    "? -\n   x\n   # c\n   y\n: v\n",
    "? -  # c\n   x\n   # d\n   y\n: v\n",
    "? -\n   x\n? -\n   y\n: v\n: w\n",
];

/// The documents as YAML (`.`), where the key's first item is the node below the dash.
const WRITTEN: &[(&str, &str)] = &[
    ("? -\n   x\n: v\n", "? - x\n: v"),
    ("? -\n   x\n", "? - x\n:"),
    ("? -\n   &x\n   x\n: v\n", "? - &x x\n: v"),
    ("? -\n   !!str\n   x\n: v\n", "? - !!str x\n: v"),
    ("? -\n   &x !!str\n   x\n: v\n", "? - &x !!str x\n: v"),
    ("? -\n   &x\n   |\n     t\n: v\n", "? - &x |\n    t\n: v"),
    ("? -\n   &x\n   - y\n: v\n", "? - &x\n    - y\n: v"),
    ("? -\n   &x\n   k: y\n: v\n", "? - &x\n    k: y\n: v"),
    ("? -\n   &x\n   x\n  - z\n: v\n", "? - &x x\n  - z\n: v"),
    ("? -\n   x\n   y\n: v\n", "? - x y\n: v"),
    ("? -\n   - x\n: v\n", "? - - x\n: v"),
    ("? -\n   k: x\n   j: y\n: v\n", "? - k: x\n    j: y\n: v"),
    ("? -  # c\n   x\n: v\n", "? # c\n  - x\n: v"),
    ("? -\n   # c\n   x\n: v\n", "? # c\n  - x\n: v"),
    ("? -  # c\n   &x\n   x\n: v\n", "? # c\n  - &x x\n: v"),
    ("? -\n   x # c\n: v\n", "? - x # c\n: v"),
    ("? -\n   x\n   # c\n: v\n", "? - x\n  # c\n: v"),
    ("? - a\n  -\n    x\n: v\n", "? - a\n  - x\n: v"),
    ("? - a\n  -  # c\n    x\n: v\n", "? - a\n  # c\n  - x\n: v"),
    ("? - -\n     x\n: v\n", "? - - x\n: v"),
    ("- ? -\n     &x\n     x\n  : v\n", "- ? - &x x\n  : v"),
    ("a:\n  ? -\n     x\n  : v\n", "a:\n  ? - x\n  : v"),
    ("? -\n: v\n", "? -\n: v"),
    ("? -\n   &x\n: v\n", "? - &x\n: v"),
    ("? -\n   # c\n: v\n", "? -\n: v\n# c"),
    ("? - x\n: v\n", "? - x\n: v"),
    ("? - a\n  - b\n  # c\n: v\n", "? - a\n  - b\n  # c\n: v"),
];

#[test]
fn yq_refuses_a_line_left_after_a_comment_ended_scalar_in_an_explicit_key_6580() -> Result<()> {
    for doc in REFUSED {
        let (stdout, status) = run_yq_stdin(".", doc, &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), status), ("", 1), "{doc:?}");
    }
    Ok(())
}

#[test]
fn yq_reads_the_value_below_a_lone_dash_in_an_explicit_key_sequence_6580() -> Result<()> {
    for (doc, expected) in WRITTEN {
        let (stdout, status) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.trim_end(), status), (*expected, 0), "{doc:?}");
    }
    Ok(())
}
