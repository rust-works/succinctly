//! #7070 (item 2, the write form): `.a += "cat"` appends an element that takes the quoting of the
//! sequence's *first* element, as yq's `toNodes` (`operator_add.go`) copies `lhs.Content[0].Style`, so
//! `a: ['dog']` becomes `a: ['dog', 'cat']` (`i7070_append_style.rs` pins the construction form). A sequence
//! on the right keeps its own spelling and an empty target has no first element. Each row was captured from
//! yq v4.53.3. A number, a boolean, a computed right side, a comma of targets and a later stage after the
//! write keep the plain spelling (limitations.md).

use anyhow::Result;

use crate::run_yq_stdin;

const INPUT: &str = "a: ['dog']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n";

/// (filter, yq's stdout over `INPUT`).
#[rustfmt::skip]
const ROWS: &[(&str, &str)] = &[
    (".a += \"cat\"", "a: ['dog', 'cat']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".c += \"q\"", "a: ['dog']\nc: [\"p\", \"q\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".a += \"1\"", "a: ['dog', '1']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".a += \"true\"", "a: ['dog', 'true']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".d += \"z\"", "a: ['dog']\nc: [\"p\"]\nd: [x, 'y', z]\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".e += \"z\"", "a: ['dog']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q, \"z\"]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".f += \"z\"", "a: ['dog']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\", z]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".g += \"z\"", "a: ['dog']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\n  - 'z'\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".h += \"z\"", "a: ['dog']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\n  - z\ni: []\nj: ['dog', \"cat\"]\n"),
    (".i += \"z\"", "a: ['dog']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni:\n  - z\nj: ['dog', \"cat\"]\n"),
    (".j += \"z\"", "a: ['dog']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\", 'z']\n"),
    (".a += null", "a: ['dog']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".d += [\"z\"]", "a: ['dog']\nc: [\"p\"]\nd: [x, 'y', z]\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".a += \"it's\"", "a: ['dog', 'it''s']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".a += \"a b\"", "a: ['dog', 'a b']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".a += .d[0]", "a: ['dog', 'x']\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".a += {\"k\":1}", "a: ['dog', {k: 1}]\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".a += [1]", "a: ['dog', 1]\nc: [\"p\"]\nd: [x, 'y']\ne: [\"p\", q]\nf: [q, \"p\"]\ng:\n  - 'x'\n  - \"y\"\nh:\n  - plain\n  - 'x'\ni: []\nj: ['dog', \"cat\"]\n"),
    (".j += \"z\" | .j[2]", "z\n"),
];

#[test]
fn plus_assign_of_a_scalar_copies_the_first_element_quoting_7070() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, INPUT, &[])?;
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{filter}");
    }
    Ok(())
}
