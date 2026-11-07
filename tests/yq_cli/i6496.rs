//! #6496: a mapping key that is a collection (`? - p`, `? a: 1`, `? {a: 1}`) is written as the key node it is,
//! after `? ` with its lines one indent step in, and the value after it on the `:` line, as yq writes it. It
//! printed as `"": v`, the text of the key (empty), though the index held it whole. Compact collections inside the
//! key lost every item after the first of the one they opened (`? - - p` / `    - q` printed `? - - p`), which was
//! the parser closing the key's first item behind a container it had opened. An empty `[]` or `{}` key is a
//! simple key, `[]: v`. Every output was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_collection_key_is_written_as_the_key_node_6496() -> Result<()> {
    let cases: &[(&str, &[&str], &str)] = &[
        ("? - p\n: v\n", &[], "? - p\n: v\n"),
        ("? - p\n: v\n", &["-I=4"], "?   - p\n: v\n"),
        ("?\n- p\n", &[], "? - p\n:\n"),
        ("?\n- p\n", &["-I=4"], "?   - p\n:\n"),
        ("? - p\n  - q\n: v\n", &[], "? - p\n  - q\n: v\n"),
        ("? - p\n  - q\n: v\n", &["-I=4"], "?   - p\n    - q\n: v\n"),
        ("? - p\n: v\nw: 1\n", &[], "? - p\n: v\nw: 1\n"),
        ("? - p\n: v\nw: 1\n", &["-I=4"], "?   - p\n: v\nw: 1\n"),
        ("? a: 1\n: v\n", &[], "? a: 1\n: v\n"),
        ("? a: 1\n: v\n", &["-I=4"], "?   a: 1\n: v\n"),
        ("? a: 1\n  b: 2\n: v\n", &[], "? a: 1\n  b: 2\n: v\n"),
        (
            "? a: 1\n  b: 2\n: v\n",
            &["-I=4"],
            "?   a: 1\n    b: 2\n: v\n",
        ),
        ("? {a: 1}\n: v\n", &[], "? {a: 1}\n: v\n"),
        ("? {a: 1}\n: v\n", &["-I=4"], "? {a: 1}\n: v\n"),
        ("? [p, q]\n: v\n", &[], "? [p, q]\n: v\n"),
        ("? [p, q]\n: v\n", &["-I=4"], "? [p, q]\n: v\n"),
        ("? []\n: v\n", &[], "[]: v\n"),
        ("? []\n: v\n", &["-I=4"], "[]: v\n"),
        ("? {}\n: v\n", &[], "{}: v\n"),
        ("? {}\n: v\n", &["-I=4"], "{}: v\n"),
        ("x:\n  ? - p\n  : v\n", &[], "x:\n  ? - p\n  : v\n"),
        (
            "x:\n  ? - p\n  : v\n",
            &["-I=4"],
            "x:\n    ?   - p\n    : v\n",
        ),
        ("- ? - p\n  : v\n", &[], "- ? - p\n  : v\n"),
        ("- ? - p\n  : v\n", &["-I=4"], "- ? - p\n  : v\n"),
        ("? - p\n:\n", &[], "? - p\n:\n"),
        ("? - p\n:\n", &["-I=4"], "?   - p\n:\n"),
        ("? - p\n: - q\n", &[], "? - p\n: - q\n"),
        ("? - p\n: - q\n", &["-I=4"], "?   - p\n:   - q\n"),
        ("? - p\n: a: 1\n", &[], "? - p\n: a: 1\n"),
        ("? - p\n: a: 1\n", &["-I=4"], "?   - p\n:   a: 1\n"),
        ("? - p\n: v\n? - r\n: w\n", &[], "? - p\n: v\n? - r\n: w\n"),
        (
            "? - p\n: v\n? - r\n: w\n",
            &["-I=4"],
            "?   - p\n: v\n?   - r\n: w\n",
        ),
        (
            "? - a: 1\n    b: 2\n: v\n",
            &[],
            "? - a: 1\n    b: 2\n: v\n",
        ),
        (
            "? - a: 1\n    b: 2\n: v\n",
            &["-I=4"],
            "?   - a: 1\n      b: 2\n: v\n",
        ),
        (
            "? - - p\n    - q\n  - r\n: v\n",
            &[],
            "? - - p\n    - q\n  - r\n: v\n",
        ),
        (
            "? - - p\n    - q\n  - r\n: v\n",
            &["-I=4"],
            "?   - - p\n      - q\n    - r\n: v\n",
        ),
        ("k: 1\n? - p\n: v\nz: 2\n", &[], "k: 1\n? - p\n: v\nz: 2\n"),
        (
            "k: 1\n? - p\n: v\nz: 2\n",
            &["-I=4"],
            "k: 1\n?   - p\n: v\nz: 2\n",
        ),
        (
            "a:\n  b:\n    ? - p\n      - q\n    : v\n    c: 1\n",
            &[],
            "a:\n  b:\n    ? - p\n      - q\n    : v\n    c: 1\n",
        ),
        (
            "a:\n  b:\n    ? - p\n      - q\n    : v\n    c: 1\n",
            &["-I=4"],
            "a:\n    b:\n        ?   - p\n            - q\n        : v\n        c: 1\n",
        ),
        (
            "? - a: 1\n  - b: 2\n: v\n",
            &[],
            "? - a: 1\n  - b: 2\n: v\n",
        ),
        (
            "? - a: 1\n  - b: 2\n: v\n",
            &["-I=4"],
            "?   - a: 1\n    - b: 2\n: v\n",
        ),
        (
            "? - - a\n    - b\n  - - c\n    - d\n: v\n",
            &[],
            "? - - a\n    - b\n  - - c\n    - d\n: v\n",
        ),
        (
            "? - - a\n    - b\n  - - c\n    - d\n: v\n",
            &["-I=4"],
            "?   - - a\n      - b\n    - - c\n      - d\n: v\n",
        ),
        (
            "? - a: 1\n    b: 2\n  - c: 3\n    d: 4\n: v\n",
            &[],
            "? - a: 1\n    b: 2\n  - c: 3\n    d: 4\n: v\n",
        ),
        (
            "? - a: 1\n    b: 2\n  - c: 3\n    d: 4\n: v\n",
            &["-I=4"],
            "?   - a: 1\n      b: 2\n    - c: 3\n      d: 4\n: v\n",
        ),
        (
            "- k: 1\n  ? - p\n    - q\n  : v\n",
            &[],
            "- k: 1\n  ? - p\n    - q\n  : v\n",
        ),
        (
            "- k: 1\n  ? - p\n    - q\n  : v\n",
            &["-I=4"],
            "- k: 1\n  ? - p\n    - q\n  : v\n",
        ),
    ];
    for &(input, args, expected) in cases {
        let (stdout, status) = run_yq_stdin(".", input, args)?;
        assert_eq!(
            (stdout.as_str(), status),
            (expected, 0),
            "{input:?} {args:?}"
        );
    }
    Ok(())
}
