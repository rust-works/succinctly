//! #7069: yq's `...` is `..` that also yields each mapping key before its value, so `[...] | length` counts the
//! keys too (7 over `a:` / `  b: thing` / `  c: something`, where `[..] | length` is 4). It was a parse error
//! outside a metadata assignment (`... comments = ""`, #2796). Each row is a live yq v4.53.3 capture
//! (`yq -o=json -I0 FILTER`). The tests from `META_ROWS` on are the second slice: a metadata assignment in the
//! body of `with(PATH; ...)` or `PATH |= (...)`, which is a stage of the filter with `PATH` in front of its
//! target (YAML output, `yq FILTER`).

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin};

/// `(document, filter, yq's output)`.
const ROWS: &[(&str, &str, &str)] = &[
    ("a:\n  b: thing\n  c: something\n", "[...] | length", "7\n"),
    ("a:\n  b: thing\n  c: something\n", "[...]", "[{\"a\":{\"b\":\"thing\",\"c\":\"something\"}},\"a\",{\"b\":\"thing\",\"c\":\"something\"},\"b\",\"thing\",\"c\",\"something\"]\n"),
    ("a:\n  b: thing\n  c: something\n", "[... | tag]", "[\"!!map\",\"!!str\",\"!!map\",\"!!str\",\"!!str\",\"!!str\",\"!!str\"]\n"),
    ("a:\n  b: thing\n  c: something\n", "... | select(tag == \"!!str\")", "\"a\"\n\"b\"\n\"thing\"\n\"c\"\n\"something\"\n"),
    ("a:\n  b: thing\n  c: something\n", "[... | select(kind == \"scalar\")] | length", "5\n"),
    ("a:\n  b: thing\n  c: something\n", "[...] | map(select(. != null)) | length", "7\n"),
    ("a:\n  b: thing\n  c: something\n", ".a | [...]", "[{\"b\":\"thing\",\"c\":\"something\"},\"b\",\"thing\",\"c\",\"something\"]\n"),
    ("a:\n  b: thing\n  c: something\n", "map(...)", "[{\"b\":\"thing\",\"c\":\"something\"},\"b\",\"thing\",\"c\",\"something\"]\n"),
    ("a:\n  b: thing\n  c: something\n", "[...] | .[1]", "\"a\"\n"),
    ("a:\n  b: thing\n  c: something\n", "[... | select(. == \"b\")]", "[\"b\"]\n"),
    ("a:\n  b: thing\n  c: something\n", "[... | select(tag == \"!!int\")] | length", "0\n"),
    ("a: [1, {x: 2}]\nb: ~\n", "[...] | length", "9\n"),
    ("a: [1, {x: 2}]\nb: ~\n", "[...]", "[{\"a\":[1,{\"x\":2}],\"b\":null},\"a\",[1,{\"x\":2}],1,{\"x\":2},\"x\",2,\"b\",null]\n"),
    ("a: [1, {x: 2}]\nb: ~\n", "[... | tag]", "[\"!!map\",\"!!str\",\"!!seq\",\"!!int\",\"!!map\",\"!!str\",\"!!int\",\"!!str\",\"!!null\"]\n"),
    ("a: [1, {x: 2}]\nb: ~\n", "... | select(tag == \"!!str\")", "\"a\"\n\"x\"\n\"b\"\n"),
    ("a: [1, {x: 2}]\nb: ~\n", "[... | select(kind == \"scalar\")] | length", "6\n"),
    ("a: [1, {x: 2}]\nb: ~\n", "[...] | map(select(. != null)) | length", "8\n"),
    ("a: [1, {x: 2}]\nb: ~\n", ".a | [...]", "[[1,{\"x\":2}],1,{\"x\":2},\"x\",2]\n"),
    ("a: [1, {x: 2}]\nb: ~\n", "map(...)", "[[1,{\"x\":2}],1,{\"x\":2},\"x\",2,null]\n"),
    ("a: [1, {x: 2}]\nb: ~\n", "[...] | .[1]", "\"a\"\n"),
    ("a: [1, {x: 2}]\nb: ~\n", "[... | select(. == \"b\")]", "[\"b\"]\n"),
    ("a: [1, {x: 2}]\nb: ~\n", "[... | select(tag == \"!!int\")] | length", "2\n"),
    ("- k: 1\n- [x, y]\n- z\n", "[...] | length", "8\n"),
    ("- k: 1\n- [x, y]\n- z\n", "[...]", "[[{\"k\":1},[\"x\",\"y\"],\"z\"],{\"k\":1},\"k\",1,[\"x\",\"y\"],\"x\",\"y\",\"z\"]\n"),
    ("- k: 1\n- [x, y]\n- z\n", "[... | tag]", "[\"!!seq\",\"!!map\",\"!!str\",\"!!int\",\"!!seq\",\"!!str\",\"!!str\",\"!!str\"]\n"),
    ("- k: 1\n- [x, y]\n- z\n", "... | select(tag == \"!!str\")", "\"k\"\n\"x\"\n\"y\"\n\"z\"\n"),
    ("- k: 1\n- [x, y]\n- z\n", "[... | select(kind == \"scalar\")] | length", "5\n"),
    ("- k: 1\n- [x, y]\n- z\n", "[...] | map(select(. != null)) | length", "8\n"),
    ("- k: 1\n- [x, y]\n- z\n", "map(...)", "[{\"k\":1},\"k\",1,[\"x\",\"y\"],\"x\",\"y\",\"z\"]\n"),
    ("- k: 1\n- [x, y]\n- z\n", "[...] | .[1]", "{\"k\":1}\n"),
    ("- k: 1\n- [x, y]\n- z\n", "[... | select(. == \"b\")]", "[]\n"),
    ("- k: 1\n- [x, y]\n- z\n", "[... | select(tag == \"!!int\")] | length", "1\n"),
    ("a: {}\nb: []\nc: [{}]\n", "[...]", "[{\"a\":{},\"b\":[],\"c\":[{}]},\"a\",{},\"b\",[],\"c\",[{}],{}]\n"),
    ("a: {}\nb: []\nc: [{}]\n", "[...] | length", "8\n"),
    ("a: {}\nb: []\nc: [{}]\n", "[... | tag]", "[\"!!map\",\"!!str\",\"!!map\",\"!!str\",\"!!seq\",\"!!str\",\"!!seq\",\"!!map\"]\n"),
    ("a: 1\n---\nb: [2]\n", "[...]", "[{\"a\":1},\"a\",1]\n[{\"b\":[2]},\"b\",[2],2]\n"),
    ("a: 1\n---\nb: [2]\n", "[...] | length", "3\n4\n"),
    ("a: 1\n---\nb: [2]\n", "[... | tag]", "[\"!!map\",\"!!str\",\"!!int\"]\n[\"!!map\",\"!!str\",\"!!seq\",\"!!int\"]\n"),
    ("# h\na: 1 # l\n# f\n", "[...]", "[{\"a\":1},\"a\",1]\n"),
    ("# h\na: 1 # l\n# f\n", "[...] | length", "3\n"),
    ("# h\na: 1 # l\n# f\n", "[... | tag]", "[\"!!map\",\"!!str\",\"!!int\"]\n"),
];

