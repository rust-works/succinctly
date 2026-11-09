//! #6738: an explicit key whose value, or whose key, is a block collection inside a compact
//! sequence entry stands at the column its continuation lines stand at, and goes on a line of its
//! own when there is no room for a blank before it.
//!
//! The first entry was padded by a fixed `-I - 1` blanks after the `:`, where the continuation lines
//! step by `deeper_yaml_indent`: the two differ once the entry sits inside a `- ` item, and the
//! output was not YAML any reader takes (`:   - 1` over `    - 2`). At `-I=3` the key and the value
//! have no room for a blank and yq starts them on the next line. Every answer was captured from yq
//! v4.53.3; the output is read back by succinctly as well.

use anyhow::Result;

use crate::run_yq_stdin;

fn check(rows: &[(&str, &str, &str)]) -> Result<()> {
    for &(input, indent, want) in rows {
        let (stdout, code) = run_yq_stdin(".", input, &[indent])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{indent} {input:?}");
        // What it printed is YAML: it reads back as the same document.
        let (again, code) = run_yq_stdin(".", &stdout, &[indent])?;
        assert_eq!(
            (again.as_str(), code),
            (want, 0),
            "read back {indent} {input:?}"
        );
    }
    Ok(())
}

#[test]
fn test_yq_aligns_an_explicit_keys_block_value_and_key_with_its_continuation_6738() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        ("- ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=3", "- ? [a, b]\n  :\n   - 1\n   - 2\n"),
        ("- ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=4", "- ? [a, b]\n  : - 1\n    - 2\n"),
        ("- ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=5", "- ? [a, b]\n  :  - 1\n     - 2\n"),
        ("- ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=6", "- ? [a, b]\n  :   - 1\n      - 2\n"),
        ("- ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=8", "- ? [a, b]\n  :     - 1\n        - 2\n"),
        ("- ? [a, b]\n  :\n    x: 1\n    y: 2\n", "-I=3", "- ? [a, b]\n  :\n   x: 1\n   y: 2\n"),
        ("- ? [a, b]\n  :\n    x: 1\n    y: 2\n", "-I=4", "- ? [a, b]\n  : x: 1\n    y: 2\n"),
        ("- ? [a, b]\n  :\n    x: 1\n    y: 2\n", "-I=5", "- ? [a, b]\n  :  x: 1\n     y: 2\n"),
        ("- ? [a, b]\n  :\n    x: 1\n    y: 2\n", "-I=6", "- ? [a, b]\n  :   x: 1\n      y: 2\n"),
        ("- ? [a, b]\n  :\n    x: 1\n    y: 2\n", "-I=8", "- ? [a, b]\n  :     x: 1\n        y: 2\n"),
        ("- - ? [a, b]\n    :\n      - 1\n      - 2\n", "-I=3", "- - ? [a, b]\n    : - 1\n      - 2\n"),
        ("- - ? [a, b]\n    :\n      - 1\n      - 2\n", "-I=5", "- - ? [a, b]\n    :\n     - 1\n     - 2\n"),
        ("- - ? [a, b]\n    :\n      - 1\n      - 2\n", "-I=6", "- - ? [a, b]\n    : - 1\n      - 2\n"),
        ("- - ? [a, b]\n    :\n      - 1\n      - 2\n", "-I=8", "- - ? [a, b]\n    :   - 1\n        - 2\n"),
        ("a:\n  - ? [a, b]\n    :\n      - 1\n", "-I=3", "a:\n   - ? [a, b]\n     :\n      - 1\n"),
        ("a:\n  - ? [a, b]\n    :\n      - 1\n", "-I=4", "a:\n    - ? [a, b]\n      : - 1\n"),
        ("a:\n  - ? [a, b]\n    :\n      - 1\n", "-I=5", "a:\n     - ? [a, b]\n       :  - 1\n"),
        ("a:\n  - ? [a, b]\n    :\n      - 1\n", "-I=6", "a:\n      - ? [a, b]\n        :   - 1\n"),
        ("a:\n  - ? [a, b]\n    :\n      - 1\n", "-I=8", "a:\n        - ? [a, b]\n          :     - 1\n"),
        ("- ? - a\n    - b\n  : 1\n", "-I=3", "- ?\n   - a\n   - b\n  : 1\n"),
        ("- ? x: 1\n    y: 2\n  : 1\n", "-I=3", "- ?\n   x: 1\n   y: 2\n  : 1\n"),
        ("- ? - a\n    - b\n  : - 1\n    - 2\n", "-I=3", "- ?\n   - a\n   - b\n  :\n   - 1\n   - 2\n"),
        ("- ? - a\n    - b\n  : - 1\n    - 2\n", "-I=4", "- ? - a\n    - b\n  : - 1\n    - 2\n"),
        ("- ? - a\n    - b\n  : - 1\n    - 2\n", "-I=5", "- ?  - a\n     - b\n  :  - 1\n     - 2\n"),
        ("- ? - a\n    - b\n  : - 1\n    - 2\n", "-I=6", "- ?   - a\n      - b\n  :   - 1\n      - 2\n"),
        ("- ? - a\n    - b\n  : - 1\n    - 2\n", "-I=8", "- ?     - a\n        - b\n  :     - 1\n        - 2\n"),
        ("- ? [a, b]\n  :\n    - 1\n- ? [c]\n  :\n    k: v\n", "-I=3", "- ? [a, b]\n  :\n   - 1\n- ? [c]\n  :\n   k: v\n"),
        ("- ? [a, b]\n  :\n    - 1\n- ? [c]\n  :\n    k: v\n", "-I=4", "- ? [a, b]\n  : - 1\n- ? [c]\n  : k: v\n"),
        ("- ? [a, b]\n  :\n    - 1\n- ? [c]\n  :\n    k: v\n", "-I=5", "- ? [a, b]\n  :  - 1\n- ? [c]\n  :  k: v\n"),
        ("- ? [a, b]\n  :\n    - 1\n- ? [c]\n  :\n    k: v\n", "-I=6", "- ? [a, b]\n  :   - 1\n- ? [c]\n  :   k: v\n"),
        ("- ? [a, b]\n  :\n    - 1\n- ? [c]\n  :\n    k: v\n", "-I=8", "- ? [a, b]\n  :     - 1\n- ? [c]\n  :     k: v\n"),
        ("- ? [a, b]\n  :\n    - ? [c]\n      :\n        - 2\n", "-I=3", "- ? [a, b]\n  :\n   - ? [c]\n     :\n      - 2\n"),
        ("- ? [a, b]\n  :\n    - ? [c]\n      :\n        - 2\n", "-I=4", "- ? [a, b]\n  : - ? [c]\n      : - 2\n"),
        ("- ? [a, b]\n  :\n    - ? [c]\n      :\n        - 2\n", "-I=5", "- ? [a, b]\n  :  - ? [c]\n       :  - 2\n"),
        ("- ? [a, b]\n  :\n    - ? [c]\n      :\n        - 2\n", "-I=6", "- ? [a, b]\n  :   - ? [c]\n        :   - 2\n"),
        ("- ? [a, b]\n  :\n    - ? [c]\n      :\n        - 2\n", "-I=8", "- ? [a, b]\n  :     - ? [c]\n          :     - 2\n"),
    ];
    check(&rows)
}

