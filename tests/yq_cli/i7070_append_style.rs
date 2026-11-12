//! #7070 (item 2): a scalar appended to a sequence with `+` takes the quote style of the
//! sequence's first element, as yq copies that node's `Style` onto the new one: `a: ['x']` and
//! `.a + "z"` is `['x', 'z']`, `[x, 'y']` is `[x, 'y', z]`, a block sequence keeps its block
//! form, an array on the right (`+ ["z"]`) brings its own elements as they are. Before, the
//! construction lost the sequence's flow style and every quote (`k:` / `- x` / `- z`). Every row is
//! `yq FILTER` on the document, captured from yq v4.53.3. A write (`.a += "z"`) and a number
//! (`+ 5` is `'5'` in yq) are recorded in `docs/compliance/yq/limitations.md`.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, yq's output), each with exit status 0.
const ROWS: &[(&str, &str, &str)] = &[
    ("a: ['x', y]\n", ".a + \"z\"", "['x', y, 'z']\n"),
    ("a: ['x', y]\n", "[.a + \"z\"]", "- ['x', y, 'z']\n"),
    (
        "a: ['x', y]\n",
        "{\"k\": (.a + \"z\")}",
        "k: ['x', y, 'z']\n",
    ),
    ("a: ['x', y]\n", ".a + [\"z\"]", "['x', y, z]\n"),
    ("a: ['x', y]\n", ".a + \"a b\"", "['x', y, 'a b']\n"),
    ("a: [x, 'y']\n", ".a + \"z\"", "[x, 'y', z]\n"),
    ("a: [x, 'y']\n", "[.a + \"z\"]", "- [x, 'y', z]\n"),
    ("a: [x, 'y']\n", "{\"k\": (.a + \"z\")}", "k: [x, 'y', z]\n"),
    ("a: [x, 'y']\n", ".a + [\"z\"]", "[x, 'y', z]\n"),
    ("a: [x, 'y']\n", ".a + \"a b\"", "[x, 'y', a b]\n"),
    ("a: [\"x\", 'y']\n", ".a + \"z\"", "[\"x\", 'y', \"z\"]\n"),
    (
        "a: [\"x\", 'y']\n",
        "[.a + \"z\"]",
        "- [\"x\", 'y', \"z\"]\n",
    ),
    (
        "a: [\"x\", 'y']\n",
        "{\"k\": (.a + \"z\")}",
        "k: [\"x\", 'y', \"z\"]\n",
    ),
    ("a: [\"x\", 'y']\n", ".a + [\"z\"]", "[\"x\", 'y', z]\n"),
    (
        "a: [\"x\", 'y']\n",
        ".a + \"a b\"",
        "[\"x\", 'y', \"a b\"]\n",
    ),
    ("a: ['x', \"y\"]\n", ".a + \"z\"", "['x', \"y\", 'z']\n"),
    ("a: ['x', \"y\"]\n", "[.a + \"z\"]", "- ['x', \"y\", 'z']\n"),
    (
        "a: ['x', \"y\"]\n",
        "{\"k\": (.a + \"z\")}",
        "k: ['x', \"y\", 'z']\n",
    ),
    ("a: ['x', \"y\"]\n", ".a + [\"z\"]", "['x', \"y\", z]\n"),
    ("a: ['x', \"y\"]\n", ".a + \"a b\"", "['x', \"y\", 'a b']\n"),
    ("a: [x, y]\n", ".a + \"z\"", "[x, y, z]\n"),
    ("a: [x, y]\n", "[.a + \"z\"]", "- [x, y, z]\n"),
    ("a: [x, y]\n", "{\"k\": (.a + \"z\")}", "k: [x, y, z]\n"),
    ("a: [x, y]\n", ".a + [\"z\"]", "[x, y, z]\n"),
    ("a: [x, y]\n", ".a + \"a b\"", "[x, y, a b]\n"),
    ("a: []\n", ".a + \"z\"", "- z\n"),
    ("a: []\n", "[.a + \"z\"]", "- - z\n"),
    ("a: []\n", "{\"k\": (.a + \"z\")}", "k:\n  - z\n"),
    ("a: []\n", ".a + null", "[]\n"),
    ("a: []\n", ".a + \"a b\"", "- a b\n"),
    (
        "a:\n  - 'x'\n  - 'y'\n",
        ".a + \"z\"",
        "- 'x'\n- 'y'\n- 'z'\n",
    ),
    (
        "a:\n  - 'x'\n  - 'y'\n",
        "[.a + \"z\"]",
        "- - 'x'\n  - 'y'\n  - 'z'\n",
    ),
    (
        "a:\n  - 'x'\n  - 'y'\n",
        "{\"k\": (.a + \"z\")}",
        "k:\n  - 'x'\n  - 'y'\n  - 'z'\n",
    ),
    (
        "a:\n  - 'x'\n  - 'y'\n",
        ".a + [\"z\"]",
        "- 'x'\n- 'y'\n- z\n",
    ),
    (
        "a:\n  - 'x'\n  - 'y'\n",
        ".a + \"a b\"",
        "- 'x'\n- 'y'\n- 'a b'\n",
    ),
    ("a:\n  - \"x\"\n", ".a + \"z\"", "- \"x\"\n- \"z\"\n"),
    ("a:\n  - \"x\"\n", "[.a + \"z\"]", "- - \"x\"\n  - \"z\"\n"),
    (
        "a:\n  - \"x\"\n",
        "{\"k\": (.a + \"z\")}",
        "k:\n  - \"x\"\n  - \"z\"\n",
    ),
    ("a:\n  - \"x\"\n", ".a + [\"z\"]", "- \"x\"\n- z\n"),
    ("a:\n  - \"x\"\n", ".a + \"a b\"", "- \"x\"\n- \"a b\"\n"),
    ("a:\n  - x\n  - 'y'\n", ".a + \"z\"", "- x\n- 'y'\n- z\n"),
    (
        "a:\n  - x\n  - 'y'\n",
        "[.a + \"z\"]",
        "- - x\n  - 'y'\n  - z\n",
    ),
    (
        "a:\n  - x\n  - 'y'\n",
        "{\"k\": (.a + \"z\")}",
        "k:\n  - x\n  - 'y'\n  - z\n",
    ),
    ("a:\n  - x\n  - 'y'\n", ".a + [\"z\"]", "- x\n- 'y'\n- z\n"),
    (
        "a:\n  - x\n  - 'y'\n",
        ".a + \"a b\"",
        "- x\n- 'y'\n- a b\n",
    ),
    ("a: [1]\n", ".a + \"z\"", "[1, z]\n"),
    ("a: [1]\n", "[.a + \"z\"]", "- [1, z]\n"),
    ("a: [1]\n", "{\"k\": (.a + \"z\")}", "k: [1, z]\n"),
    ("a: [1]\n", ".a + [\"z\"]", "[1, z]\n"),
    ("a: [1]\n", ".a + \"a b\"", "[1, a b]\n"),
    ("a: ['x']\n", ".a + \"z\"", "['x', 'z']\n"),
    ("a: ['x']\n", "[.a + \"z\"]", "- ['x', 'z']\n"),
    ("a: ['x']\n", "{\"k\": (.a + \"z\")}", "k: ['x', 'z']\n"),
    ("a: ['x']\n", ".a + [\"z\"]", "['x', z]\n"),
    ("a: ['x']\n", ".a + \"a b\"", "['x', 'a b']\n"),
    (
        "a: ['x', # c\n  y]\n",
        ".a + \"z\"",
        "['x', # c\n  y, 'z']\n",
    ),
    (
        "a: ['x', # c\n  y]\n",
        "[.a + \"z\"]",
        "- ['x', # c\n  y, 'z']\n",
    ),
    (
        "a: ['x', # c\n  y]\n",
        "{\"k\": (.a + \"z\")}",
        "k: ['x', # c\n  y, 'z']\n",
    ),
    (
        "a: ['x', # c\n  y]\n",
        ".a + [\"z\"]",
        "['x', # c\n  y, z]\n",
    ),
    (
        "a: ['x', # c\n  y]\n",
        ".a + \"a b\"",
        "['x', # c\n  y, 'a b']\n",
    ),
    (
        "a: ['x']\nb: &anc w # note\n",
        ".a + .b",
        "['x', &anc 'w', # note\n]\n",
    ),
    (
        "a: ['x']\nb: &anc w # note\n",
        "[.a + .b]",
        "- ['x', &anc 'w', # note\n]\n",
    ),
    (
        "a: ['x']\nb: &anc w # note\n",
        "{\"k\": (.a + .b)}",
        "k: ['x', &anc 'w', # note\n]\n",
    ),
    ("a: ['x']\nb: !!str w\n", ".a + .b", "['x', 'w']\n"),
    ("a: ['x']\nb: !!str w\n", "[.a + .b]", "- ['x', 'w']\n"),
    (
        "a: ['x']\nb: !!str w\n",
        "{\"k\": (.a + .b)}",
        "k: ['x', 'w']\n",
    ),
    ("a: ['x']\nb: 'q'\n", ".a + .b", "['x', 'q']\n"),
    ("a: ['x']\nb: 'q'\n", "[.a + .b]", "- ['x', 'q']\n"),
    (
        "a: ['x']\nb: 'q'\n",
        "{\"k\": (.a + .b)}",
        "k: ['x', 'q']\n",
    ),
    ("a: ['x']\nb: \"q\"\n", ".a + .b", "['x', 'q']\n"),
    ("a: ['x']\nb: \"q\"\n", "[.a + .b]", "- ['x', 'q']\n"),
    (
        "a: ['x']\nb: \"q\"\n",
        "{\"k\": (.a + .b)}",
        "k: ['x', 'q']\n",
    ),
    (
        "a: ['x']\nb: |\n  l1\n  l2\n",
        ".a + .b",
        "['x', 'l1\n\n    l2\n\n']\n",
    ),
    (
        "a: ['x']\nb: |\n  l1\n  l2\n",
        "[.a + .b]",
        "- ['x', 'l1\n\n    l2\n\n']\n",
    ),
    (
        "a: ['x']\nb: |\n  l1\n  l2\n",
        "{\"k\": (.a + .b)}",
        "k: ['x', 'l1\n\n    l2\n\n']\n",
    ),
    ("a: ['x']\nb: # c\n  w\n", ".a + .b", "['x', 'w']\n"),
    ("a: ['x']\nb: # c\n  w\n", "[.a + .b]", "- ['x', 'w']\n"),
    (
        "a: ['x']\nb: # c\n  w\n",
        "{\"k\": (.a + .b)}",
        "k: ['x', 'w']\n",
    ),
    (
        "a: ['x']\nb: &anc !!str w # n\n",
        ".a + .b",
        "['x', &anc 'w', # n\n]\n",
    ),
    (
        "a: ['x']\nb: &anc !!str w # n\n",
        "[.a + .b]",
        "- ['x', &anc 'w', # n\n]\n",
    ),
    (
        "a: ['x']\nb: &anc !!str w # n\n",
        "{\"k\": (.a + .b)}",
        "k: ['x', &anc 'w', # n\n]\n",
    ),
    (
        "a: [x]\nb: &anc w # note\n",
        ".a + .b",
        "[x, &anc w, # note\n]\n",
    ),
    (
        "a: [x]\nb: &anc w # note\n",
        "[.a + .b]",
        "- [x, &anc w, # note\n]\n",
    ),
    (
        "a: [x]\nb: &anc w # note\n",
        "{\"k\": (.a + .b)}",
        "k: [x, &anc w, # note\n]\n",
    ),
    ("a: [x]\nb: !!str w\n", ".a + .b", "[x, w]\n"),
    ("a: [x]\nb: !!str w\n", "[.a + .b]", "- [x, w]\n"),
    ("a: [x]\nb: !!str w\n", "{\"k\": (.a + .b)}", "k: [x, w]\n"),
    ("a: [x]\nb: 'q'\n", ".a + .b", "[x, q]\n"),
    ("a: [x]\nb: 'q'\n", "[.a + .b]", "- [x, q]\n"),
    ("a: [x]\nb: 'q'\n", "{\"k\": (.a + .b)}", "k: [x, q]\n"),
    ("a: [x]\nb: \"q\"\n", ".a + .b", "[x, q]\n"),
    ("a: [x]\nb: \"q\"\n", "[.a + .b]", "- [x, q]\n"),
    ("a: [x]\nb: \"q\"\n", "{\"k\": (.a + .b)}", "k: [x, q]\n"),
    (
        "a: [x]\nb: |\n  l1\n  l2\n",
        ".a + .b",
        "[x, \"l1\\nl2\\n\"]\n",
    ),
    (
        "a: [x]\nb: |\n  l1\n  l2\n",
        "[.a + .b]",
        "- [x, \"l1\\nl2\\n\"]\n",
    ),
    (
        "a: [x]\nb: |\n  l1\n  l2\n",
        "{\"k\": (.a + .b)}",
        "k: [x, \"l1\\nl2\\n\"]\n",
    ),
    ("a: [x]\nb: # c\n  w\n", ".a + .b", "[x, w]\n"),
    ("a: [x]\nb: # c\n  w\n", "[.a + .b]", "- [x, w]\n"),
    ("a: [x]\nb: # c\n  w\n", "{\"k\": (.a + .b)}", "k: [x, w]\n"),
    (
        "a: [x]\nb: &anc !!str w # n\n",
        ".a + .b",
        "[x, &anc w, # n\n]\n",
    ),
    (
        "a: [x]\nb: &anc !!str w # n\n",
        "[.a + .b]",
        "- [x, &anc w, # n\n]\n",
    ),
    (
        "a: [x]\nb: &anc !!str w # n\n",
        "{\"k\": (.a + .b)}",
        "k: [x, &anc w, # n\n]\n",
    ),
    (
        "a: [\"x\"]\nb: &anc w # note\n",
        ".a + .b",
        "[\"x\", &anc \"w\", # note\n]\n",
    ),
    (
        "a: [\"x\"]\nb: &anc w # note\n",
        "[.a + .b]",
        "- [\"x\", &anc \"w\", # note\n]\n",
    ),
    (
        "a: [\"x\"]\nb: &anc w # note\n",
        "{\"k\": (.a + .b)}",
        "k: [\"x\", &anc \"w\", # note\n]\n",
    ),
    ("a: [\"x\"]\nb: !!str w\n", ".a + .b", "[\"x\", \"w\"]\n"),
    (
        "a: [\"x\"]\nb: !!str w\n",
        "[.a + .b]",
        "- [\"x\", \"w\"]\n",
    ),
    (
        "a: [\"x\"]\nb: !!str w\n",
        "{\"k\": (.a + .b)}",
        "k: [\"x\", \"w\"]\n",
    ),
    ("a: [\"x\"]\nb: 'q'\n", ".a + .b", "[\"x\", \"q\"]\n"),
    ("a: [\"x\"]\nb: 'q'\n", "[.a + .b]", "- [\"x\", \"q\"]\n"),
    (
        "a: [\"x\"]\nb: 'q'\n",
        "{\"k\": (.a + .b)}",
        "k: [\"x\", \"q\"]\n",
    ),
    ("a: [\"x\"]\nb: \"q\"\n", ".a + .b", "[\"x\", \"q\"]\n"),
    ("a: [\"x\"]\nb: \"q\"\n", "[.a + .b]", "- [\"x\", \"q\"]\n"),
    (
        "a: [\"x\"]\nb: \"q\"\n",
        "{\"k\": (.a + .b)}",
        "k: [\"x\", \"q\"]\n",
    ),
    (
        "a: [\"x\"]\nb: |\n  l1\n  l2\n",
        ".a + .b",
        "[\"x\", \"l1\\nl2\\n\"]\n",
    ),
    (
        "a: [\"x\"]\nb: |\n  l1\n  l2\n",
        "[.a + .b]",
        "- [\"x\", \"l1\\nl2\\n\"]\n",
    ),
    (
        "a: [\"x\"]\nb: |\n  l1\n  l2\n",
        "{\"k\": (.a + .b)}",
        "k: [\"x\", \"l1\\nl2\\n\"]\n",
    ),
    ("a: [\"x\"]\nb: # c\n  w\n", ".a + .b", "[\"x\", \"w\"]\n"),
    (
        "a: [\"x\"]\nb: # c\n  w\n",
        "[.a + .b]",
        "- [\"x\", \"w\"]\n",
    ),
    (
        "a: [\"x\"]\nb: # c\n  w\n",
        "{\"k\": (.a + .b)}",
        "k: [\"x\", \"w\"]\n",
    ),
    (
        "a: [\"x\"]\nb: &anc !!str w # n\n",
        ".a + .b",
        "[\"x\", &anc \"w\", # n\n]\n",
    ),
    (
        "a: [\"x\"]\nb: &anc !!str w # n\n",
        "[.a + .b]",
        "- [\"x\", &anc \"w\", # n\n]\n",
    ),
    (
        "a: [\"x\"]\nb: &anc !!str w # n\n",
        "{\"k\": (.a + .b)}",
        "k: [\"x\", &anc \"w\", # n\n]\n",
    ),
    (
        "a: [[1], x]\nb: &anc w # note\n",
        ".a + .b",
        "[[1], x, &anc w, # note\n]\n",
    ),
    (
        "a: [[1], x]\nb: &anc w # note\n",
        "[.a + .b]",
        "- [[1], x, &anc w, # note\n]\n",
    ),
    (
        "a: [[1], x]\nb: &anc w # note\n",
        "{\"k\": (.a + .b)}",
        "k: [[1], x, &anc w, # note\n]\n",
    ),
    ("a: [[1], x]\nb: !!str w\n", ".a + .b", "[[1], x, w]\n"),
    ("a: [[1], x]\nb: !!str w\n", "[.a + .b]", "- [[1], x, w]\n"),
    (
        "a: [[1], x]\nb: !!str w\n",
        "{\"k\": (.a + .b)}",
        "k: [[1], x, w]\n",
    ),
    ("a: [[1], x]\nb: 'q'\n", ".a + .b", "[[1], x, q]\n"),
    ("a: [[1], x]\nb: 'q'\n", "[.a + .b]", "- [[1], x, q]\n"),
    (
        "a: [[1], x]\nb: 'q'\n",
        "{\"k\": (.a + .b)}",
        "k: [[1], x, q]\n",
    ),
    ("a: [[1], x]\nb: \"q\"\n", ".a + .b", "[[1], x, q]\n"),
    ("a: [[1], x]\nb: \"q\"\n", "[.a + .b]", "- [[1], x, q]\n"),
    (
        "a: [[1], x]\nb: \"q\"\n",
        "{\"k\": (.a + .b)}",
        "k: [[1], x, q]\n",
    ),
    (
        "a: [[1], x]\nb: |\n  l1\n  l2\n",
        ".a + .b",
        "[[1], x, \"l1\\nl2\\n\"]\n",
    ),
    (
        "a: [[1], x]\nb: |\n  l1\n  l2\n",
        "[.a + .b]",
        "- [[1], x, \"l1\\nl2\\n\"]\n",
    ),
    (
        "a: [[1], x]\nb: |\n  l1\n  l2\n",
        "{\"k\": (.a + .b)}",
        "k: [[1], x, \"l1\\nl2\\n\"]\n",
    ),
    ("a: [[1], x]\nb: # c\n  w\n", ".a + .b", "[[1], x, w]\n"),
    ("a: [[1], x]\nb: # c\n  w\n", "[.a + .b]", "- [[1], x, w]\n"),
    (
        "a: [[1], x]\nb: # c\n  w\n",
        "{\"k\": (.a + .b)}",
        "k: [[1], x, w]\n",
    ),
    (
        "a: [[1], x]\nb: &anc !!str w # n\n",
        ".a + .b",
        "[[1], x, &anc w, # n\n]\n",
    ),
    (
        "a: [[1], x]\nb: &anc !!str w # n\n",
        "[.a + .b]",
        "- [[1], x, &anc w, # n\n]\n",
    ),
    (
        "a: [[1], x]\nb: &anc !!str w # n\n",
        "{\"k\": (.a + .b)}",
        "k: [[1], x, &anc w, # n\n]\n",
    ),
    (
        "a:\n  - 'x'\nb: &anc w # note\n",
        ".a + .b",
        "- 'x'\n- &anc 'w' # note\n",
    ),
    (
        "a:\n  - 'x'\nb: &anc w # note\n",
        "[.a + .b]",
        "- - 'x'\n  - &anc 'w' # note\n",
    ),
    (
        "a:\n  - 'x'\nb: &anc w # note\n",
        "{\"k\": (.a + .b)}",
        "k:\n  - 'x'\n  - &anc 'w' # note\n",
    ),
    ("a:\n  - 'x'\nb: !!str w\n", ".a + .b", "- 'x'\n- 'w'\n"),
    (
        "a:\n  - 'x'\nb: !!str w\n",
        "[.a + .b]",
        "- - 'x'\n  - 'w'\n",
    ),
    (
        "a:\n  - 'x'\nb: !!str w\n",
        "{\"k\": (.a + .b)}",
        "k:\n  - 'x'\n  - 'w'\n",
    ),
    ("a:\n  - 'x'\nb: 'q'\n", ".a + .b", "- 'x'\n- 'q'\n"),
    ("a:\n  - 'x'\nb: 'q'\n", "[.a + .b]", "- - 'x'\n  - 'q'\n"),
    (
        "a:\n  - 'x'\nb: 'q'\n",
        "{\"k\": (.a + .b)}",
        "k:\n  - 'x'\n  - 'q'\n",
    ),
    ("a:\n  - 'x'\nb: \"q\"\n", ".a + .b", "- 'x'\n- 'q'\n"),
    ("a:\n  - 'x'\nb: \"q\"\n", "[.a + .b]", "- - 'x'\n  - 'q'\n"),
    (
        "a:\n  - 'x'\nb: \"q\"\n",
        "{\"k\": (.a + .b)}",
        "k:\n  - 'x'\n  - 'q'\n",
    ),
    (
        "a:\n  - 'x'\nb: |\n  l1\n  l2\n",
        ".a + .b",
        "- 'x'\n- 'l1\n\n  l2\n\n'\n",
    ),
    (
        "a:\n  - 'x'\nb: |\n  l1\n  l2\n",
        "[.a + .b]",
        "- - 'x'\n  - 'l1\n\n    l2\n\n'\n",
    ),
    (
        "a:\n  - 'x'\nb: |\n  l1\n  l2\n",
        "{\"k\": (.a + .b)}",
        "k:\n  - 'x'\n  - 'l1\n\n    l2\n\n'\n",
    ),
    ("a:\n  - 'x'\nb: # c\n  w\n", ".a + .b", "- 'x'\n- 'w'\n"),
    (
        "a:\n  - 'x'\nb: # c\n  w\n",
        "[.a + .b]",
        "- - 'x'\n  - 'w'\n",
    ),
    (
        "a:\n  - 'x'\nb: # c\n  w\n",
        "{\"k\": (.a + .b)}",
        "k:\n  - 'x'\n  - 'w'\n",
    ),
    (
        "a:\n  - 'x'\nb: &anc !!str w # n\n",
        ".a + .b",
        "- 'x'\n- &anc 'w' # n\n",
    ),
    (
        "a:\n  - 'x'\nb: &anc !!str w # n\n",
        "[.a + .b]",
        "- - 'x'\n  - &anc 'w' # n\n",
    ),
    (
        "a:\n  - 'x'\nb: &anc !!str w # n\n",
        "{\"k\": (.a + .b)}",
        "k:\n  - 'x'\n  - &anc 'w' # n\n",
    ),
];

#[test]
fn a_scalar_appended_to_a_sequence_takes_the_first_elements_quote_style_7070() -> Result<()> {
    for (document, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, document, &[])?;
        assert_eq!((out.as_str(), code), (*want, 0), "{document:?} | {filter}");
    }
    Ok(())
}
