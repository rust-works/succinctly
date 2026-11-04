//! #6130: yq's `select` reads its condition read-only: a path through an absent key is empty, so a
//! pipe that starts at one has nothing to run on and the item is not kept. `succinctly yq` ran the
//! stage on `null` (`select(.zz | not)` kept every item, `select(.t[-1] == 1)` raised an index error
//! for an item with no `t`). Every expected output is what yq v4.53.3 prints (`-o=json -I=0`).

use anyhow::Result;

use crate::run_yq_stdin;

const A: &str = "a: 1\n";
const ARR: &str = "- t: [1]\n  n: 1\n- a: 1\n- t: []\n  n: 0\n- {n: 2, t: [2, 3]}\n";
const MAP: &str = "x: {t: [1]}\ny: {a: 1}\n";

/// (document, filter, expected stdout).
const ORACLE_ROWS: &[(&str, &str, &str)] = &[
    ("A", r"select(.zz | not)", ""),
    ("A", r"select(.zz | contains(null))", ""),
    ("A", r"select(.zz | . == null)", ""),
    ("A", r"select(.zz | length == 0)", ""),
    ("A", r#"select(.zz | tag == "!!null")"#, ""),
    ("A", r#"select(.zz | test("a"))"#, ""),
    ("A", r"select(.zz | @csv)", ""),
    ("A", r"select(.tags[-1])", ""),
    ("A", r"select(.zz == null)", "{\"a\":1}\n"),
    ("A", r#"select(.zz // "d" | . == "d")"#, "{\"a\":1}\n"),
    (
        "ARR",
        r"map(select(.t | length == 0))",
        "[{\"t\":[],\"n\":0}]\n",
    ),
    ("ARR", r"map(select(.t | not))", "[]\n"),
    (
        "ARR",
        r"map(select(.t | contains([1])))",
        "[{\"t\":[1],\"n\":1}]\n",
    ),
    ("ARR", r"[.[] | select(.t | . == null)]", "[]\n"),
    (
        "MAP",
        r"with_entries(select(.value.t[-1] == 1))",
        "{\"x\":{\"t\":[1]}}\n",
    ),
    (
        "ARR",
        r"del(.[] | select(.t | not))",
        "[{\"t\":[1],\"n\":1},{\"a\":1},{\"t\":[],\"n\":0},{\"n\":2,\"t\":[2,3]}]\n",
    ),
    (
        "ARR",
        r"del(.[] | select(.t | contains([1])))",
        "[{\"a\":1},{\"t\":[],\"n\":0},{\"n\":2,\"t\":[2,3]}]\n",
    ),
    (
        "ARR",
        r"(.[] | select(.t | length == 0)) = 9",
        "[{\"t\":[1],\"n\":1},{\"a\":1},9,{\"n\":2,\"t\":[2,3]}]\n",
    ),
    (
        "ARR",
        r"(.[] | select(.zz | not)) |= 9",
        "[{\"t\":[1],\"n\":1},{\"a\":1},{\"t\":[],\"n\":0},{\"n\":2,\"t\":[2,3]}]\n",
    ),
    (
        "ARR",
        r".[] |= select(.t | not)",
        "[{\"t\":[1],\"n\":1},{\"a\":1},{\"t\":[],\"n\":0},{\"n\":2,\"t\":[2,3]}]\n",
    ),
];

#[test]
fn select_reads_its_condition_read_only_6130() -> Result<()> {
    for &(document, filter, expected) in ORACLE_ROWS {
        let document = match document {
            "A" => A,
            "ARR" => ARR,
            _ => MAP,
        };
        let (output, code) = run_yq_stdin(filter, document, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(output, expected, "`{filter}` over {document:?}");
    }
    Ok(())
}

/// The reported shape: an item without `t` is skipped, as yq skips it.
#[test]
fn a_negative_index_on_an_absent_key_in_select_skips_the_item_6130() -> Result<()> {
    let (output, code) = run_yq_stdin(
        ".[] | select(.t[-1] == 1)",
        "- t: [1]\n- a: 1\n",
        &["-o=json", "-I=0"],
    )?;
    assert_eq!((output.as_str(), code), ("{\"t\":[1]}\n", 0));
    Ok(())
}

/// A key that exists but holds an empty sequence is not absent: the index error stays, in `select`
/// as outside it.
#[test]
fn an_empty_sequence_is_not_an_absent_key_6130() -> Result<()> {
    let (_, stderr, code) =
        crate::run_yq_stdin_with_stderr("select(.tags[-1])", "tags: []\n", &[])?;
    assert_eq!(code, 1);
    assert!(
        stderr.contains("index [-1] out of range, array size is 0"),
        "{stderr}"
    );
    Ok(())
}

/// Shapes that reach the other `select` routes: nested, after a bind, after a construction, in a
/// boolean, under `first`, over a computed value (the owned door).
const ROUTE_ROWS: &[(&str, &str)] = &[
    ("[.[] | select(.zz | not)] | length", "0\n"),
    ("map(select(select(.zz | not)))", "[]\n"),
    (".[] as $x | $x | select(.zz | not)", ""),
    ("map({\"v\": .} | select(.v.zz | not))", "[]\n"),
    ("map(select(.a.b | not))", "[]\n"),
    ("[.[] | {\"k\": .} | .k | select(.zz | not)]", "[]\n"),
    ("(. as $r | $r | .[] | select(.zz | not))", ""),
    ("map(. * {\"q\": 1} | select(.zz | not))", "[]\n"),
    ("to_entries | map(select(.value.zz | not))", "[]\n"),
    (
        ".[] | select(.n == 1 or (.zz | not))",
        "{\"t\":[1],\"n\":1}\n",
    ),
    ("map(select((.zz | not) and true))", "[]\n"),
];

#[test]
fn every_select_route_reads_its_condition_read_only_6130() -> Result<()> {
    let document = "- t: [1]\n  n: 1\n- a: 1\n- t: []\n  n: 0\n";
    for &(filter, expected) in ROUTE_ROWS {
        let (output, code) = run_yq_stdin(filter, document, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(output, expected, "`{filter}`");
    }
    Ok(())
}
