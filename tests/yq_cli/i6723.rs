//! #6723: yq names a merged member by where its node lives: with `a: &a {x: 1}` and
//! `item: {<<: *a, k: 0}`, `.item.x | path` is `["a","x"]`, not `["item","x"]`, and `key` and
//! `parent` follow. The path-context walk (`path`, `key`, `parent`, `parents`) starts the trail
//! over at the anchor's location; every other step, and every write, keeps the step it was given.
//! Each row is a live yq v4.53.3 capture (`yq -o=json -I=0`).

use anyhow::Result;

use crate::run_yq_stdin;

const JSON: &[&str] = &["-o=json", "-I=0"];

/// `(document, filter, yq's output)`.
const ROWS: &[(&str, &str, &str)] = &[
    ("a: &a {x: 1, z: 9}\nb: &b {y: 2, z: 8}\nitem: {<<: [*a, *b], k: 0}\n", "[.item | .[] | path]", "[[\"a\",\"x\"],[\"b\",\"z\"],[\"b\",\"y\"],[\"item\",\"k\"]]\n"),
    ("a: &a {x: 1, z: 9}\nb: &b {y: 2, z: 8}\nitem: {<<: [*a, *b], k: 0}\n", "[.item | .x | path]", "[[\"a\",\"x\"]]\n"),
    ("a: &a {x: 1, z: 9}\nb: &b {y: 2, z: 8}\nitem: {<<: [*a, *b], k: 0}\n", "[.item.z | path]", "[[\"b\",\"z\"]]\n"),
    ("a: &a {x: 1, z: 9}\nb: &b {y: 2, z: 8}\nitem: {<<: [*a, *b], k: 0}\n", "[.item | .[] | key]", "[\"x\",\"z\",\"y\",\"k\"]\n"),
    ("a: &a {x: 1, z: 9}\nb: &b {y: 2, z: 8}\nitem: {<<: [*a, *b], k: 0}\n", "[.item | .[] | parent | path]", "[[\"a\"],[\"b\"],[\"b\"],[\"item\"]]\n"),
    ("a: &a {x: 1, z: 9}\nb: &b {y: 2, z: 8}\nitem: {<<: [*a, *b], k: 0}\n", "[.item | .y | parent | path]", "[[\"b\"]]\n"),
    ("base: &base {x: 1, y: 2}\nitem: {<<: *base, k: 0}\n", "[.item | .[] | path]", "[[\"base\",\"x\"],[\"base\",\"y\"],[\"item\",\"k\"]]\n"),
    ("base: &base {x: 1, y: 2}\nitem: {<<: *base, k: 0}\n", "[.item.x | path]", "[[\"base\",\"x\"]]\n"),
    ("a: &a {x: 1}\nb: &b {<<: *a, y: 2}\nitem: {<<: *b, k: 0}\n", "[.item | .y | path]", "[[\"b\",\"y\"]]\n"),
    ("a: &a {x: 1}\nlist:\n  - {<<: *a, k: 0}\n  - {<<: *a, k: 1}\n", "[.list[] | .x | path]", "[[\"a\",\"x\"],[\"a\",\"x\"]]\n"),
    ("a: &a {x: 1}\nlist:\n  - {<<: *a, k: 0}\n  - {<<: *a, k: 1}\n", "[.list[0].x | path]", "[[\"a\",\"x\"]]\n"),
    ("a: &a {x: {p: 1}}\nitem: {<<: *a, k: {r: 2}}\n", "[.item | .[] | path]", "[[\"a\",\"x\"],[\"item\",\"k\"]]\n"),
    ("a: &a {x: {p: 1}}\nitem: {<<: *a, k: {r: 2}}\n", "[.item.x.p | path]", "[[\"a\",\"x\",\"p\"]]\n"),
    ("a: &a {x: 1, k: 5}\nitem: {<<: *a, k: 0}\n", "[.item | .[] | path]", "[[\"a\",\"x\"],[\"item\",\"k\"]]\n"),
    ("a: &a {x: 1, k: 5}\nitem: {<<: *a, k: 0}\n", "[.item.k | path]", "[[\"item\",\"k\"]]\n"),
    ("item: {<<: {q: 1}, k: 0}\n", "[.item | .k | path]", "[[\"item\",\"k\"]]\n"),
    ("a: &a {x: 1, z: 9}\nb: &b {y: 2, z: 8}\nitem: {<<: [*a, *b], k: 0}\n", "[.item | .x | line]", "[1]\n"),
    ("a: &a {x: 1, z: 9}\nb: &b {y: 2, z: 8}\nitem: {<<: [*a, *b], k: 0}\n", ".item.k = 7", "{\"a\":{\"x\":1,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"x\":1,\"k\":7}}\n"),
    ("a: &a {x: 1, z: 9}\nb: &b {y: 2, z: 8}\nitem: {<<: [*a, *b], k: 0}\n", "del(.item.k)", "{\"a\":{\"x\":1,\"z\":9},\"b\":{\"y\":2,\"z\":8},\"item\":{\"y\":2,\"z\":9,\"x\":1}}\n"),
];

#[test]
fn test_a_merged_member_is_named_by_its_anchors_location_6723() -> Result<()> {
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
fn test_the_shapes_that_keep_the_mappings_own_path_6723() -> Result<()> {
    // Not yq's answers, recorded in docs/compliance/yq/limitations.md: a member of an inline
    // `<<: {q: 1}` is `["item","<<","q"]` in yq, and the walk cannot climb through the merge key,
    // so it keeps the mapping's own path; and two merge sources that each merge list a literal
    // `<<` entry here where yq lists the entries behind it (`["a","x"]`).
    for (document, filter, want) in [
        (
            "item: {<<: {q: 1}, k: 0}\n",
            "[.item | .q | path]",
            "[[\"item\",\"q\"]]\n",
        ),
        (
            "a: &a {x: 1}\nb: &b {<<: *a, y: 2}\nitem: {<<: *b, k: 0}\n",
            "[.item | .[] | path]",
            "[[\"item\",\"<<\"],[\"b\",\"y\"],[\"item\",\"k\"]]\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, document, JSON)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter} on {document:?}");
    }
    Ok(())
}
