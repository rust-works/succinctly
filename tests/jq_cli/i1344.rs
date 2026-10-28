//! #1344, ADR-0027: only `succinctly yq` records a repeated mapping key's earlier entry in an owned
//! value (I4). jq reads `{"a":1,"a":2}` as `{"a":2}` on every route, so jq mode keeps collapsing --
//! pinned over the routes the yq mode now keeps it on.

use anyhow::Result;

use crate::run_jq_stdin;

#[test]
fn jq_mode_collapses_a_repeated_key_on_every_owned_route_1344() -> Result<()> {
    for (filter, expected) in [
        ("[.] | .[0]", r#"{"a":2,"b":3}"#),
        (". as $d | $d", r#"{"a":2,"b":3}"#),
        ("with_entries(.)", r#"{"a":2,"b":3}"#),
        ("to_entries | from_entries", r#"{"a":2,"b":3}"#),
        ("map_values(. + 10)", r#"{"a":12,"b":13}"#),
        (". + {\"q\": 1}", r#"{"a":2,"b":3,"q":1}"#),
        (".a = 5", r#"{"a":5,"b":3}"#),
        ("[..]", r#"[{"a":2,"b":3},2,3]"#),
    ] {
        let (out, code) = run_jq_stdin(filter, r#"{"a":1,"a":2,"b":3}"#, &["-c"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out.trim(), expected, "{filter}");
    }
    Ok(())
}
