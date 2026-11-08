//! #6674: `with_entries(select(...))` and `map(select(...))` keep each surviving item as it was
//! written. The filter was outside the replay of presentation-preserving filters, so the result
//! came from the plain route and every item lost its spelling (`a:` as an empty value, `~`, `True`),
//! its quoting and its comment: `with_entries(select(.key != "zz"))` turned `a:` into `a: null`.
//! A document's head comment is also kept across `with_entries` and `to_entries | from_entries`
//! (yq keeps it on the first key, so the written-out `to_entries | map(select(...)) | from_entries`
//! drops it with that entry where `with_entries` does not). Every row was captured from yq v4.53.3
//! and is asserted against it.
//!
//! What still differs (a sequence's `with_entries`, whose keys are integers, and an anchor a
//! filter drops while an alias to it stays) is in `docs/compliance/yq/limitations.md`.

use anyhow::Result;

use crate::run_yq_stdin;

/// `(document, filter, yq's output)`.
const ROWS: &[(&str, &str, &str)] = &[
    (
        "a:\nb: 1\n",
        "with_entries(select(.key != \"zz\"))",
        "a:\nb: 1\n",
    ),
    ("a:\nb: 1\n", "with_entries(select(.value != 1))", "a:\n"),
    (
        "a:\nb: 1\n",
        "with_entries(select(.value != null))",
        "b: 1\n",
    ),
    (
        "a:\nb: 1\n",
        "to_entries | map(select(.value != 1)) | from_entries",
        "a:\n",
    ),
    ("a:\nb: 1\n", "with_entries(.)", "a:\nb: 1\n"),
    (
        "a: ~\nb: 1\nc: True\n",
        "with_entries(select(.key != \"zz\"))",
        "a: ~\nb: 1\nc: True\n",
    ),
    (
        "a: ~\nb: 1\nc: True\n",
        "with_entries(select(.value != 1))",
        "a: ~\nc: True\n",
    ),
    (
        "a: ~\nb: 1\nc: True\n",
        "with_entries(select(.value != null))",
        "b: 1\nc: True\n",
    ),
    (
        "a: ~\nb: 1\nc: True\n",
        "to_entries | map(select(.value != 1)) | from_entries",
        "a: ~\nc: True\n",
    ),
    (
        "a: ~\nb: 1\nc: True\n",
        "with_entries(.)",
        "a: ~\nb: 1\nc: True\n",
    ),
    (
        "a: 1 # c\nb: 2\n",
        "with_entries(select(.key != \"zz\"))",
        "a: 1 # c\nb: 2\n",
    ),
    (
        "a: 1 # c\nb: 2\n",
        "with_entries(select(.value != 1))",
        "b: 2\n",
    ),
    (
        "a: 1 # c\nb: 2\n",
        "with_entries(select(.value != null))",
        "a: 1 # c\nb: 2\n",
    ),
    (
        "a: 1 # c\nb: 2\n",
        "to_entries | map(select(.value != 1)) | from_entries",
        "b: 2\n",
    ),
    ("a: 1 # c\nb: 2\n", "with_entries(.)", "a: 1 # c\nb: 2\n"),
    (
        "# head\na: 1\nb: 2 # t\n# foot\n",
        "with_entries(select(.key != \"zz\"))",
        "# head\na: 1\nb: 2 # t\n# foot\n",
    ),
    (
        "# head\na: 1\nb: 2 # t\n# foot\n",
        "with_entries(select(.value != 1))",
        "# head\nb: 2 # t\n# foot\n",
    ),
    (
        "# head\na: 1\nb: 2 # t\n# foot\n",
        "with_entries(select(.value != null))",
        "# head\na: 1\nb: 2 # t\n# foot\n",
    ),
    (
        "# head\na: 1\nb: 2 # t\n# foot\n",
        "to_entries | map(select(.value != 1)) | from_entries",
        "b: 2 # t\n# foot\n",
    ),
    (
        "# head\na: 1\nb: 2 # t\n# foot\n",
        "with_entries(.)",
        "# head\na: 1\nb: 2 # t\n# foot\n",
    ),
    (
        "a: \"x\"\nb: 'y'\nc: 3\n",
        "with_entries(select(.key != \"zz\"))",
        "a: \"x\"\nb: 'y'\nc: 3\n",
    ),
    (
        "a: \"x\"\nb: 'y'\nc: 3\n",
        "with_entries(select(.value != 1))",
        "a: \"x\"\nb: 'y'\nc: 3\n",
    ),
    (
        "a: \"x\"\nb: 'y'\nc: 3\n",
        "with_entries(select(.value != null))",
        "a: \"x\"\nb: 'y'\nc: 3\n",
    ),
    (
        "a: \"x\"\nb: 'y'\nc: 3\n",
        "to_entries | map(select(.value != 1)) | from_entries",
        "a: \"x\"\nb: 'y'\nc: 3\n",
    ),
    (
        "a: \"x\"\nb: 'y'\nc: 3\n",
        "with_entries(.)",
        "a: \"x\"\nb: 'y'\nc: 3\n",
    ),
    (
        "a:\n  x:\n  y: ~\nb: 2\n",
        "with_entries(select(.key != \"zz\"))",
        "a:\n  x:\n  y: ~\nb: 2\n",
    ),
    (
        "a:\n  x:\n  y: ~\nb: 2\n",
        "with_entries(select(.value != 1))",
        "a:\n  x:\n  y: ~\nb: 2\n",
    ),
    (
        "a:\n  x:\n  y: ~\nb: 2\n",
        "with_entries(select(.value != null))",
        "a:\n  x:\n  y: ~\nb: 2\n",
    ),
    (
        "a:\n  x:\n  y: ~\nb: 2\n",
        "to_entries | map(select(.value != 1)) | from_entries",
        "a:\n  x:\n  y: ~\nb: 2\n",
    ),
    (
        "a:\n  x:\n  y: ~\nb: 2\n",
        "with_entries(.)",
        "a:\n  x:\n  y: ~\nb: 2\n",
    ),
    (
        "a: null\nb: Null\nc: NULL\nd: yes\n",
        "with_entries(select(.key != \"zz\"))",
        "a: null\nb: Null\nc: NULL\nd: yes\n",
    ),
    (
        "a: null\nb: Null\nc: NULL\nd: yes\n",
        "with_entries(select(.value != 1))",
        "a: null\nb: Null\nc: NULL\nd: yes\n",
    ),
    (
        "a: null\nb: Null\nc: NULL\nd: yes\n",
        "with_entries(select(.value != null))",
        "d: yes\n",
    ),
    (
        "a: null\nb: Null\nc: NULL\nd: yes\n",
        "to_entries | map(select(.value != 1)) | from_entries",
        "a: null\nb: Null\nc: NULL\nd: yes\n",
    ),
    (
        "a: null\nb: Null\nc: NULL\nd: yes\n",
        "with_entries(.)",
        "a: null\nb: Null\nc: NULL\nd: yes\n",
    ),
    (
        "- a\n- ~\n- \n- True # c\n- 5\n",
        "map(select(. != 5))",
        "- a\n- ~\n-\n- True # c\n",
    ),
    (
        "- a\n- ~\n- \n- True # c\n- 5\n",
        "map(select(. != null))",
        "- a\n- True # c\n- 5\n",
    ),
    (
        "- a: 1\n  b:\n- a: 2\n  b: ~\n",
        "map(select(.a == 2))",
        "- a: 2\n  b: ~\n",
    ),
    (
        "- 1\n- 0x10\n- '3'\n- \"4\"\n- 5 # five\n",
        "map(select(. != 1))",
        "- 0x10\n- '3'\n- \"4\"\n- 5 # five\n",
    ),
    (
        "# head\na: 1\n",
        "to_entries | map(.) | from_entries",
        "a: 1\n",
    ),
    (
        "# head\na: 1\n",
        "to_entries | from_entries",
        "# head\na: 1\n",
    ),
    (
        "# head\na: 1\nb: 2\n",
        "to_entries | map(select(.value != 1)) | from_entries",
        "b: 2\n",
    ),
    (
        "# head\na: 1\nb: 2\n",
        "with_entries(select(.value != 1))",
        "# head\nb: 2\n",
    ),
    (
        "a: {c: 1}\nb: 2\n",
        "with_entries(select(.key == \"a\"))",
        "a: {c: 1}\n",
    ),
    (
        "a:\nb: 1\nc: ~ # k\nd: 'q'\n",
        "with_entries(select(.key == \"a\" or .key == \"c\"))",
        "a:\nc: ~ # k\n",
    ),
    (
        "a:\nb: 1\nc: ~ # k\nd: 'q'\n",
        "with_entries(select(.key != \"a\" and .value != 1))",
        "c: ~ # k\nd: 'q'\n",
    ),
    (
        "a:\nb: 1\nc: ~ # k\nd: 'q'\n",
        "with_entries(select((.key == \"a\") | not))",
        "b: 1\nc: ~ # k\nd: 'q'\n",
    ),
    (
        "a:\nb: 1\nc: ~ # k\nd: 'q'\n",
        "with_entries((select(.key != \"b\")))",
        "a:\nc: ~ # k\nd: 'q'\n",
    ),
    (
        "a:\nb: 1\nc: ~ # k\nd: 'q'\n",
        "with_entries(select(.key | . != \"b\"))",
        "a:\nc: ~ # k\nd: 'q'\n",
    ),
    (
        "a:\nb: 1\nc: ~ # k\nd: 'q'\n",
        "with_entries(select((.value != 1)))",
        "a:\nc: ~ # k\nd: 'q'\n",
    ),
];

#[test]
fn select_keeps_each_surviving_item_as_written_6674() -> Result<()> {
    for (doc, filter, expected) in ROWS {
        let (out, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(
            (out.as_str(), code),
            (*expected, 0),
            "{filter} over {doc:?}"
        );
    }
    Ok(())
}
