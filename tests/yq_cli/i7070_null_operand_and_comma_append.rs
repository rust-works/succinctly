//! #7070 (the slices of the 2026-11-18 triage): a sequence stays the node it was when the other operand
//! of `+` is null, and a comma of paths takes an appended string as one path does.
//!
//! - `.a + null`, `.a + .absent`, `.a * null` answer the left node itself in yq, so a flow sequence (`['dog']`)
//!   or a commented block one keeps its style and comments; they printed a block sequence without the comment.
//!   A null on the right of a nested `*` (`.o * {"k": null}`) and of the flagged merges leaves the left value too.
//! - `(.a, .b) += "z"` and `(.a, .b) |= . + "z"` write the string in the quote style of the first element of
//!   each sequence (`['dog', 'z']` and `["p", "z"]`); each printed a plain `z`. A single path did already (#7188).
//!   Overlapping branches, a branch that is no sequence or is absent, and the other writes of a pipe agree too.
//!
//! Each row is a live yq v4.53.3 capture (`yq [FLAGS] FILTER`, YAML output) over [`DOC`].

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "a: ['dog']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\n";

/// `(flags, filter, yq's output)`.
const ROWS: &[(&[&str], &str, &str)] = &[
    (&[], ".a + null", "['dog']\n"),
    (&[], ".a + .z", "['dog']\n"),
    (&[], ".a + .n", "['dog']\n"),
    (&[], ".b + null", "[\"p\"]\n"),
    (&[], ".plain + null", "[x, 'y']\n"),
    (&[], ".a * null", "['dog']\n"),
    (&[], "[.a + null]", "- ['dog']\n"),
    (&[], "{\"k\": .a + null}", "k: ['dog']\n"),
    (&[], ".blk + null", "- 1 # c\n- 2\n"),
    (&[], ".blk + .z", "- 1 # c\n- 2\n"),
    (&[], ".m + .z", "{x: 1, y: [2]}\n"),
    (&[], ".s + null", "hi\n"),
    (&[], "null + .a", "['dog']\n"),
    (&[], ".o * {\"k\": null}", "{k: null, j: {z: 1}}\n"),
    (&[], ".o *n {\"k\": null}", "{k: [1], j: {z: 1}}\n"),
    (&[], ".o *+ {\"k\": null}", "{k: null, j: {z: 1}}\n"),
    (&[], ".o *d {\"k\": null}", "{k: null, j: {z: 1}}\n"),
    (&[], ".o * {\"j\": null}", "{k: [1], j: null}\n"),
    (&[], ".m * null", "{x: 1, y: [2]}\n"),
    (&[], ".blk * null", "- 1 # c\n- 2\n"),
    (&[], ".blk + .n", "- 1 # c\n- 2\n"),
    (&[], "(.a, .b) += \"z\"", "a: ['dog', 'z']\nb: [\"p\", \"z\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.a, .b) |= . + \"z\"", "a: ['dog', 'z']\nb: [\"p\", \"z\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.a, .b, .plain) += \"z\"", "a: ['dog', 'z']\nb: [\"p\", \"z\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y', z]\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.a, .blk) += \"z\"", "a: ['dog', 'z']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\n  - z\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.b, .a) += \"q\"", "a: ['dog', 'q']\nb: [\"p\", \"q\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.a, .b) += [\"z\"]", "a: ['dog', z]\nb: [\"p\", z]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.a, .b) += \"\"", "a: ['dog', '']\nb: [\"p\", \"\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.a, .plain) |= . + \"a b\"", "a: ['dog', 'a b']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y', a b]\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.a, .a) += \"z\"", "a: ['dog', 'z', 'z']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.a, .a[0]) |= . + \"z\"", "a: ['dogz', 'z']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.a, .b) += \"z\" | del(.a[0])", "a: ['z']\nb: [\"p\", \"z\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.a, .b) += \"z\" | .a += \"q\"", "a: ['dog', 'z', 'q']\nb: [\"p\", \"z\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.a, .s) += \"z\"", "a: ['dog', 'z']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hiz\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\n"),
    (&[], "(.a, .absent) += \"z\"", "a: ['dog', 'z']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\nabsent: z\n"),
    (&[], "(.absent, .a) += \"z\"", "a: ['dog', 'z']\nb: [\"p\"]\nm: {x: 1, y: [2]}\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain: [x, 'y']\no: {k: [1], j: {z: 1}}\nabsent: z\n"),
    (&["-P"], ".a + null", "- dog\n"),
    (&["-P"], "(.a, .b) += \"z\"", "a:\n  - dog\n  - z\nb:\n  - p\n  - z\nm:\n  x: 1\n  y:\n    - 2\ns: hi\nn: ~ # note\nblk:\n  - 1 # c\n  - 2\nplain:\n  - x\n  - 'y'\no:\n  k:\n    - 1\n  j:\n    z: 1\n"),
];

#[test]
fn a_null_operand_keeps_the_sequence_and_a_comma_append_takes_each_first_style_7070() -> Result<()>
{
    for &(flags, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, DOC, flags)?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter} {flags:?}");
    }
    Ok(())
}