#[test]
fn dots_walks_the_nodes_and_the_keys_of_mappings_7069() -> Result<()> {
    for (document, filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, document, &["-o=json", "-I0"])?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, 0),
            "{filter} over {document:?}"
        );
    }
    Ok(())
}

/// The clearing write keeps its own reading of `...`, and jq has no `...` at all.
#[test]
fn dots_as_a_comment_target_and_in_jq_keep_their_readings_7069() -> Result<()> {
    let (out, code) = run_yq_stdin("... comments = \"\"", "# h\na: 1 # l\n# m\nb: 2\n", &[])?;
    assert_eq!((out.as_str(), code), ("a: 1\nb: 2\n", 0));
    let (_, code) = run_jq_stdin("...", "{}", &[])?;
    assert_ne!(code, 0, "jq has no `...`");
    Ok(())
}

/// #7069 (the slice of the 2026-11-18 triage): a metadata assignment in the body of `with(PATH; ...)` or
/// `PATH |= (...)` is a stage of the filter with `PATH` in front of its target, in the same order, so
/// `with(.a.b; . = "new" | . style="double")` is `.a.b = "new" | .a.b style="double"`. Both spellings
/// were refused. Each row is a live yq v4.53.3 capture (`yq FILTER`, YAML output).
const META_ROWS: &[(&str, &str, &str)] = &[
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.b; . = \"new\" | . style=\"double\")",
        "a:\n  b: \"new\"\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.deeply.nested; . = \"newValue\" | . style=\"single\")",
        "a:\n  b: thing\n  c: something\n  deeply:\n    nested: 'newValue'\n",
    ),
    (
        "a:\n  deeply:\n    nested: v\n",
        "with(.a.deeply.nested; . = \"newValue\" | . style=\"single\")",
        "a:\n  deeply:\n    nested: 'newValue'\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a; .b line_comment=\"x\")",
        "a:\n  b: thing # x\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.b; . style=\"double\" | . = \"new\")",
        "a:\n  b: \"new\"\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.b; . style=\"double\")",
        "a:\n  b: \"thing\"\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.b; . line_comment=\"x\")",
        "a:\n  b: thing # x\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.b; . = \"n\" | . line_comment=\"x\")",
        "a:\n  b: n # x\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a; .b style=\"double\" | .c style=\"single\")",
        "a:\n  b: \"thing\"\n  c: 'something'\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a; .b = \"n\" | .b style=\"double\")",
        "a:\n  b: \"n\"\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.b; . = {\"x\": 1} | .x style=\"double\")",
        "a:\n  b:\n    x: \"1\"\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.b; . = [1, 2] | . style=\"flow\")",
        "a:\n  b: [1, 2]\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.b; . = 5 | . style=\"single\")",
        "a:\n  b: '5'\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.b; . anchor=\"z\")",
        "a:\n  b: &z thing\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.b; . tag=\"!!str\")",
        "a:\n  b: thing\n  c: something\n",
    ),
    (
        "a:\n  b: thing # c1\n  c: [1, 2]\nd: 4\n",
        "with(.a.b; . = \"new\" | . style=\"double\")",
        "a:\n  b: \"new\" # c1\n  c: [1, 2]\nd: 4\n",
    ),
    (
        "a:\n  b: thing # c1\n  c: [1, 2]\nd: 4\n",
        "with(.a.c; . style=\"double\")",
        "a:\n  b: thing # c1\n  c:\n    - 1\n    - 2\nd: 4\n",
    ),
    (
        "a:\n  b: thing # c1\n  c: [1, 2]\nd: 4\n",
        "with(.d; . line_comment=\"y\")",
        "a:\n  b: thing # c1\n  c: [1, 2]\nd: 4 # y\n",
    ),
    (
        "l:\n  - x\n  - y\n",
        "with(.l[0]; . = \"new\" | . style=\"double\")",
        "l:\n  - \"new\"\n  - y\n",
    ),
    (
        "l:\n  - x\n  - y\n",
        "with(.l[]; . style=\"double\")",
        "l:\n  - \"x\"\n  - \"y\"\n",
    ),
    (
        "l:\n  - x\n  - y\n",
        "with(.l[]; . = \"n\" | . style=\"single\")",
        "l:\n  - 'n'\n  - 'n'\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.z; . = \"new\" | . style=\"double\")",
        "a:\n  b: thing\n  c: something\nz: \"new\"\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.z; . = 5 | . line_comment=\"five\")",
        "a:\n  b: thing\n  c: something\n  z: 5 # five\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        "with(.a.b?; . style=\"double\")",
        "a:\n  b: \"thing\"\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        ".a.b |= (. = \"new\" | . style=\"double\")",
        "a:\n  b: \"new\"\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        ".a |= (.b line_comment=\"x\")",
        "a:\n  b: thing # x\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        ".a.b |= (. style=\"double\" | . = \"new\")",
        "a:\n  b: \"new\"\n  c: something\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        ".a.b |= . style=\"double\"",
        "a:\n  b: \"thing\"\n  c: something\n",
    ),
    (
        "l:\n  - x\n  - y\n",
        ".l[] |= (. = \"n\" | . style=\"single\")",
        "l:\n  - 'n'\n  - 'n'\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        ".a[] |= (. style=\"double\")",
        "a:\n  b: \"thing\"\n  c: \"something\"\n",
    ),
    (
        "a:\n  b: thing\n  c: something\n",
        ".[] |= (.b style=\"single\" | .c line_comment=\"x\")",
        "a:\n  b: 'thing'\n  c: something # x\n",
    ),
];

#[test]
fn a_metadata_assignment_inside_with_or_an_update_body_writes_at_the_path_7069() -> Result<()> {
    for &(doc, filter, want) in META_ROWS {
        let (out, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter} on {doc:?}");
    }
    Ok(())
}

/// A body that is not stages of literals stays refused: a right side reads the node the body runs on,
/// so it would read another one with `PATH` in front of it, and a `del` is not an assignment of this
/// kind. yq answers these; they are listed in `limitations.md`.
#[test]
fn a_body_that_reads_the_node_or_deletes_stays_refused_7069() -> Result<()> {
    for (doc, filter) in [
        (
            "a:\n  b: thing\n  c: something\n",
            "with(.a; .b line_comment = .c)",
        ),
        (
            "a:\n  b: thing\n  c: something\n",
            ".a |= (. = .b | . style=\"double\")",
        ),
        (
            "a:\n  b: thing\n  c: something\n",
            "with(.a; del(.b) | . style=\"double\")",
        ),
        (
            "a:\n  b: thing\n  c: something\n",
            "with(.a; . style=\"double\" | .c = .b)",
        ),
    ] {
        let (out, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}

/// The same writes as a stage among others: behind a variable binding, a `select`, a parenthesis,
/// another write, and as the first of two (every row is yq's output).
const META_CONTEXT_ROWS: &[(&str, &str, &str)] = &[
    (
        "- a: x\n- a: y\n",
        ". as $v | with(.[0]; .a style=\"double\")",
        "- a: \"x\"\n- a: y\n",
    ),
    (
        "- a: x\n- a: y\n",
        "select(true) | with(.[]; .a style=\"double\")",
        "- a: \"x\"\n- a: \"y\"\n",
    ),
    (
        "- a: x\n- a: y\n",
        "with(.[0]; .a style=\"double\") | .[0]",
        "a: \"x\"\n",
    ),
    (
        "- a: x\n- a: y\n",
        "(with(.[]; .a style=\"double\"))",
        "- a: \"x\"\n- a: \"y\"\n",
    ),
    (
        "- a: x\n- a: y\n",
        "with(.[1]; with(.[]; .a style=\"double\"))",
        "- a: x\n- a: y\n",
    ),
    (
        "- a: x\n- a: y\n",
        ".[0] |= (.a style=\"double\") | .[0] |= (.a style=\"double\")",
        "- a: \"x\"\n- a: y\n",
    ),
    (
        "- a: x\n- a: y\n",
        "with(.[0]; .a style=\"double\") | with(.[0]; .a style=\"double\")",
        "- a: \"x\"\n- a: y\n",
    ),
    (
        "- a: x\n- a: y\n",
        ". as $v | .[0] |= (.a style=\"double\")",
        "- a: \"x\"\n- a: y\n",
    ),
];

#[test]
fn a_metadata_assignment_in_a_with_body_works_as_a_stage_among_others_7069() -> Result<()> {
    for &(doc, filter, want) in META_CONTEXT_ROWS {
        let (out, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter} on {doc:?}");
    }
    Ok(())
}

/// Behind a navigation that is not a path to one node the write is refused (#2679), where the build
/// before the rewrite printed the input with an extra `0: null` entry for the same filters.
#[test]
fn a_metadata_assignment_in_a_with_body_behind_a_navigation_is_refused_7069() -> Result<()> {
    for (doc, filter) in [
        ("- a: x\n- a: y\n", ".[] | with(.[0]; .a style=\"double\")"),
        ("- a: x\n- a: y\n", ".. | .[0] |= (.a style=\"double\")"),
    ] {
        let (out, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}
