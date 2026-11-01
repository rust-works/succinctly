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

#[test]
fn object_slice_counts_a_repeated_keys_earlier_entries() -> Result<()> {
    // yq's child list is `[a,1,a,2,b,3]`: one key/value pair per entry, not per distinct key.
    for (filter, expected) in [
        (".[0:2]", "[\"a\",1]\n"),
        (".[1:]", "[1,\"a\"]\n"),
        (".[2:5]", "[\"a\",2,\"b\"]\n"),
        (".[-3:-1]", "[2,\"b\"]\n"),
        (".[:2]", "[\"a\",1]\n"),
        (".[5:]", "[]\n"),
        ("[.] | .[0] | .[1:4]", "[1,\"a\",2]\n"),
    ] {
        let (out, code) = run_yq_stdin(filter, DUP, &["-o=json", "-I0"])?;
        assert_eq!((out.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn object_slice_without_a_repeated_key_is_unchanged() -> Result<()> {
    let (out, _) = run_yq_stdin(".[1:3]", "a: 1\nb: 2\nc: 3\n", &["-o=json", "-I0"])?;
    assert_eq!(out, "[1,\"b\"]\n");
    Ok(())
}

/// ADR-0027 P4: `sort_keys` keeps every entry of a repeated key and stable-sorts them. yq copies
/// the last value over the earlier entries (`{"a":2,"a":2,"b":3}`), which would destroy a
/// document value, so this is a recorded divergence (ADR-0018), not a match.
#[test]
fn sort_keys_stable_sorts_a_repeated_keys_entries() -> Result<()> {
    let (out, code) = run_yq_stdin("sort_keys(.)", "b: 0\na: 1\nb: 5\na: 2\n", &["-o=json", "-I0"])?;
    assert_eq!((out.as_str(), code), ("{\"a\":1,\"a\":2,\"b\":0,\"b\":5}\n", 0));
    let (out, _) = run_yq_stdin("sort_keys(.)", DUP, &["-o=json", "-I0"])?;
    assert_eq!(out, "{\"a\":1,\"a\":2,\"b\":3}\n");
    let (out, _) = run_yq_stdin(".x | sort_keys(.)", "x:\n  b: 1\n  a: 1\n  b: 2\n", &["-o=json", "-I0"])?;
    assert_eq!(out, "{\"a\":1,\"b\":1,\"b\":2}\n");
    Ok(())
}

#[test]
fn sort_keys_without_a_repeated_key_is_unchanged() -> Result<()> {
    let (out, _) = run_yq_stdin("sort_keys(.)", "b: 2\na: 1\n", &["-o=json", "-I0"])?;
    assert_eq!(out, "{\"a\":1,\"b\":2}\n");
    Ok(())
}
