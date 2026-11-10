//! #6785: a write through a member a `<<` merge brought in changes the node the member comes from,
//! as in yq: with `a: &a {x: 1}` and `item: {<<: *a, k: 0}`, `.item.x = 5` sets `a.x`, and so the
//! `x` of every mapping that merges `a`. A write to the source is seen through the merging mapping
//! too (`.a.x = 5`), and a key the source loses leaves the mapping (`del(.item.x)`). Where several
//! sources hold a key, a write lands on the last one while the mapping shows the first
//! (`.item.z = 7` sets `b.z` and `item.z` stays `9`). Each row is a live yq v4.53.3 capture
//! (`yq -o=json -I=0`).

use anyhow::Result;

use crate::run_yq_stdin;

const JSON: &[&str] = &["-o=json", "-I=0"];

/// Two sources hold `z`.
const TWO: &str = "a: &a {x: 1, z: 9}\nb: &b {y: 2, z: 8}\nitem: {<<: [*a, *b], k: 0}\n";

/// One source, and two more mappings that merge it.
const ONE: &str = "base: &base {x: 1, y: 2, n: {p: 1, q: 2}}\nitem: {<<: *base, k: 0}\nlist:\n  - {<<: *base, i: 0}\n  - {<<: *base, i: 1}\n";

