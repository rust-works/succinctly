//! #6209: a plain scalar's continuation line drops every blank in front of its text, spaces after a tab
//! included (go-yaml folds `x` + break + `\t y` to `x y`); the tab itself is refused below the enclosing
//! indent (#6059). Every row was captured from yq v4.53.3. The same fold after a `---` marker on the
//! scalar's line is a separate yq quirk (the marker line's first break is dropped), not pinned here.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_continuation_line_drops_all_of_its_leading_blanks_6209() -> Result<()> {
    for (input, expected) in [
        ("a: [x\n y]\n", "{\"a\":[\"x y\"]}"),
        ("a: [x\n  y]\n", "{\"a\":[\"x y\"]}"),
        ("a: [x\n \ty]\n", "{\"a\":[\"x y\"]}"),
        ("a: [x\n \t y]\n", "{\"a\":[\"x y\"]}"),
        ("a: [x\n  \ty]\n", "{\"a\":[\"x y\"]}"),
        ("[x\n y]\n", "[\"x y\"]"),
        ("[x\n  y]\n", "[\"x y\"]"),
        ("[x\n\ty]\n", "[\"x y\"]"),
        ("[x\n\t\ty]\n", "[\"x y\"]"),
        ("[x\n \ty]\n", "[\"x y\"]"),
        ("[x\n\t y]\n", "[\"x y\"]"),
        ("[x\n \t y]\n", "[\"x y\"]"),
        ("[x\n\t \ty]\n", "[\"x y\"]"),
        ("[x\n  \ty]\n", "[\"x y\"]"),
        ("[x\n\t  y]\n", "[\"x y\"]"),
        ("a: {k: x\n y}\n", "{\"a\":{\"k\":\"x y\"}}"),
        ("a: {k: x\n  y}\n", "{\"a\":{\"k\":\"x y\"}}"),
        ("a: {k: x\n \ty}\n", "{\"a\":{\"k\":\"x y\"}}"),
        ("a: {k: x\n \t y}\n", "{\"a\":{\"k\":\"x y\"}}"),
        ("a: {k: x\n  \ty}\n", "{\"a\":{\"k\":\"x y\"}}"),
        ("{k: x\n y}\n", "{\"k\":\"x y\"}"),
        ("{k: x\n  y}\n", "{\"k\":\"x y\"}"),
        ("{k: x\n\ty}\n", "{\"k\":\"x y\"}"),
        ("{k: x\n\t\ty}\n", "{\"k\":\"x y\"}"),
        ("{k: x\n \ty}\n", "{\"k\":\"x y\"}"),
        ("{k: x\n\t y}\n", "{\"k\":\"x y\"}"),
        ("{k: x\n \t y}\n", "{\"k\":\"x y\"}"),
        ("{k: x\n\t \ty}\n", "{\"k\":\"x y\"}"),
        ("{k: x\n  \ty}\n", "{\"k\":\"x y\"}"),
        ("{k: x\n\t  y}\n", "{\"k\":\"x y\"}"),
        ("a: x\n y\n", "{\"a\":\"x y\"}"),
        ("a: x\n  y\n", "{\"a\":\"x y\"}"),
        ("a: x\n \ty\n", "{\"a\":\"x y\"}"),
        ("a: x\n \t y\n", "{\"a\":\"x y\"}"),
        ("a: x\n  \ty\n", "{\"a\":\"x y\"}"),
        ("- x\n y\n", "[\"x y\"]"),
        ("- x\n  y\n", "[\"x y\"]"),
        ("- x\n \ty\n", "[\"x y\"]"),
        ("- x\n \t y\n", "[\"x y\"]"),
        ("- x\n  \ty\n", "[\"x y\"]"),
        ("x\n y\n", "\"x y\""),
        ("x\n  y\n", "\"x y\""),
        ("x\n\ty\n", "\"x y\""),
        ("x\n\t\ty\n", "\"x y\""),
        ("x\n \ty\n", "\"x y\""),
        ("x\n\t y\n", "\"x y\""),
        ("x\n \t y\n", "\"x y\""),
        ("x\n\t \ty\n", "\"x y\""),
        ("x\n  \ty\n", "\"x y\""),
        ("x\n\t  y\n", "\"x y\""),
        ("---\nx\n y\n", "\"x y\""),
        ("---\nx\n  y\n", "\"x y\""),
        ("---\nx\n\ty\n", "\"x y\""),
        ("---\nx\n\t\ty\n", "\"x y\""),
        ("---\nx\n \ty\n", "\"x y\""),
        ("---\nx\n\t y\n", "\"x y\""),
        ("---\nx\n \t y\n", "\"x y\""),
        ("---\nx\n\t \ty\n", "\"x y\""),
        ("---\nx\n  \ty\n", "\"x y\""),
        ("---\nx\n\t  y\n", "\"x y\""),
        ("a: x\n  y\n", "{\"a\":\"x y\"}"),
        ("a: x\n   y\n", "{\"a\":\"x y\"}"),
        ("a: x\n \ty\n", "{\"a\":\"x y\"}"),
        ("a: x\n \t\ty\n", "{\"a\":\"x y\"}"),
        ("a: x\n  \ty\n", "{\"a\":\"x y\"}"),
        ("a: x\n \t y\n", "{\"a\":\"x y\"}"),
        ("a: x\n  \t y\n", "{\"a\":\"x y\"}"),
        ("a: x\n \t \ty\n", "{\"a\":\"x y\"}"),
        ("a: x\n   \ty\n", "{\"a\":\"x y\"}"),
        ("a: x\n \t  y\n", "{\"a\":\"x y\"}"),
        ("a: x\n\n y\n", "{\"a\":\"x\\ny\"}"),
        ("a: x\n\n  y\n", "{\"a\":\"x\\ny\"}"),
        ("a: x\n\n \ty\n", "{\"a\":\"x\\ny\"}"),
        ("a: x\n\n \t y\n", "{\"a\":\"x\\ny\"}"),
        ("a: x\n\n  \ty\n", "{\"a\":\"x\\ny\"}"),
        ("a: [x\n y\n z]\n", "{\"a\":[\"x y z\"]}"),
        ("a: [x\n  y\n  z]\n", "{\"a\":[\"x y z\"]}"),
        ("a: [x\n \ty\n \tz]\n", "{\"a\":[\"x y z\"]}"),
        ("a: [x\n \t y\n \t z]\n", "{\"a\":[\"x y z\"]}"),
        ("a: [x\n  \ty\n  \tz]\n", "{\"a\":[\"x y z\"]}"),
        ("a: [k: x\n y]\n", "{\"a\":[{\"k\":\"x y\"}]}"),
        ("a: [k: x\n  y]\n", "{\"a\":[{\"k\":\"x y\"}]}"),
        ("a: [k: x\n \ty]\n", "{\"a\":[{\"k\":\"x y\"}]}"),
        ("a: [k: x\n \t y]\n", "{\"a\":[{\"k\":\"x y\"}]}"),
        ("a: [k: x\n  \ty]\n", "{\"a\":[{\"k\":\"x y\"}]}"),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!((out.trim_end(), code), (expected, 0), "{input:?}");
    }
    Ok(())
}

