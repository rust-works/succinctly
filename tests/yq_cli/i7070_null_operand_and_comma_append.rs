//! #7070 (the slices of the 2026-11-18 triage): a sequence stays the node it was when the other operand
//! of `+` is null, and a comma of paths takes an appended string as one path does.
//!
//! - `.a + null`, `.a + .absent`, `.a * null` answer the left node itself in yq, so a flow sequence (`['dog']`)
//!   or a commented block one keeps its style and comments; they printed a block sequence without the comment.
//! - `(.a, .b) += "z"` and `(.a, .b) |= . + "z"` write the string in the quote style of the first element of
//!   each sequence (`['dog', 'z']` and `["p", "z"]`); each printed a plain `z`. A single path did already (#7188).
//!
//! Each row is a live yq v4.53.3 capture (`yq FILTER`, YAML output).

use anyhow::Result;

use crate::run_yq_stdin;

const ROWS: &[(&str, &str, &str)] = &[
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", ".a + null", "['dog']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", ".a + .z", "['dog']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", ".a + .n", "['dog']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", ".b + null", "[\"p\"]\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", ".plain + null", "[x, 'y']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", ".a * null", "['dog']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", "[.a + null]", "- ['dog']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", "{\"k\": .a + null}", "k: ['dog']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", ".blk + null", "- 1 # c\n- 2\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", ".blk + .z", "- 1 # c\n- 2\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", ".m + .z", "{x: 1, y: [2]}\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", ".s + null", "hi\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", "null + .a", "['dog']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", "(.a, .b) += \"z\"", "a: ['dog', 'z']\nb: [\"p\", \"z\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", "(.a, .b) |= . + \"z\"", "a: ['dog', 'z']\nb: [\"p\", \"z\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", "(.a, .b, .plain) += \"z\"", "a: ['dog', 'z']\nb: [\"p\", \"z\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y', z]\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", "(.a, .blk) += \"z\"", "a: ['dog', 'z']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\n  - z\nplain: [x, 'y']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", "(.b, .a) += \"q\"", "a: ['dog', 'q']\nb: [\"p\", \"q\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", "(.a, .b) += [\"z\"]", "a: ['dog', z]\nb: [\"p\", z]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", "(.a, .b) += \"\"", "a: ['dog', '']\nb: [\"p\", \"\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n"),
    ("a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\n", "(.a, .plain) |= . + \"a b\"", "a: ['dog', 'a b']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y', a b]\n"),
];

#[test]
fn a_null_operand_keeps_the_sequence_and_a_comma_append_takes_each_first_style_7070() -> Result<()>
{
    for &(doc, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter} on {doc:?}");
    }
    Ok(())
}