/// `(document, filter, yq's output)`.
const ROWS: &[(&str, &str, &str)] = &[
    (TWO, ".item.x = 5", "{\"a\":{\"x\":5,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"x\":5,\"k\":0}}\n"),
    (TWO, ".item.x |= . + 1", "{\"a\":{\"x\":2,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"x\":2,\"k\":0}}\n"),
    (TWO, "del(.item.x)", "{\"a\":{\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"k\":0}}\n"),
    (TWO, ".item.z = 7", "{\"a\":{\"x\":1,\"z\":9},\"b\":{\"y\":2,\"z\":7},\"item\":{\"y\":2,\"z\":9,\"x\":1,\"k\":0}}\n"),
    (TWO, "(.item | .[]) |= 7", "{\"a\":{\"x\":7,\"z\":9},\"b\":{\"y\":7,\"z\":7},\"item\":{\"y\":7,\"z\":9,\"x\":7,\"k\":7}}\n"),
    (TWO, ".item[] |= 7", "{\"a\":{\"x\":7,\"z\":9},\"b\":{\"y\":7,\"z\":7},\"item\":{\"y\":7,\"z\":9,\"x\":7,\"k\":7}}\n"),
    (TWO, ".item.k = 7", "{\"a\":{\"x\":1,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"x\":1,\"k\":7}}\n"),
    (TWO, ".item | .x = 5", "{\"y\":2,\"z\":9,\"x\":5,\"k\":0}\n"),
    (TWO, ".item.y = 5", "{\"a\":{\"x\":1,\"z\":9},\"b\":{\"y\":5,\"z\":8},\"item\":{\"y\":5,\"z\":9,\"x\":1,\"k\":0}}\n"),
    (TWO, ".item.nx = 1", "{\"a\":{\"x\":1,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"x\":1,\"k\":0,\"nx\":1}}\n"),
    (TWO, ".item.x += 2", "{\"a\":{\"x\":3,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"x\":3,\"k\":0}}\n"),
    (TWO, ".a.x = 5", "{\"a\":{\"x\":5,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"x\":5,\"k\":0}}\n"),
    (TWO, "del(.a.x)", "{\"a\":{\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"k\":0}}\n"),
    (TWO, ".b.z = 1", "{\"a\":{\"x\":1,\"z\":9},\"b\":{\"y\":2,\"z\":1},\"item\":{\"y\":2,\"z\":9,\"x\":1,\"k\":0}}\n"),
    (TWO, "del(.a.z)", "{\"a\":{\"x\":1},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":8,\"x\":1,\"k\":0}}\n"),
    (TWO, ".item.x = 5 | .item.x = 6", "{\"a\":{\"x\":6,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"x\":6,\"k\":0}}\n"),
    (TWO, ".a.x = 3 | .item.x = 4", "{\"a\":{\"x\":4,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"x\":4,\"k\":0}}\n"),
    (TWO, ".item = {\"q\": 1} | .item.x = 5", "{\"a\":{\"x\":1,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"q\":1,\"x\":5}}\n"),
    (TWO, "setpath([\"item\",\"x\"]; 8)", "{\"a\":{\"x\":8,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"x\":8,\"k\":0}}\n"),
    (ONE, ".item.n.p = 9", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":9,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":9,\"q\":2},\"k\":0},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":9,\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":9,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".item.n |= . + {\"r\": 3}", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2,\"r\":3}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2,\"r\":3},\"k\":0},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2,\"r\":3},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2,\"r\":3},\"i\":1}]}\n"),
    (ONE, "del(.item.n.p)", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"q\":2},\"k\":0},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list[0].x = 5", "{\"base\":{\"x\":5,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":5,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":5,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":5,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list[].x = 5", "{\"base\":{\"x\":5,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":5,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":5,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":5,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".list[1].x |= . + 1", "{\"base\":{\"x\":2,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":2,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":2,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":2,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, "del(.item.y)", "{\"base\":{\"x\":1,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":1,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":1,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":1,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".item.n.p = 9 | .item.x = 3", "{\"base\":{\"x\":3,\"y\":2,\"n\":{\"p\":9,\"q\":2}},\"item\":{\"x\":3,\"y\":2,\"n\":{\"p\":9,\"q\":2},\"k\":0},\"list\":[{\"x\":3,\"y\":2,\"n\":{\"p\":9,\"q\":2},\"i\":0},{\"x\":3,\"y\":2,\"n\":{\"p\":9,\"q\":2},\"i\":1}]}\n"),
    (ONE, "(.item.x, .item.y) = 0", "{\"base\":{\"x\":0,\"y\":0,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":0,\"y\":0,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":0,\"y\":0,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":0,\"y\":0,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".item.n = {\"z\": 1} | .item.n.z = 2", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"z\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"z\":2},\"k\":0},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"z\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"z\":2},\"i\":1}]}\n"),
    (ONE, "del(.base.n)", "{\"base\":{\"x\":1,\"y\":2},\"item\":{\"x\":1,\"y\":2,\"k\":0},\"list\":[{\"x\":1,\"y\":2,\"i\":0},{\"x\":1,\"y\":2,\"i\":1}]}\n"),
    (ONE, ".base.n.p = 100", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":100,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":100,\"q\":2},\"k\":0},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":100,\"q\":2},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":100,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".item.n.p = 9 | .list[0].n.q = 8", "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":9,\"q\":8}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":9,\"q\":8},\"k\":0},\"list\":[{\"x\":1,\"y\":2,\"n\":{\"p\":9,\"q\":8},\"i\":0},{\"x\":1,\"y\":2,\"n\":{\"p\":9,\"q\":8},\"i\":1}]}\n"),
    (ONE, "del(.list[0].x)", "{\"base\":{\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
    (ONE, ".item.x as $v | .list[0].x = 3", "{\"base\":{\"x\":3,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":3,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":3,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":3,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n"),
];

#[test]
fn test_a_write_through_a_merged_member_reaches_its_source_6785() -> Result<()> {
    for (document, filter, want) in ROWS {
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
fn test_the_writes_that_still_change_only_the_mappings_copy_6785() -> Result<()> {
    // Not yq's answers, recorded in docs/compliance/yq/limitations.md: a write whose body runs on
    // the merging mapping with a path of its own (`|= (.x = 7)`, `with(...)`) has no position to
    // redirect from, so it changes the mapping's copy and leaves the source alone; and a key the
    // source loses is placed where the mapping listed it, where yq re-renders it last.
    for (document, filter, want) in [
        (
            ONE,
            ".list[] |= (.x = 7)",
            "{\"base\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2}},\"item\":{\"x\":1,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"k\":0},\"list\":[{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":0},{\"x\":7,\"y\":2,\"n\":{\"p\":1,\"q\":2},\"i\":1}]}\n",
        ),
        (
            TWO,
            "del(.b.z)",
            "{\"a\":{\"x\":1,\"z\":9},\"b\":{\"y\":2},\"item\":{\"y\":2,\"z\":9,\"x\":1,\"k\":0}}\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, document, JSON)?;
        assert_eq!(
            (stdout.as_str(), code),
            (want, 0),
            "{filter} on {document:?}"
        );
    }
    Ok(())
}
