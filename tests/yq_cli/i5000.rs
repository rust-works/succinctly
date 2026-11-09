//! #5000: yq's `..` walks a mapping's own entries (a `<<` key among them), not the members a merge brings in.

use anyhow::Result;

use crate::run_yq_stdin;

const MERGED: &str = "a: &a {x: 1, z: 9}\nb: &b {y: 2, z: 8}\nitem: {<<: [*a, *b], k: 0}\n";

/// Every row is captured from yq v4.53.3 (`-o=json -I0`). The `<<` entry is a node of the mapping, its alias
/// sequence is walked and each alias stops there; the merged members are the anchors' nodes and are not visited
/// from the mapping.
#[test]
fn test_5000_recursive_descent_walks_a_mappings_own_entries_not_its_merged_members() -> Result<()> {
    let rows: &[(&str, &str, &str)] = &[
        (MERGED, "[.item | .. | select(tag==\"!!int\")]", "[0]"),
        (MERGED, "[.item | .. | path]", "[[\"item\"],[\"item\",\"<<\"],[\"item\",\"<<\",0],[\"item\",\"<<\",1],[\"item\",\"k\"]]"),
        // The anchors' own nodes are still walked from the document root.
        (MERGED, "[.. | select(tag==\"!!int\")]", "[1,9,2,8,0]"),
        // A mapping with no merge key is walked as before.
        ("p: {q: 1, r: [2, 3]}\n", "[.. | select(tag==\"!!int\")]", "[1,2,3]"),
    ];
    for (input, filter, want) in rows {
        let (out, code) = run_yq_stdin(filter, input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter} on {input:?}: {out:?}");
        assert_eq!(out.trim_end(), *want, "{filter} on {input:?}");
    }
    Ok(())
}
