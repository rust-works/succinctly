//! #7068: an update whose target is a `//` creates the left operand as yq does when it is absent,
//! and a write in the right operand (yq's update-or-create idiom, `(.a // (.a = 0)) += 1`) is
//! applied to the document the update works on, after which the left operand is tested again.
//! Every answer was captured from yq v4.53.3.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// (input, filter, yq's stdout).
const ROWS: &[(&str, &str, &str)] = &[
    ("b: camel\n", "(.a // (.a = 0)) += 1", "b: camel\na: 1\n"),
    (
        "a: 3\nb: camel\n",
        "(.a // (.a = 0)) += 1",
        "a: 4\nb: camel\n",
    ),
    ("b: camel\n", "(.a // .z) += 1", "b: camel\na: null\nz: 1\n"),
    ("b: camel\n", "(.a // \"x\") = 1", "b: camel\na: null\n"),
    ("b: camel\n", "(.z // .b) = \"new\"", "b: new\nz: null\n"),
    ("b: camel\n", "(.b // .a) += \"!\"", "b: camel!\n"),
    (
        "b: camel\n",
        "(.a.b // .z) = 1",
        "b: camel\na:\n  b: null\nz: 1\n",
    ),
    (
        "b: camel\n",
        "(.a // .z // .y) = 1",
        "b: camel\na: null\nz: null\ny: 1\n",
    ),
    (
        "b: camel\n",
        "((.a // .z) // .y) = 1",
        "b: camel\na: null\nz: null\ny: 1\n",
    ),
    (
        "b: camel\n",
        "(.a // (.a = 0)) |= . + 1",
        "b: camel\na: 1\n",
    ),
    ("b: camel\n", "(.a // (.a |= 7)) += 1", "b: camel\na: 8\n"),
    ("b: camel\n", "(.a // .b) |= \"x\"", "b: x\na: null\n"),
    (
        "b: camel\n",
        "(.a // (.a = 0, .b = 1)) += 1",
        "b: 1\na: 1\n",
    ),
    (
        "b: camel\n",
        "(.a // (.b | select(false))) = 1",
        "b: camel\na: 1\n",
    ),
    ("a: false\n", "(.a // (.a = 0)) += 1", "a: 1\n"),
    ("a: 0\n", "(.a // .z) = 1", "a: 1\n"),
    (
        "b: camel\n",
        "with(.k; (.a // (.a = 0)) += 1)",
        "b: camel\nk:\n  a: 1\n",
    ),
    (
        "k: {b: 1}\n",
        ".k |= ((.a // (.a = 0)) += 1)",
        "k: {b: 1, a: 1}\n",
    ),
    (
        "b: camel\n",
        "(.a.b // (.a.b = 0)) += 1",
        "b: camel\na:\n  b: 1\n",
    ),
    ("b: camel\n", "((.a // (.a = 0)) += 1) | .a", "1\n"),
    ("b: camel\n", "(.a // (.a |= 3)) = 9", "b: camel\na: 9\n"),
    (
        "l: [1]\n",
        "(.l[3] // (.l[3] = 0)) += 1",
        "l: [1, null, null, 1]\n",
    ),
    (
        "b: camel\n",
        "(.a // (.a = 0) // .z) += 1",
        "b: camel\na: 1\n",
    ),
    ("a: [1]\n", "(.a // (.a = [])) += [2]", "a: [1, 2]\n"),
    (
        "a: {x: 1}\n",
        "(.a // (.a = {})) *= {\"y\": 2}",
        "a: {x: 1, y: 2}\n",
    ),
    (
        "# head\nb: camel # line\n# foot\n",
        "(.a // (.a = 0)) += 1",
        "# head\nb: camel # line\na: 1\n# foot\n",
    ),
    (
        "x: &a {k: 1}\ny: *a\n",
        "(.x.k // (.x.k = 0)) += 1",
        "x: &a {k: 2}\ny: *a\n",
    ),
    (
        "b: 1\n---\na: 2\n",
        "(.a // (.a = 0)) += 1",
        "b: 1\na: 1\n---\na: 3\n",
    ),
    // a left operand through a scalar yields nothing, which is falsy
    (
        "a: 3\nb: camel\n",
        "(.a.b // .z) = [1]",
        "a: 3\nb: camel\nz:\n  - 1\n",
    ),
    // a left operand through a scalar yields nothing, which is falsy
    (
        "a: 3\nb: camel\n",
        "(.a.b // .a // .z) *= 2",
        "a: 6\nb: camel\n",
    ),
    // a left operand through a scalar yields nothing, which is falsy
    (
        "a: false\nn: 0\n",
        "(.a.b // .m) += 1",
        "a: false\nn: 0\nm: 1\n",
    ),
];

/// The write leaves the left operand falsy, so the target is the document, and adding `1` to a
/// mapping is an error, as in yq (`!!int () cannot be added to a !!map ()`).
const ERROR_ROWS: &[(&str, &str)] = &[
    ("b: camel\n", "(.a // (.c.d = 0)) += 1"),
    ("b: camel\n", "(.a // (.a = null)) += 1"),
    ("b: camel\n", "(.a // (.z += 1)) += 1"),
];

#[test]
fn an_alternative_target_creates_and_writes_as_yq_does_7068() -> Result<()> {
    for (input, filter, expected) in ROWS {
        let (stdout, status) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, 0),
            "{filter} over {input:?}"
        );
    }
    Ok(())
}

#[test]
fn a_write_that_leaves_the_left_falsy_targets_the_document_7068() -> Result<()> {
    for (input, filter) in ERROR_ROWS {
        let (stdout, stderr, status) = run_yq_stdin_with_stderr(filter, input, &[])?;
        assert_eq!((stdout.as_str(), status), ("", 1), "{filter}");
        assert!(
            stderr.contains("cannot be added to a !!map"),
            "{filter}: {stderr}"
        );
    }
    Ok(())
}
