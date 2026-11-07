//! #6414: a property in front of a block sequence `-` on its line, and a tab after the `-`, are
//! refused by yq: `block sequence entries are not allowed in this context` for the first, and
//! `found character that cannot start any token` for the second. `succinctly yq` read both as a
//! sequence (`&t - x` was `["x"]`, `-⇥x` was `["x"]`, and `&t - a: b` was `{"- a": "b"}`). Every
//! verdict was captured from yq v4.53.3; the grid behind them (7 property forms, 11 contexts,
//! 6 tails, and 14 tab shapes in 4 contexts: 518 documents) has no difference in verdict.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const PROPERTY: &str = "block sequence entries are not allowed in this context";
const TAB: &str = "found character that cannot start any token";

#[test]
fn a_property_in_front_of_a_dash_on_its_line_is_refused_6414() -> Result<()> {
    for input in [
        "&t - x\n",
        "&t - x\n  - y\n",
        "&t -\n",
        "&t - - x\n",
        "&a &b - x\n",
        "&t !!seq - x\n",
        "!!seq - x\n",
        "- &t - x\n",
        "- !!seq - x\n",
        "a: &t - x\n",
        "a:\n  b: &t - x\n",
        "? &t - x\n: v\n",
        "&t - a: b\n",
        "- &t - a: b\n",
        "---\n&t - x\n",
        "a: 1\nb: &t - x\n",
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(".", input, &["-o=json", "-I=0"])?;
        assert_ne!(code, 0, "{input:?}");
        assert_eq!(out, "", "{input:?}");
        assert!(err.contains(PROPERTY), "{input:?}: {err}");
    }
    Ok(())
}

#[test]
fn a_property_on_its_own_line_above_a_dash_is_still_read_6414() -> Result<()> {
    for (input, expected) in [
        ("&t\n- x\n", "[\"x\"]"),
        ("!!seq\n- x\n", "[\"x\"]"),
        ("- &a\n  - x\n", "[[\"x\"]]"),
        ("a: &t\n  - x\n", "{\"a\":[\"x\"]}"),
        ("- &a x\n", "[\"x\"]"),
        ("- !!str x\n", "[\"x\"]"),
        ("- # c\n- x\n", "[null,\"x\"]"),
        ("-  # c\n", "[null]"),
        ("- - x\n", "[[\"x\"]]"),
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(".", input, &["-o=json", "-I=0"])?;
        assert_eq!((code, out.trim_end()), (0, expected), "{input:?}: {err}");
    }
    Ok(())
}

#[test]
fn a_tab_after_a_dash_is_refused_6414() -> Result<()> {
    for input in [
        "-\tx\n",
        "- \tx\n",
        "-\t\tx\n",
        "-\t\n",
        "- \t\n",
        "-\t# c\n",
        "- \t# c\n",
        "-\t- x\n",
        "-\ta: b\n",
        "a:\n  -\tx\n",
        "a:\n  - b\n  -\t# c\n",
        "- a:\n    -\tx\n",
        "? -\tx\n: v\n",
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(".", input, &["-o=json", "-I=0"])?;
        assert_ne!(code, 0, "{input:?}");
        assert_eq!(out, "", "{input:?}");
        assert!(err.contains(TAB), "{input:?}: {err}");
    }
    Ok(())
}

#[test]
fn a_tab_elsewhere_in_an_item_is_still_read_6414() -> Result<()> {
    for (input, expected) in [
        ("- x\t# c\n", "[\"x\"]"),
        ("- a\ty\n", "[\"a\\ty\"]"),
        ("- \"a\\tb\"\n", "[\"a\\tb\"]"),
        ("- |\n  t\n", "[\"t\\n\"]"),
        ("- # c\n", "[null]"),
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(".", input, &["-o=json", "-I=0"])?;
        assert_eq!((code, out.trim_end()), (0, expected), "{input:?}: {err}");
    }
    Ok(())
}
