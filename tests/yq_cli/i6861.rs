//! #6861: a write to the node an update targets (`. = X`, `setpath([]; X)`) replaces the node at that
//! position, so over an alias it rebinds the position and leaves the shared node alone, as in yq; a
//! write below the target (`.p = 9`) still reaches the shared node (#1351). `|=` used to send any
//! body that is a chain of writes to the anchor, `. = X` included, and rewrote it and every alias.
//! Each row is a live yq v4.53.3 capture: `yq -o=json -I=0`, and the default YAML output for the
//! rows that show which position keeps its `*x` mark.

use anyhow::Result;

use crate::run_yq_stdin;

const JSON: &[&str] = &["-o=json", "-I=0"];

/// An anchor, two aliases of it, and a list holding one alias.
const ALIASES: &str = "a: &x {p: 1}\nb: *x\nc: *x\nl: [*x, {p: 3}]\n";

/// `(filter, yq's output)`.
const ROWS: &[(&str, &str)] = &[
    (".b |= (. = {\"p\":3})", "{\"a\":{\"p\":1},\"b\":{\"p\":3},\"c\":{\"p\":1},\"l\":[{\"p\":1},{\"p\":3}]}\n"),
    (".l[] |= (. = {\"p\":3})", "{\"a\":{\"p\":1},\"b\":{\"p\":1},\"c\":{\"p\":1},\"l\":[{\"p\":3},{\"p\":3}]}\n"),
    (".b |= (. = {\"p\":3} | .p = 9)", "{\"a\":{\"p\":1},\"b\":{\"p\":9},\"c\":{\"p\":1},\"l\":[{\"p\":1},{\"p\":3}]}\n"),
    ("with(.b; . = {\"p\":3})", "{\"a\":{\"p\":1},\"b\":{\"p\":3},\"c\":{\"p\":1},\"l\":[{\"p\":1},{\"p\":3}]}\n"),
    (".b |= (. = {\"p\":3}, .p = 9)", "{\"a\":{\"p\":1},\"b\":{\"p\":9},\"c\":{\"p\":1},\"l\":[{\"p\":1},{\"p\":3}]}\n"),
    (".b |= setpath([]; {\"p\":3})", "{\"a\":{\"p\":1},\"b\":{\"p\":3},\"c\":{\"p\":1},\"l\":[{\"p\":1},{\"p\":3}]}\n"),
    (".b |= (. as $v | . = {\"p\":3})", "{\"a\":{\"p\":1},\"b\":{\"p\":3},\"c\":{\"p\":1},\"l\":[{\"p\":1},{\"p\":3}]}\n"),
    (".b |= (.p = 9)", "{\"a\":{\"p\":9},\"b\":{\"p\":9},\"c\":{\"p\":9},\"l\":[{\"p\":9},{\"p\":3}]}\n"),
    (".l[] |= (.p = 7)", "{\"a\":{\"p\":7},\"b\":{\"p\":7},\"c\":{\"p\":7},\"l\":[{\"p\":7},{\"p\":7}]}\n"),
    (".b |= 5", "{\"a\":{\"p\":1},\"b\":5,\"c\":{\"p\":1},\"l\":[{\"p\":1},{\"p\":3}]}\n"),
    (".b = {\"p\":3}", "{\"a\":{\"p\":1},\"b\":{\"p\":3},\"c\":{\"p\":1},\"l\":[{\"p\":1},{\"p\":3}]}\n"),
    (".b |= (.p = 9 | del(.p))", "{\"a\":{},\"b\":{},\"c\":{},\"l\":[{},{\"p\":3}]}\n"),
    (".b |= (.p = 1, .q = 2)", "{\"a\":{\"p\":1,\"q\":2},\"b\":{\"p\":1,\"q\":2},\"c\":{\"p\":1,\"q\":2},\"l\":[{\"p\":1,\"q\":2},{\"p\":3}]}\n"),
    (".b |= setpath([\"p\"]; 9)", "{\"a\":{\"p\":9},\"b\":{\"p\":9},\"c\":{\"p\":9},\"l\":[{\"p\":9},{\"p\":3}]}\n"),
];