/// Rows that matched yq before this change, and must go on matching.
#[test]
fn test_yq_keeps_the_explicit_key_rows_that_were_aligned_6738() -> Result<()> {
    #[rustfmt::skip]
    let rows = [
        ("? [a, b]\n:\n  - 1\n  - 2\n", "-I=1", "? [a, b]\n: - 1\n  - 2\n"),
        ("? [a, b]\n:\n  - 1\n  - 2\n", "-I=2", "? [a, b]\n: - 1\n  - 2\n"),
        ("? [a, b]\n:\n  - 1\n  - 2\n", "-I=3", "? [a, b]\n:  - 1\n   - 2\n"),
        ("? [a, b]\n:\n  - 1\n  - 2\n", "-I=4", "? [a, b]\n:   - 1\n    - 2\n"),
        ("? [a, b]\n:\n  - 1\n  - 2\n", "-I=5", "? [a, b]\n:    - 1\n     - 2\n"),
        ("? [a, b]\n:\n  - 1\n  - 2\n", "-I=6", "? [a, b]\n:     - 1\n      - 2\n"),
        ("? [a, b]\n:\n  - 1\n  - 2\n", "-I=8", "? [a, b]\n:       - 1\n        - 2\n"),
        ("? [a, b]\n:\n  x: 1\n  y: 2\n", "-I=1", "? [a, b]\n: x: 1\n  y: 2\n"),
        ("? [a, b]\n:\n  x: 1\n  y: 2\n", "-I=2", "? [a, b]\n: x: 1\n  y: 2\n"),
        ("? [a, b]\n:\n  x: 1\n  y: 2\n", "-I=3", "? [a, b]\n:  x: 1\n   y: 2\n"),
        ("? [a, b]\n:\n  x: 1\n  y: 2\n", "-I=4", "? [a, b]\n:   x: 1\n    y: 2\n"),
        ("? [a, b]\n:\n  x: 1\n  y: 2\n", "-I=5", "? [a, b]\n:    x: 1\n     y: 2\n"),
        ("? [a, b]\n:\n  x: 1\n  y: 2\n", "-I=6", "? [a, b]\n:     x: 1\n      y: 2\n"),
        ("? [a, b]\n:\n  x: 1\n  y: 2\n", "-I=8", "? [a, b]\n:       x: 1\n        y: 2\n"),
        ("? [a, b]\n: v\n", "-I=1", "? [a, b]\n: v\n"),
        ("? [a, b]\n: v\n", "-I=2", "? [a, b]\n: v\n"),
        ("? [a, b]\n: v\n", "-I=3", "? [a, b]\n: v\n"),
        ("? [a, b]\n: v\n", "-I=4", "? [a, b]\n: v\n"),
        ("? [a, b]\n: v\n", "-I=5", "? [a, b]\n: v\n"),
        ("? [a, b]\n: v\n", "-I=6", "? [a, b]\n: v\n"),
        ("? [a, b]\n: v\n", "-I=8", "? [a, b]\n: v\n"),
        ("- ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=1", "- ? [a, b]\n  : - 1\n    - 2\n"),
        ("- ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=2", "- ? [a, b]\n  : - 1\n    - 2\n"),
        ("- ? [a, b]\n  :\n    x: 1\n    y: 2\n", "-I=1", "- ? [a, b]\n  : x: 1\n    y: 2\n"),
        ("- ? [a, b]\n  :\n    x: 1\n    y: 2\n", "-I=2", "- ? [a, b]\n  : x: 1\n    y: 2\n"),
        ("- ? [a, b]\n  : v\n", "-I=1", "- ? [a, b]\n  : v\n"),
        ("- ? [a, b]\n  : v\n", "-I=2", "- ? [a, b]\n  : v\n"),
        ("- ? [a, b]\n  : v\n", "-I=3", "- ? [a, b]\n  : v\n"),
        ("- ? [a, b]\n  : v\n", "-I=4", "- ? [a, b]\n  : v\n"),
        ("- ? [a, b]\n  : v\n", "-I=5", "- ? [a, b]\n  : v\n"),
        ("- ? [a, b]\n  : v\n", "-I=6", "- ? [a, b]\n  : v\n"),
        ("- ? [a, b]\n  : v\n", "-I=8", "- ? [a, b]\n  : v\n"),
        ("- - ? [a, b]\n    :\n      - 1\n      - 2\n", "-I=1", "- - ? [a, b]\n    : - 1\n      - 2\n"),
        ("- - ? [a, b]\n    :\n      - 1\n      - 2\n", "-I=2", "- - ? [a, b]\n    : - 1\n      - 2\n"),
        ("- - ? [a, b]\n    :\n      - 1\n      - 2\n", "-I=4", "- - ? [a, b]\n    :   - 1\n        - 2\n"),
        ("a:\n  ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=1", "a:\n  ? [a, b]\n  : - 1\n    - 2\n"),
        ("a:\n  ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=2", "a:\n  ? [a, b]\n  : - 1\n    - 2\n"),
        ("a:\n  ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=3", "a:\n   ? [a, b]\n   :  - 1\n      - 2\n"),
        ("a:\n  ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=4", "a:\n    ? [a, b]\n    :   - 1\n        - 2\n"),
        ("a:\n  ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=5", "a:\n     ? [a, b]\n     :    - 1\n          - 2\n"),
        ("a:\n  ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=6", "a:\n      ? [a, b]\n      :     - 1\n            - 2\n"),
        ("a:\n  ? [a, b]\n  :\n    - 1\n    - 2\n", "-I=8", "a:\n        ? [a, b]\n        :       - 1\n                - 2\n"),
        ("a:\n  b:\n    ? [a, b]\n    :\n      x: 1\n", "-I=1", "a:\n  b:\n    ? [a, b]\n    : x: 1\n"),
        ("a:\n  b:\n    ? [a, b]\n    :\n      x: 1\n", "-I=2", "a:\n  b:\n    ? [a, b]\n    : x: 1\n"),
        ("a:\n  b:\n    ? [a, b]\n    :\n      x: 1\n", "-I=3", "a:\n   b:\n      ? [a, b]\n      :  x: 1\n"),
        ("a:\n  b:\n    ? [a, b]\n    :\n      x: 1\n", "-I=4", "a:\n    b:\n        ? [a, b]\n        :   x: 1\n"),
        ("a:\n  b:\n    ? [a, b]\n    :\n      x: 1\n", "-I=5", "a:\n     b:\n          ? [a, b]\n          :    x: 1\n"),
        ("a:\n  b:\n    ? [a, b]\n    :\n      x: 1\n", "-I=6", "a:\n      b:\n            ? [a, b]\n            :     x: 1\n"),
        ("a:\n  b:\n    ? [a, b]\n    :\n      x: 1\n", "-I=8", "a:\n        b:\n                ? [a, b]\n                :       x: 1\n"),
        ("a:\n  - ? [a, b]\n    :\n      - 1\n", "-I=1", "a:\n  - ? [a, b]\n    : - 1\n"),
        ("a:\n  - ? [a, b]\n    :\n      - 1\n", "-I=2", "a:\n  - ? [a, b]\n    : - 1\n"),
        ("? - a\n  - b\n: 1\n", "-I=1", "? - a\n  - b\n: 1\n"),
        ("? - a\n  - b\n: 1\n", "-I=2", "? - a\n  - b\n: 1\n"),
        ("? - a\n  - b\n: 1\n", "-I=3", "?  - a\n   - b\n: 1\n"),
        ("? - a\n  - b\n: 1\n", "-I=4", "?   - a\n    - b\n: 1\n"),
        ("? - a\n  - b\n: 1\n", "-I=5", "?    - a\n     - b\n: 1\n"),
        ("? - a\n  - b\n: 1\n", "-I=6", "?     - a\n      - b\n: 1\n"),
        ("? - a\n  - b\n: 1\n", "-I=8", "?       - a\n        - b\n: 1\n"),
        ("- ? - a\n    - b\n  : 1\n", "-I=1", "- ? - a\n    - b\n  : 1\n"),
        ("- ? - a\n    - b\n  : 1\n", "-I=2", "- ? - a\n    - b\n  : 1\n"),
        ("- ? - a\n    - b\n  : 1\n", "-I=4", "- ? - a\n    - b\n  : 1\n"),
        ("- ? - a\n    - b\n  : 1\n", "-I=5", "- ?  - a\n     - b\n  : 1\n"),
        ("- ? - a\n    - b\n  : 1\n", "-I=6", "- ?   - a\n      - b\n  : 1\n"),
        ("- ? - a\n    - b\n  : 1\n", "-I=8", "- ?     - a\n        - b\n  : 1\n"),
        ("? x: 1\n  y: 2\n: 1\n", "-I=1", "? x: 1\n  y: 2\n: 1\n"),
        ("? x: 1\n  y: 2\n: 1\n", "-I=2", "? x: 1\n  y: 2\n: 1\n"),
        ("? x: 1\n  y: 2\n: 1\n", "-I=3", "?  x: 1\n   y: 2\n: 1\n"),
        ("? x: 1\n  y: 2\n: 1\n", "-I=4", "?   x: 1\n    y: 2\n: 1\n"),
        ("? x: 1\n  y: 2\n: 1\n", "-I=5", "?    x: 1\n     y: 2\n: 1\n"),
        ("? x: 1\n  y: 2\n: 1\n", "-I=6", "?     x: 1\n      y: 2\n: 1\n"),
        ("? x: 1\n  y: 2\n: 1\n", "-I=8", "?       x: 1\n        y: 2\n: 1\n"),
        ("- ? x: 1\n    y: 2\n  : 1\n", "-I=1", "- ? x: 1\n    y: 2\n  : 1\n"),
        ("- ? x: 1\n    y: 2\n  : 1\n", "-I=2", "- ? x: 1\n    y: 2\n  : 1\n"),
        ("- ? x: 1\n    y: 2\n  : 1\n", "-I=4", "- ? x: 1\n    y: 2\n  : 1\n"),
        ("- ? x: 1\n    y: 2\n  : 1\n", "-I=5", "- ?  x: 1\n     y: 2\n  : 1\n"),
        ("- ? x: 1\n    y: 2\n  : 1\n", "-I=6", "- ?   x: 1\n      y: 2\n  : 1\n"),
        ("- ? x: 1\n    y: 2\n  : 1\n", "-I=8", "- ?     x: 1\n        y: 2\n  : 1\n"),
        ("- ? - a\n    - b\n  : - 1\n    - 2\n", "-I=1", "- ? - a\n    - b\n  : - 1\n    - 2\n"),
        ("- ? - a\n    - b\n  : - 1\n    - 2\n", "-I=2", "- ? - a\n    - b\n  : - 1\n    - 2\n"),
        ("? - a\n  - b\n: - 1\n  - 2\n", "-I=1", "? - a\n  - b\n: - 1\n  - 2\n"),
        ("? - a\n  - b\n: - 1\n  - 2\n", "-I=2", "? - a\n  - b\n: - 1\n  - 2\n"),
        ("? - a\n  - b\n: - 1\n  - 2\n", "-I=3", "?  - a\n   - b\n:  - 1\n   - 2\n"),
        ("? - a\n  - b\n: - 1\n  - 2\n", "-I=4", "?   - a\n    - b\n:   - 1\n    - 2\n"),
        ("? - a\n  - b\n: - 1\n  - 2\n", "-I=5", "?    - a\n     - b\n:    - 1\n     - 2\n"),
        ("? - a\n  - b\n: - 1\n  - 2\n", "-I=6", "?     - a\n      - b\n:     - 1\n      - 2\n"),
        ("? - a\n  - b\n: - 1\n  - 2\n", "-I=8", "?       - a\n        - b\n:       - 1\n        - 2\n"),
        ("- ? [a, b]\n  :\n    - 1\n- ? [c]\n  :\n    k: v\n", "-I=1", "- ? [a, b]\n  : - 1\n- ? [c]\n  : k: v\n"),
        ("- ? [a, b]\n  :\n    - 1\n- ? [c]\n  :\n    k: v\n", "-I=2", "- ? [a, b]\n  : - 1\n- ? [c]\n  : k: v\n"),
        ("- ? [a, b]\n  :\n    - ? [c]\n      :\n        - 2\n", "-I=1", "- ? [a, b]\n  : - ? [c]\n      : - 2\n"),
        ("- ? [a, b]\n  :\n    - ? [c]\n      :\n        - 2\n", "-I=2", "- ? [a, b]\n  : - ? [c]\n      : - 2\n"),
        ("- ? k\n  :\n    ? [a]\n    :\n      - 1\n", "-I=1", "- k:\n    ? [a]\n    : - 1\n"),
        ("- ? k\n  :\n    ? [a]\n    :\n      - 1\n", "-I=2", "- k:\n    ? [a]\n    : - 1\n"),
        ("- ? k\n  :\n    ? [a]\n    :\n      - 1\n", "-I=3", "- k:\n   ? [a]\n   :  - 1\n"),
        ("- ? k\n  :\n    ? [a]\n    :\n      - 1\n", "-I=4", "- k:\n    ? [a]\n    :   - 1\n"),
        ("- ? k\n  :\n    ? [a]\n    :\n      - 1\n", "-I=5", "- k:\n     ? [a]\n     :    - 1\n"),
        ("- ? k\n  :\n    ? [a]\n    :\n      - 1\n", "-I=6", "- k:\n      ? [a]\n      :     - 1\n"),
        ("- ? k\n  :\n    ? [a]\n    :\n      - 1\n", "-I=8", "- k:\n        ? [a]\n        :       - 1\n"),
    ];
    check(&rows)
}
