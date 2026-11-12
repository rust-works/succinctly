//! #7069: yq's `...` is `..` that also yields each mapping key before its value, so `[...] | length` counts the
//! keys too (7 over `a:` / `  b: thing` / `  c: something`, where `[..] | length` is 4). It was a parse error
//! outside a metadata assignment (`... comments = ""`, #2796). Each row is a live yq v4.53.3 capture
//! (`yq -o=json -I0 FILTER`).

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
