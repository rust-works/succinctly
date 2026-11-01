//! #5426: `contains` reads a repeated mapping key's earliest entry, as yq v4.53.3 does
//! (`a: 1 / a: 2` is `contains({"a":1})` = true, `contains({"a":2})` = false).

use anyhow::Result;

use crate::run_yq_stdin;

const DUP: &str = "a: 1\na: 2\nb: 3\n";

#[test]
fn contains_reads_the_earliest_entry_of_a_repeated_key() -> Result<()> {
    for (filter, expected) in [
        (r#"contains({"a":1})"#, "true\n"),
        (r#"contains({"a":2})"#, "false\n"),
        (r#"contains({"a":3})"#, "false\n"),
        (r#"contains({"a":1,"b":3})"#, "true\n"),
        (r#"contains({"b":3})"#, "true\n"),
    ] {
        let (out, code) = run_yq_stdin(filter, DUP, &["-o=json", "-I0"])?;
        assert_eq!((out.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn contains_without_a_repeated_key_is_unchanged() -> Result<()> {
    let (out, _) = run_yq_stdin(r#"contains({"a":2})"#, "a: 2\nb: 3\n", &["-o=json", "-I0"])?;
    assert_eq!(out, "true\n");
    Ok(())
}

#[test]
fn contains_reads_the_earliest_entry_at_every_depth() -> Result<()> {
    let doc = "x:\n  a: 1\n  a: 2\n  b: 3\nl:\n  - a: 1\n    a: 2\nt: 1\nt: 2\nt: 3\n";
    for (filter, expected) in [
        (".x | contains({\"a\":1})", "true\n"),
        (".x | contains({\"a\":2})", "false\n"),
        (".l | contains([{\"a\":1}])", "true\n"),
        (".l | contains([{\"a\":2}])", "false\n"),
        ("contains({\"t\":1})", "true\n"),
        ("contains({\"t\":3})", "false\n"),
        ("contains({\"x\":{\"a\":1}})", "true\n"),
        ("contains({\"x\":{\"a\":2}})", "false\n"),
    ] {
        let (out, code) = run_yq_stdin(filter, doc, &["-o=json", "-I0"])?;
        assert_eq!((out.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}
