//! #6906: `omit([...])` keeps the presentation of what it keeps (comments, flow style, quoting,
//! block scalars, the head and foot comments of the document), as `pick` does since #5003 and as
//! yq does. It printed the rest of the document as plain block YAML. A key of the wrong kind for the
//! node (`omit(["a"])` over a sequence, `omit([0])` over a mapping) matches nothing and the node
//! comes through whole. Each row is a live yq v4.53.3 capture (default YAML output).

use anyhow::Result;

use crate::run_yq_stdin;

const D0: &str = "# head\na: 1 # one\nb:\n  - x # ex\n  - y\nc: {k: v, m: [1, 2]} # flow\nd: |\n  literal\n  text\ne: \"quoted\"\nf: 'single'\n";
const D2: &str = "- name: one # n\n  tags: [a, b]\n- name: two\n  tags: []\n# foot\n";
const D3: &str = "top:\n  mid:\n    leaf: 1 # l\n    other: |-\n      block\n  list:\n    - 1\n    - 2 # two\n\n  after: yes\n";
const D5: &str = "# c1\nx: 1\n\n# c2\ny: 2 # t\n\nz: 3\n# end\n";
const D6: &str = "- 1 # one\n- 2\n# between\n- 3\n- {a: 1}\n";

/// `(document, filter, yq's output)`.
const ROWS: &[(&str, &str, &str)] = &[
    (D0, "omit([\"a\"])", "# head\nb:\n  - x # ex\n  - y\nc: {k: v, m: [1, 2]} # flow\nd: |\n  literal\n  text\ne: \"quoted\"\nf: 'single'\n"),
    (D0, "omit([\"b\",\"c\"])", "# head\na: 1 # one\nd: |\n  literal\n  text\ne: \"quoted\"\nf: 'single'\n"),
    (D0, "omit([\"zz\"])", "# head\na: 1 # one\nb:\n  - x # ex\n  - y\nc: {k: v, m: [1, 2]} # flow\nd: |\n  literal\n  text\ne: \"quoted\"\nf: 'single'\n"),
    (D0, "omit([])", "# head\na: 1 # one\nb:\n  - x # ex\n  - y\nc: {k: v, m: [1, 2]} # flow\nd: |\n  literal\n  text\ne: \"quoted\"\nf: 'single'\n"),
    (D0, "omit([\"d\",\"e\",\"f\"])", "# head\na: 1 # one\nb:\n  - x # ex\n  - y\nc: {k: v, m: [1, 2]} # flow\n"),
    (D0, "omit([0])", "# head\na: 1 # one\nb:\n  - x # ex\n  - y\nc: {k: v, m: [1, 2]} # flow\nd: |\n  literal\n  text\ne: \"quoted\"\nf: 'single'\n"),
    (D0, "omit([\"a\"]) | .n = 1", "# head\nb:\n  - x # ex\n  - y\nc: {k: v, m: [1, 2]} # flow\nd: |\n  literal\n  text\ne: \"quoted\"\nf: 'single'\nn: 1\n"),
    (D0, "omit([\"a\",\"a\"])", "# head\nb:\n  - x # ex\n  - y\nc: {k: v, m: [1, 2]} # flow\nd: |\n  literal\n  text\ne: \"quoted\"\nf: 'single'\n"),
    (D0, "omit([\"a\",\"b\",\"c\",\"d\",\"e\",\"f\"])", "# head\n{}\n"),
    (D2, "omit([\"name\"])", "- name: one # n\n  tags: [a, b]\n- name: two\n  tags: []\n# foot\n"),
    (D2, "omit([0])", "- name: two\n  tags: []\n# foot\n"),
    (D2, "omit([1])", "- name: one # n\n  tags: [a, b]\n# foot\n"),
    (D2, "omit([0,1])", "[]\n# foot\n"),
    (D3, "omit([0])", "top:\n  mid:\n    leaf: 1 # l\n    other: |-\n      block\n  list:\n    - 1\n    - 2 # two\n  after: yes\n"),
    (D5, "omit([\"x\"])", "# c1\n# c2\ny: 2 # t\nz: 3\n# end\n"),
    (D5, "omit([\"y\"])", "# c1\nx: 1\nz: 3\n# end\n"),
    (D5, "omit([\"z\"])", "# c1\nx: 1\n# c2\ny: 2 # t\n"),
    (D5, "omit([\"x\",\"z\"])", "# c1\n# c2\ny: 2 # t\n"),
    (D6, "omit([0])", "- 2\n# between\n- 3\n- {a: 1}\n"),
    (D6, "omit([2])", "- 1 # one\n- 2\n- {a: 1}\n"),
    (D6, "omit([0,1,2,3])", "[]\n"),
    (D6, "omit([4])", "- 1 # one\n- 2\n# between\n- 3\n- {a: 1}\n"),
    (D6, "omit([-1])", "- 1 # one\n- 2\n# between\n- 3\n- {a: 1}\n"),
    (D6, "omit([\"a\"])", "- 1 # one\n- 2\n# between\n- 3\n- {a: 1}\n"),
];

#[test]
fn test_omit_keeps_the_presentation_of_what_it_keeps_6906() -> Result<()> {
    for (document, filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, document, &[])?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, 0),
            "{filter} on {document:?}"
        );
    }
    Ok(())
}

/// A document that opens with `---`, a head comment and a foot comment.
const MARKED: &str = "---\n# head\na: 1\nb: # bc\n  x: 1 # xc\n  y: 2\n# foot\n";

/// A node below the root does not carry the document's foot comment, and an `omit` of it keeps the
/// plain route's answer; the foot comment stays with the document.
const MARKED_ROWS: &[(&str, &str)] = &[
    (".b | omit([\"x\"])", "y: 2\n"),
    ("omit([\"b\"])", "---\n# head\na: 1\n"),
    (".b | omit([\"x\"]) | .z = 1", "y: 2\nz: 1\n"),
];

/// `(filter, yq's output, this tool's output)`: the head comment after `---` goes with the first
/// key when that key is omitted (`pick` does the same), where yq keeps it with the document.
const MARKED_DIVERGENT: &[(&str, &str, &str)] = &[
    (
        "omit([\"a\"])",
        "---\n# head\nb: # bc\n  x: 1 # xc\n  y: 2\n# foot\n",
        "---\nb: # bc\n  x: 1 # xc\n  y: 2\n# foot\n",
    ),
    (
        "omit([\"a\"]) | .n = 1",
        "---\n# head\nb: # bc\n  x: 1 # xc\n  y: 2\n# foot\n\nn: 1\n",
        "---\nb: # bc\n  x: 1 # xc\n  y: 2\n# foot\n\nn: 1\n",
    ),
];

#[test]
fn test_omit_below_the_root_and_after_a_document_marker_6906() -> Result<()> {
    for (filter, want) in MARKED_ROWS {
        let (stdout, code) = run_yq_stdin(filter, MARKED, &[])?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "{filter}");
    }
    for (filter, yq, ours) in MARKED_DIVERGENT {
        let (stdout, code) = run_yq_stdin(filter, MARKED, &[])?;
        assert_eq!((stdout.as_str(), code), (*ours, 0), "{filter}");
        assert_ne!(
            ours, yq,
            "{filter} now matches yq: move the row to MARKED_ROWS"
        );
    }
    Ok(())
}