/// The same documents, in YAML: the shared node and its aliases keep their marks.
const YAML_ROWS: &[(&str, &str)] = &[
    (
        ".b |= (. = {\"p\":3})",
        "a: &x {p: 1}\nb:\n  p: 3\nc: *x\nl: [*x, {p: 3}]\n",
    ),
    (
        ".l[] |= (. = {\"p\":3})",
        "a: &x {p: 1}\nb: *x\nc: *x\nl: [{p: 3}, {p: 3}]\n",
    ),
    (
        ".b |= setpath([]; {\"p\":3})",
        "a: &x {p: 1}\nb:\n  p: 3\nc: *x\nl: [*x, {p: 3}]\n",
    ),
    (
        ".b |= (.p = 9)",
        "a: &x {p: 9}\nb: *x\nc: *x\nl: [*x, {p: 3}]\n",
    ),
];

/// `(filter, yq's output, this tool's output)`: a body whose write below the target reaches the
/// shared node and whose later write replaces the target. The position is rebound, but the first
/// write stays on the position's own copy (the redirect is one answer per update).
const STILL_DIVERGENT: &[(&str, &str, &str)] = &[
    (".b |= (.p = 9 | . = {\"p\":3} | .p = 1)", "{\"a\":{\"p\":9},\"b\":{\"p\":1},\"c\":{\"p\":9},\"l\":[{\"p\":9},{\"p\":3}]}\n", "{\"a\":{\"p\":1},\"b\":{\"p\":1},\"c\":{\"p\":1},\"l\":[{\"p\":1},{\"p\":3}]}\n"),
    (".l[] |= (.u = 4 | . = {\"p\":3})", "{\"a\":{\"p\":1,\"u\":4},\"b\":{\"p\":1,\"u\":4},\"c\":{\"p\":1,\"u\":4},\"l\":[{\"p\":3},{\"p\":3}]}\n", "{\"a\":{\"p\":1},\"b\":{\"p\":1},\"c\":{\"p\":1},\"l\":[{\"p\":3},{\"p\":3}]}\n"),
    (".b |= (.p = 5 | . = {\"p\":3})", "{\"a\":{\"p\":5},\"b\":{\"p\":3},\"c\":{\"p\":5},\"l\":[{\"p\":5},{\"p\":3}]}\n", "{\"a\":{\"p\":1},\"b\":{\"p\":3},\"c\":{\"p\":1},\"l\":[{\"p\":1},{\"p\":3}]}\n"),
    (".b |= (del(.p) | . = {\"p\":3})", "{\"a\":{},\"b\":{\"p\":3},\"c\":{},\"l\":[{},{\"p\":3}]}\n", "{\"a\":{\"p\":1},\"b\":{\"p\":3},\"c\":{\"p\":1},\"l\":[{\"p\":1},{\"p\":3}]}\n"),
];

#[test]
fn test_a_write_to_the_target_rebinds_an_alias_position_6861() -> Result<()> {
    for (filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, ALIASES, JSON)?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn test_the_yaml_output_keeps_the_marks_of_the_untouched_positions_6861() -> Result<()> {
    for (filter, want) in YAML_ROWS {
        let (stdout, code) = run_yq_stdin(filter, ALIASES, &[])?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn test_the_bodies_that_reach_the_shared_node_and_then_rebind_still_differ_6861() -> Result<()> {
    for (filter, yq, ours) in STILL_DIVERGENT {
        let (stdout, code) = run_yq_stdin(filter, ALIASES, JSON)?;
        assert_eq!((stdout.as_str(), code), (*ours, 0), "{filter}");
        assert_ne!(ours, yq, "{filter} now matches yq: move the row to ROWS");
    }
    Ok(())
}
