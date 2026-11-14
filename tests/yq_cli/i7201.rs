//! #7201: iterating a mapping whose `<<` source itself merges lists the source's own merged members,
//! as yq's `doTraverseMap` does, instead of a literal `<<` entry. Rows captured from yq v4.53.3
//! (default merge reading, `-o json -I 0`); `.a.q` and `.a` already agreed and stay pinned.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "b: &b {p: 1, q: 2}
n: &n {<<: *b, n: 5}
a: {<<: *n, p: 7}
c: {<<: [*n, *b]}
";

#[test]
fn iteration_follows_a_sources_own_merge_7201() -> Result<()> {
    let rows = [
        (".a[]", "7\n2\n5\n"),
        ("[.a[] | key]", "[\"p\",\"q\",\"n\"]\n"),
        (".c[]", "1\n2\n5\n"),
        ("[.c[] | key]", "[\"p\",\"q\",\"n\"]\n"),
        (".n[]", "1\n2\n5\n"),
        (".a.q", "2\n"),
        (".a", "{\"<<\":{\"p\":1,\"q\":2},\"n\":5,\"p\":7}\n"),
    ];
    for (filter, expected) in rows {
        let (out, code) = run_yq_stdin(filter, DOC, &["-o", "json", "-I", "0"])?;
        assert_eq!((out.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}
