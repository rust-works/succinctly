//! #6230: under `-P`, which clears every style, a mapping key that holds a line break is written by
//! yq as `? |-` with the text as a literal block and the `:` on a line of its own, from a plain,
//! single-quoted or double-quoted source key alike, and so is one over 128 bytes. `succinctly yq`
//! wrote `? "k\nz"`: the key a quoted source leaves after `-P` is a known string with no style, which
//! the explicit-key branch did not take for plain. Every row is a live yq v4.53.3 capture; the two
//! rows without `-P` keep the quoting the source wrote.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, extra arguments, expected output).
const ROWS: &[(&str, &str, &[&str], &str)] = &[
    ("? 'k\n\nz'\n: 1\n", ".", &["-P"], "? |-\n  k\n  z\n: 1\n"),
    ("? \"k\\nz\"\n: 1\n", ".", &["-P"], "? |-\n  k\n  z\n: 1\n"),
    ("? 'k\n\nz'\n: 1\nb: 2\n", ".", &["-P"], "? |-\n  k\n  z\n: 1\nb: 2\n"),
    ("? 'k\n\n\nz'\n: 1\n", ".", &["-P"], "? |-\n  k\n\n  z\n: 1\n"),
    ("a:\n  ? 'k\n\nz'\n  : 1\n", ".", &["-P"], "a:\n  ? |-\n    k\n    z\n  : 1\n"),
    ("? 'k\n\nz'\n: {b: 'q\n\n r'}\n", ".", &["-P"], "? |-\n  k\n  z\n: b: |-\n    q\n    r\n"),
    ("? 'k\n\nz'\n: 1\n", ".", &["-P", "-I4"], "? |-\n    k\n    z\n: 1\n"),
    ("- ? 'k\n\nz'\n  : 1\n", ".", &["-P"], "- ? |-\n    k\n    z\n  : 1\n"),
    ("? 'k\n\nz'\n: 1\n", ".zz = 1", &["-P"], "? |-\n  k\n  z\n: 1\nzz: 1\n"),
    ("? \"k\\nz\"\n: [1, 2]\n", ".", &["-P"], "? |-\n  k\n  z\n: - 1\n  - 2\n"),
    ("? 'kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk\n\nz'\n: 1\n", ".", &["-P"], "? |-\n  kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk\n  z\n: 1\n"),
    ("? \"k\\nz\\n\"\n: 1\n", ".", &["-P"], "? |\n  k\n  z\n: 1\n"),
    ("\"1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111\": 1\n", ".", &["-P"], "? \"1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111\"\n: 1\n"),
    ("'1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111': 1\n", ".", &["-P"], "? \"1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111\"\n: 1\n"),
    ("? \"k\\u2028z\"\n: 1\n", ".", &["-P"], "? 'k   z'\n: 1\n"),
    ("? \"k\\rz\"\n: 1\n", ".", &["-P"], "? \"k\\rz\"\n: 1\n"),
    ("? 'k\n\nz'\n: 1\n", ".", &[], "? 'k\n\n  z'\n: 1\n"),
    ("? \"k\\nz\"\n: 1\n", ".", &[], "? \"k\\nz\"\n: 1\n"),
    ("? k\n  z\n: 1\n", ".", &["-P"], "k z: 1\n"),
];

#[test]
fn a_p_key_with_a_line_break_is_a_literal_block_6230() -> Result<()> {
    for (doc, filter, args, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, doc, args)?;
        assert_eq!(code, 0, "{doc:?} {filter}");
        assert_eq!(&stdout, want, "{doc:?} {filter} {args:?}");
    }
    Ok(())
}
