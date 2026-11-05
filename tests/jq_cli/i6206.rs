//! #6206: `key` is not a jq 1.7.1 builtin (`key/0 is not defined`), so `succinctly jq`'s own `key`
//! has no reference to differ from. A `select` over it in the target of a write takes the resolver
//! `succinctly yq` takes, and so judges the condition against the key of the node it stands at.

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn a_select_over_key_in_a_write_target_reaches_the_matching_nodes_6206() -> Result<()> {
    for (filter, expected) in [
        (
            r#"(.. | select(key == "x")) = 5"#,
            r#"{"a":{"x":5,"y":{"x":5}}}"#,
        ),
        (
            r#"(.. | select(key == "x")) |= key"#,
            r#"{"a":{"x":"x","y":{"x":"x"}}}"#,
        ),
        (r#"del(.. | select(key == "x"))"#, r#"{"a":{"y":{}}}"#),
        (
            r#"(.. | select(if key == "x" then true else false end)) = 5"#,
            r#"{"a":{"x":5,"y":{"x":5}}}"#,
        ),
        (
            r#"(.. | select(key == "nope")) = 5"#,
            r#"{"a":{"x":1,"y":{"x":2}}}"#,
        ),
    ] {
        let (out, code) = run_jq_stdin(filter, r#"{"a":{"x":1,"y":{"x":2}}}"#, &["-c"])?;
        assert_eq!(out.trim_end(), expected, "{filter}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}