#[test]
fn a_tab_that_starts_a_continuation_below_the_indent_is_still_refused_6209() -> Result<()> {
    for input in [
        "a: [x\n\ty]\n",
        "a: [x\n\t\ty]\n",
        "a: [x\n\t y]\n",
        "a: [x\n\t \ty]\n",
        "a: [x\n\t  y]\n",
        "a: {k: x\n\ty}\n",
        "a: {k: x\n\t\ty}\n",
        "a: {k: x\n\t y}\n",
        "a: {k: x\n\t \ty}\n",
        "a: {k: x\n\t  y}\n",
        "a: x\n\ty\n",
        "a: x\n\t\ty\n",
        "a: x\n\t y\n",
        "a: x\n\t \ty\n",
        "a: x\n\t  y\n",
        "- x\n\ty\n",
        "- x\n\t\ty\n",
        "- x\n\t y\n",
        "- x\n\t \ty\n",
        "- x\n\t  y\n",
        "a: x\n\n\ty\n",
        "a: x\n\n\t\ty\n",
        "a: x\n\n\t y\n",
        "a: x\n\n\t \ty\n",
        "a: x\n\n\t  y\n",
        "a: [x\n\ty\n\tz]\n",
        "a: [x\n\t\ty\n\t\tz]\n",
        "a: [x\n\t y\n\t z]\n",
        "a: [x\n\t \ty\n\t \tz]\n",
        "a: [x\n\t  y\n\t  z]\n",
        "a: [k: x\n\ty]\n",
        "a: [k: x\n\t\ty]\n",
        "a: [k: x\n\t y]\n",
        "a: [k: x\n\t \ty]\n",
        "a: [k: x\n\t  y]\n",
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(out.trim_end(), "", "{input:?}");
        assert_ne!(code, 0, "{input:?}");
    }
    Ok(())
}
