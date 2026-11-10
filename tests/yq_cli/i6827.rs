//! #6827: a write made by an update body on a mapping that merges a source (`.list[] |= (.x = 7)`,
//! `with(.item; .x = 4)`) reaches the source's node, as in yq, where #6785 changed only the
//! mapping's own copy. yq's `.x` inside the body finds the node the merge brought in, so the
//! source and every mapping that merges it follow. Each row is a live yq v4.53.3 capture
//! (`yq -o=json -I=0`).

use anyhow::Result;

use crate::run_yq_stdin;

const JSON: &[&str] = &["-o=json", "-I=0"];

/// Two sources hold `z`.
const TWO: &str = "a: &a {x: 1, z: 9}\nb: &b {y: 2, z: 8}\nitem: {<<: [*a, *b], k: 0}\n";

/// One source, and two more mappings that merge it.
const ONE: &str = "base: &base {x: 1, y: 2, n: {p: 1, q: 2}}\nitem: {<<: *base, k: 0}\nlist:\n  - {<<: *base, i: 0}\n  - {<<: *base, i: 1}\n";

/// A whole source.
const WHOLE: &str = "a: &a {x: 1}\nitem: {<<: *a, k: 0}\n";

/// A source and two aliases of it, no merge key.
const ALIAS: &str = "base: &base {x: 1, y: 2}\nlist: [*base, *base]\n";

/// A mapping with a merged member that is itself anchored and aliased.
const ALIASED: &str = "base: &base {m: 1}\nl:\n  - {<<: *base}\n  - &s {<<: *base, z: 1}\n  - *s\n";

