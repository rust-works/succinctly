//! #2643: yq's `X as $y` binds a *copy* of the node, so a write through `$y` (`$y.b = 5`,
//! `$y[0] |= f`, `$y.b += 1`, `del($y.b)`) changes the copy and leaves the document alone. Every
//! later read of `$y` sees the change, the right side of `=` and `op=` is evaluated against the
//! stage's input and not against `$y`, and the write stage itself answers its input. Every row is
//! `yq -o=json -I0 FILTER` captured from yq v4.53.3.
//!
//! Not covered here, and recorded in `docs/compliance/yq/limitations.md`: a bare `del($y)` (yq
//! deletes the bound node from its live parent), a var-rooted path nested in an assignment's left
//! side (`(.a as $y | .c | $y.b) = 5`), and a write inside a comma.

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin};

const MAP: &str = "a: {b: 1}\nc: {b: 1}\n";
const ARR: &str = "a:\n  - 1\n  - 2\n  - 3\n";
const ANC: &str = "a: &x {p: 1}\nb: *x\n";

/// (filter, stdout) on [`MAP`], each the same as yq v4.53.3's with exit code 0.
const MAP_ROWS: &[(&str, &str)] = &[
    (".a as $y | $y.b = 5", "{\"a\":{\"b\":1},\"c\":{\"b\":1}}\n"),
    (".a as $y | $y.b = 5 | $y", "{\"b\":5}\n"),
    (".a as $y | $y.b = .c.b + 10 | $y", "{\"b\":11}\n"),
    (".a as $y | $y.b |= . + 100 | $y", "{\"b\":101}\n"),
    (".a as $y | $y.b += .c.b | $y", "{\"b\":2}\n"),
    (".a as $y | del($y.b) | $y", "{}\n"),
    (".a as $y | $y = 5 | $y", "5\n"),
    (". as $y | del($y)", "{\"a\":{\"b\":1},\"c\":{\"b\":1}}\n"),
    (". as $y | del($y.a)", "{\"a\":{\"b\":1},\"c\":{\"b\":1}}\n"),
    (
        ". as $y | $y.a.b = 5",
        "{\"a\":{\"b\":1},\"c\":{\"b\":1}}\n",
    ),
    ("(.a as $y | $y) = 5", "{\"a\":{\"b\":1},\"c\":{\"b\":1}}\n"),
    (".a as $y | $y.b = (1,2) | $y", "{\"b\":2}\n"),
    (".a as $y | $y.b = 5 | .a as $y | $y", "{\"b\":1}\n"),
    (".a as $y | $y.b = 5 | .a.b", "1\n"),
    (
        ".a as $y | ($y.b = 5), $y",
        "{\"a\":{\"b\":1},\"c\":{\"b\":1}}\n",
    ),
    (".a as $y | .c.b = 7 | $y.b = 9 | .c", "{\"b\":7}\n"),
    (".a as $y | ($y.b = 5) | $y", "{\"b\":5}\n"),
    (".a as $y | ($y).b = 5 | $y", "{\"b\":5}\n"),
];

/// (filter, stdout) on [`ARR`], each the same as yq v4.53.3's with exit code 0.
const ARR_ROWS: &[(&str, &str)] = &[
    (".a as $x | $x[0] = 9", "{\"a\":[1,2,3]}\n"),
    (".a as $x | del($x[0])", "{\"a\":[1,2,3]}\n"),
    (".a as $x | $x[0] |= 9 | $x", "[9,2,3]\n"),
];

/// (filter, stdout) on [`ANC`], each the same as yq v4.53.3's with exit code 0.
const ANC_ROWS: &[(&str, &str)] = &[
    (
        ".a as $y | $y.p = 5 | .",
        "{\"a\":{\"p\":1},\"b\":{\"p\":1}}\n",
    ),
    (".a as $y | del($y.p) | $y", "{}\n"),
];

fn check(doc: &str, rows: &[(&str, &str)]) -> Result<()> {
    for (filter, expected) in rows {
        let (stdout, code) = run_yq_stdin(filter, doc, &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

#[test]
fn a_write_through_a_bound_mapping_changes_the_copy_2643() -> Result<()> {
    check(MAP, MAP_ROWS)
}

#[test]
fn a_write_through_a_bound_sequence_changes_the_copy_2643() -> Result<()> {
    check(ARR, ARR_ROWS)
}

#[test]
fn a_write_through_a_bound_anchor_changes_the_copy_2643() -> Result<()> {
    check(ANC, ANC_ROWS)
}

/// The jq rule is unchanged: `succinctly jq` refuses a write through a bound variable, as jq does
/// (the mode decides, never the input format).
#[test]
fn jq_mode_still_refuses_a_write_through_a_bound_variable_2643() -> Result<()> {
    for filter in [".a as $y | $y.b = 5", ".a as $y | del($y.b)"] {
        let (stdout, code) = run_jq_stdin(filter, "{\"a\":{\"b\":1}}", &["-c"])?;
        assert_ne!(code, 0, "{filter}");
        assert_eq!(stdout, "", "{filter}");
    }
    Ok(())
}
