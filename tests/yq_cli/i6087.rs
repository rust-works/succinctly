//! #6087: a descent's `|=` updates the *nodes* it selects, as yq does, so an outer node's update cannot move an
//! inner one out of reach. `(.. | select(tag == "!!map")) |= with_entries(.key |= "KEY_" + .)` renames every
//! level; it renamed the outer one and wrote `b: null` at the inner node's old name. Every expectation is a live
//! yq v4.53.3 capture.

use anyhow::Result;

use crate::run_yq_stdin;

const FLAT: &str = "a: 1\nb:\n  b_a: nested\n  b_b: thing\n";
const NESTED: &str = "l:
  - x: 1
    y:
      - 1
      - 2
  - x: 2
    y:
      - 3
m:
  n:
    o:
      p: 1
";

/// (document, filter, expected YAML output).
const ROWS: &[(&str, &str, &str)] = &[
    (
        FLAT,
        r#"(.. | select(tag=="!!map")) |= with_entries(.key |= "KEY_" + .)"#,
        "KEY_a: 1\nKEY_b:\n  KEY_b_a: nested\n  KEY_b_b: thing\n",
    ),
    (
        FLAT,
        r#"(.. | select(tag=="!!map")) |= keys"#,
        "- a\n- b\n",
    ),
    (
        FLAT,
        r#"(.. | select(tag=="!!map")) |= with_entries(.value |= "v")"#,
        "a: v\nb: v\n",
    ),
    (
        FLAT,
        r#".. |= (select(tag=="!!map") | with_entries(.key |= "K_" + .))"#,
        "K_a: 1\nK_b:\n  K_b_a: nested\n  K_b_b: thing\n",
    ),
    (
        FLAT,
        r#"(.. | select(tag=="!!map")) |= {"w": .}"#,
        "w:\n  a: 1\n  b:\n    w:\n      b_a: nested\n      b_b: thing\n",
    ),
    (
        FLAT,
        r#"(.. | select(tag=="!!map")) |= (. * {"z": 1})"#,
        "a: 1\nb:\n  b_a: nested\n  b_b: thing\n  z: 1\nz: 1\n",
    ),
    (
        FLAT,
        r#"(.. | select(tag=="!!map")) |= with_entries(.key |= "K_" + .) | (.. | select(tag=="!!map")) |= with_entries(.key |= "J_" + .)"#,
        "J_K_a: 1\nJ_K_b:\n  J_K_b_a: nested\n  J_K_b_b: thing\n",
    ),
    (
        NESTED,
        r#"(.. | select(tag=="!!map")) |= with_entries(.key |= "K_" + .)"#,
        "K_l:\n  - K_x: 1\n    K_y:\n      - 1\n      - 2\n  - K_x: 2\n    K_y:\n      - 3\nK_m:\n  K_n:\n    K_o:\n      K_p: 1\n",
    ),
    (
        NESTED,
        r#"(.. | select(tag=="!!seq")) |= length"#,
        "l: 2\nm:\n  n:\n    o:\n      p: 1\n",
    ),
    (
        NESTED,
        r#"(.. | select(tag=="!!map")) |= {"w": .}"#,
        "w:\n  l:\n    - w:\n        x: 1\n        y:\n          - 1\n          - 2\n    - w:\n        x: 2\n        y:\n          - 3\n  m:\n    w:\n      n:\n        w:\n          o:\n            w:\n              p: 1\n",
    ),
];

#[test]
fn a_descent_update_reaches_every_level_as_in_yq() -> Result<()> {
    for (doc, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out, *want, "{filter}");
    }
    Ok(())
}

/// A path set that is not from a descent keeps the order it was named in; only a descent is written
/// deepest first. Known gap, pinned so a change is deliberate: yq answers `a: {w: {b: {w: 1}}}` for
/// `(.a, .a.b) |= {"w": .}`.
#[test]
fn a_comma_of_nested_paths_keeps_its_named_order_gap_6087() -> Result<()> {
    let (out, code) = run_yq_stdin(
        r#"(.a, .a.b) |= {"w": .}"#,
        "a:\n  b: 1\n",
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(out.trim_end(), r#"{"a":{"w":{"b":1},"b":{"w":null}}}"#);
    Ok(())
}