/// `(document, filter, yq's output)`: a body that writes below the node it runs on.
const BODY_WRITES: &[(&str, &str, &str)] = &[
    (ONE, ".list[] |= (.x = 7)", "{\"base\":{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, "with(.item; .x = 4)", "{\"base\":{\"x\":4,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":4,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":4,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":4,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".item |= (.x |= . + 1)", "{\"base\":{\"x\":2,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":2,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":2,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":2,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list[0] |= (.x = 7 | .y = 8)", "{\"base\":{\"x\":7,\"y\":8,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":7,\"y\":8,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":7,\"y\":8,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":7,\"y\":8,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list[] |= del(.x)", "{\"base\":{\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".item |= (.k = 5)", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":5},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".item |= (.n.p = 9)", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":9,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":9,\"q\":2},\"k\":0},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":9,\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":9,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list[1] |= (.i = 5)", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":5}]}\n"),
    (ONE, ".item |= (.zz = 1)", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0,\"zz\":1},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, "with(.item; .x = 4 | .y = 5)", "{\"base\":{\"x\":4,\"y\":5,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":4,\"y\":5,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":4,\"y\":5,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":4,\"y\":5,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".item |= del(.n.p)", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"q\":2},\"k\":0},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"q\":2},\"i\":1}]}\n"),
    (TWO, ".item |= (.x = 5)", "{\"a\":{\"x\":5,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"x\":5,\"k\":0}}\n"),
    (TWO, ".item |= (.z = 7)", "{\"a\":{\"x\":1,\"z\":9},\"b\":{\"y\":2,\"z\":7},\"item\":{\"y\":2,\"z\":9,\"x\":1,\"k\":0}}\n"),
    (TWO, ".item |= del(.x)", "{\"a\":{\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"k\":0}}\n"),
    (TWO, ".item |= (.z = 7 | .x = 5)", "{\"a\":{\"x\":5,\"z\":9},\"b\":{\"y\":2,\"z\":7},\"item\":{\"y\":2,\"z\":9,\"x\":5,\"k\":0}}\n"),
    (ALIASED, ".l[1] |= (.m = 7)", "{\"base\":{\"m\":7},\"l\":[{\"m\":7},{\"m\":7,\"z\":1},{\"m\":7,\"z\":1}]}\n"),
    (WHOLE, ".item |= (.x = 9)", "{\"a\":{\"x\":9},\"item\":{\"x\":9,\"k\":0}}\n"),
    (ONE, ".list[-1] |= (.x = 7)", "{\"base\":{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".item = {\"k\": 0} | .item |= (.q = 1)", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"k\":0,\"q\":1},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".item = {\"x\": 99, \"k\": 0} | .item |= (.q = 1)", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":99,\"k\":0,\"q\":1},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ALIAS, ".list[] |= (.x = 7)", "{\"base\":{\"x\":7,\"y\":2},\"list\":[{\"x\":7,\"y\":2},{\"x\":7,\"y\":2}]}\n"),
    (ALIAS, ".list[0] |= (.p = 9)", "{\"base\":{\"x\":1,\"y\":2,\"p\":9},\"list\":[{\"x\":1,\"y\":2,\"p\":9},{\"x\":1,\"y\":2,\"p\":9}]}\n"),
    (ONE, ".list[] |= (.x = .i)", "{\"base\":{\"x\":0,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":0,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":0,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":0,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list[] |= (.x = 7, .y = 1)", "{\"base\":{\"x\":7,\"y\":1,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":7,\"y\":1,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":7,\"y\":1,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":7,\"y\":1,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
];

/// A body that replaces the mapping leaves the sources alone, in yq as here.
const BODY_REPLACES: &[(&str, &str, &str)] = &[
    (ONE, ".item |= (. = {\"x\": 5})", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":5},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list[0] |= (. = {\"x\": 5, \"i\": 0})", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":5,\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".item |= {\"x\": 5}", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":5},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
];

/// `(document, filter, yq's output, this tool's output)`: the shapes the update body's edit is not
/// carried for, which still change only the mapping's copy.
const STILL_DIVERGENT: &[(&str, &str, &str, &str)] = &[
    (ONE, ".list[0] |= (del(.x) | .x = 3)", "{\"base\":{\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0,\"x\":3},{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0,\"x\":3},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list[0] |= (.x = 3 | del(.y))", "{\"base\":{\"x\":3,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":3,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":3,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":3,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":3,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list[0] |= ((del(.x)) | .x = 3)", "{\"base\":{\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0,\"x\":3},{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0,\"x\":3},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list[] |= (.x += 1)", "{\"base\":{\"x\":3,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":3,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":3,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":3,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n", "{\"base\":{\"x\":2,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":2,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":2,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":2,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list[0:1] |= (.[0].x = 5)", "{\"base\":{\"x\":5,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":5,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":5,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":5,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list |= map(.x = 7)", "{\"base\":{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
];

fn check(rows: &[(&str, &str, &str)]) -> Result<()> {
    for (document, filter, want) in rows {
        let (stdout, code) = run_yq_stdin(filter, document, JSON)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, 0),
            "{filter} on {document:?}"
        );
    }
    Ok(())
}

#[test]
fn test_an_update_body_write_on_a_merging_mapping_reaches_the_source_6827() -> Result<()> {
    check(BODY_WRITES)
}

#[test]
fn test_an_update_body_that_replaces_the_mapping_leaves_the_sources_alone_6827() -> Result<()> {
    check(BODY_REPLACES)
}

#[test]
fn test_the_body_shapes_that_still_change_only_the_mappings_copy_6827() -> Result<()> {
    // Not yq's answers, recorded in docs/compliance/yq/limitations.md. A body that deletes a key and
    // then writes more (also behind a group) cannot be told from a changed member by comparing the
    // two documents. A body that reads the member it writes (`.x += 1` over mappings that share a
    // source) writes the source once, where yq counts each mapping's turn. A slice target, and a body that is not a chain of writes (`map(.x = 7)`) is not looked into. Both
    // answers are pinned, so a change in either shows.
    for (document, filter, yq, ours) in STILL_DIVERGENT {
        let (stdout, code) = run_yq_stdin(filter, document, JSON)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*ours, 0),
            "{filter} on {document:?}"
        );
        assert_ne!(
            ours, yq,
            "{filter} on {document:?} now matches yq: move the row to BODY_WRITES"
        );
    }
    Ok(())
}
