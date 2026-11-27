//! #8034: yq's empty-stage rule (#6943) in a `map(f)` whose body reads path context. A literal, a
//! constructor or `error(...)` after a stage that matched nothing still runs once
//! (`map(select(false) | 3)` is `[3]`), and that held here unless `f` read `key`, `path` or
//! `parent`: the parser spells such a `map` as a collect over the members, and the route that
//! evaluates that collect at each member's position answered nothing (`[]`).
//!
//! The candidate's own path reads are of the empty context, so they drop out of what it emits
//! (`select(false) | [key]` is `[]`), on the right side of a write and in `map_values` and
//! `with_entries` too (#8002 declined those). A read *after* the candidate still declines: the
//! node it emits has no position (`select(false) | 3 | key` is nothing).
//!
//! Every row was captured from yq v4.53.3. `succinctly jq` is unchanged.

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin, run_yq_stdin_with_stderr};

const DOC: &str = "b: {x: 1, y: 2}\ns: [5, 6]\nz: {}\ne: []\nn: null\nk: 3\n";

/// The rest of [`DOC`] after `.b`, for the write rows.
const REST: &str = "\"s\":[5,6],\"z\":{},\"e\":[],\"n\":null,\"k\":3";

/// A path-reading body whose prefix matched no member re-emits its constant, as the same body
/// that reads no path context does.
#[test]
fn a_path_reading_map_body_re_emits_after_an_empty_stage_8034() -> Result<()> {
    for (filter, expected) in [
        (".b | map(select(key == 5) | 3)", "[3]"),
        (".b | map(select(false) | 3)", "[3]"),
        ("[.b[] | select(key == 5) | 3]", "[3]"),
        (".b | map(select(path[0] == \"z\") | 3)", "[3]"),
        (".b | map(select(path | length == 9) | 3)", "[3]"),
        (".b | map(select(parent == null) | 3)", "[3]"),
        (".b | map(select(parent.k == 9) | 3)", "[3]"),
        (".s | map(select(key == 5) | 3)", "[3]"),
        (".b | map((select(key == 5) | 3))", "[3]"),
        (".b | map(select(key == 5) | (3))", "[3]"),
        (".b | map(select(key == 5) | {\"a\": 1})", "[{\"a\":1}]"),
        (".b | map(select(key == 5) | 3 | . + 1)", "[4]"),
        (".b | map(select(key == 5) | 3) | length", "1"),
        // A prefix that matched is left as it answered.
        (".b | map(select(key == \"x\") | 3)", "[3]"),
        (".b | map(select(key == \"x\") | key)", "[\"x\"]"),
        (".s | map(select(key == 0) | 3)", "[3]"),
        (".s | map(select(key == 0) | key)", "[0]"),
        (".b | map(select(key == 5))", "[]"),
        // A node with no members never runs the body.
        (".z | map(select(key == 5) | 3)", "{}"),
        (".e | map(select(key == 5) | 3)", "[]"),
        (".n | map(select(key == 5) | 3)", "[]"),
        (".k | map(select(key == 5) | 3)", "3"),
        (".b | map_values(select(key == 5) | 3)", "{\"x\":3,\"y\":3}"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, format!("{expected}\n"), "{filter}");
    }
    Ok(())
}

/// The re-emitted node has no position: a path read after it answers nothing, and one the
/// candidate keeps (inside an operand's pipe) declines the rule.
#[test]
fn a_path_read_after_the_candidate_does_not_re_emit_8034() -> Result<()> {
    for (filter, expected) in [
        (".b | map(select(key == 5) | key)", "[]"),
        (".b | map(select(key == 5) | 3 | key)", "[]"),
        (".b | map(select(key == 5) | [1] | key)", "[]"),
        (".b | map(select(key == 5) | {\"a\": key})", "[]"),
        (".b | map(select(key == 5) | (1 | key) + 2)", "[]"),
        (".b | map(select(false) | 3 | key)", "[]"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, format!("{expected}\n"), "{filter}");
    }
    let filter = ".b |= (select(false) | (1 | key) + 2)";
    let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
    assert_eq!(code, 0, "{filter}");
    assert_eq!(
        stdout,
        format!("{{\"b\":{{\"x\":1,\"y\":2}},{REST}}}\n"),
        "{filter}"
    );
    Ok(())
}

/// A postfix index is part of the term in yq, so a path-reading collect indexed in place answers
/// nothing after an empty stage (`[key][0]` is not `[] | .[0]`). Without the index, or with a
/// stage that is not a navigation after it, the collect still re-emits.
#[test]
fn a_postfix_index_on_a_path_reading_candidate_does_not_re_emit_8034() -> Result<()> {
    for (filter, expected) in [
        (".b | map(select(key == \"zz\") | [key][0])", "[]"),
        (".b | map(select(key == \"zz\") | [key][])", "[]"),
        (
            ".b | map_values(select(key == \"zz\") | [key][0])",
            "{\"x\":1,\"y\":2}",
        ),
        (".b | map(select(key == \"zz\") | [key])", "[[]]"),
        (".b | map(select(key == \"zz\") | [key] | length)", "[0]"),
        (".b | map(select(key == \"zz\") | (1, key) | . + 1)", "[2]"),
        (
            ".b | map(select(key == \"zz\") | {\"a\": [key]})",
            "[{\"a\":[]}]",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, format!("{expected}\n"), "{filter}");
    }
    let filter = ".[] |= (select(. > 3) | [key][0])";
    let (stdout, code) = run_yq_stdin(filter, "{\"a\":2,\"b\":5}", &["-p=json", "-o=json", "-I0"])?;
    assert_eq!(code, 0, "{filter}");
    assert_eq!(stdout, "{\"a\":2,\"b\":\"b\"}\n", "{filter}");
    Ok(())
}

/// A candidate's own path reads are of the empty context, and drop out of what it emits: a
/// collect is `[]` and a comma keeps its constant branches, in `map` and on every #8002 site.
#[test]
fn a_candidate_that_reads_path_context_still_re_emits_8034() -> Result<()> {
    for (filter, expected) in [
        (".b | map(select(key == 5) | [key])", "[[]]".to_string()),
        (".b | map(select(key == 5) | [path])", "[[]]".to_string()),
        (".b | map(select(key == 5) | [parent])", "[[]]".to_string()),
        (".s | map(select(key == 5) | [key])", "[[]]".to_string()),
        (".b | map(select(false) | [key])", "[[]]".to_string()),
        (".b | map(select(key == 5) | (1, key))", "[1]".to_string()),
        (
            ".s | map(select(parent == null) | (1, path))",
            "[1]".to_string(),
        ),
        (
            ".b | map_values(select(false) | [key])",
            "{\"x\":[],\"y\":[]}".to_string(),
        ),
        (
            ".b | map_values(select(key == 5) | [key])",
            "{\"x\":[],\"y\":[]}".to_string(),
        ),
        (
            ".b | with_entries(select(false) | {\"key\": \"q\", \"value\": [key]})",
            "{\"q\":[],\"q\":[]}".to_string(),
        ),
        (
            ".b |= (select(false) | [key])",
            format!("{{\"b\":[],{REST}}}"),
        ),
        (
            ".b[] |= (select(false) | [key])",
            format!("{{\"b\":{{\"x\":[],\"y\":[]}},{REST}}}"),
        ),
        (
            ".c = (.b | select(false) | [key])",
            format!("{{\"b\":{{\"x\":1,\"y\":2}},{REST},\"c\":[]}}"),
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, format!("{expected}\n"), "{filter}");
    }
    Ok(())
}

/// The map on a write's right side, an update of the map's own node, and an `error` after the
/// empty stage, which raises.
#[test]
fn the_rule_holds_in_writes_and_raises_an_error_8034() -> Result<()> {
    for (filter, expected) in [
        (
            ".c = (.b | map(select(key == 5) | 3))",
            format!("{{\"b\":{{\"x\":1,\"y\":2}},{REST},\"c\":[3]}}"),
        ),
        (
            ".c = (.b | map(select(key == \"x\") | 3))",
            format!("{{\"b\":{{\"x\":1,\"y\":2}},{REST},\"c\":[3]}}"),
        ),
        (
            ".b |= map(select(key == 5) | 3)",
            format!("{{\"b\":[3],{REST}}}"),
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, format!("{expected}\n"), "{filter}");
    }
    let filter = ".b | map(select(key == 5) | error(\"boom\"))";
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, DOC, &["-o=json", "-I0"])?;
    assert_ne!(code, 0, "{filter}");
    assert_eq!(stdout, "", "{filter}");
    assert_eq!(stderr, "Error: boom\n", "{filter}");
    Ok(())
}

/// The document, `--eval-all` and `-n` routes agree.
#[test]
fn the_rule_holds_under_eval_all_and_null_input_8034() -> Result<()> {
    let multi = "b: {x: 1}\n---\nb: {y: 2, z: 3}\n";
    for args in [&["-o=json", "-I0"][..], &["ea", "-o=json", "-I0"][..]] {
        for (filter, expected) in [
            (".b | map(select(key == 5) | 3)", "[3]\n[3]\n"),
            (".b | map(select(key == \"y\") | 3)", "[3]\n[3]\n"),
            (".b | map(select(key == 5) | [key])", "[[]]\n[[]]\n"),
        ] {
            let (stdout, code) = run_yq_stdin(filter, multi, args)?;
            assert_eq!(code, 0, "{args:?} {filter}");
            assert_eq!(stdout, expected, "{args:?} {filter}");
        }
    }
    for (filter, expected) in [
        (
            "{\"b\": {\"x\": 1}} | .b | map(select(key == 5) | 3)",
            "[3]\n",
        ),
        ("[1, 2] | map(select(key == 5) | 3)", "[3]\n"),
        (
            "{\"b\": {\"x\": 1}} | .b | map(select(key == 5) | [key])",
            "[[]]\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "", &["-n", "-o=json", "-I0"])?;
        assert_eq!(code, 0, "-n {filter}");
        assert_eq!(stdout, expected, "-n {filter}");
    }
    Ok(())
}

/// `succinctly jq` keeps jq's pipe, which loops zero times over an empty stage.
#[test]
fn jq_mode_is_unchanged_8034() -> Result<()> {
    for (filter, expected) in [
        ("map(select(. > 5) | 3)", "[]\n"),
        ("map(select(false) | [1])", "[]\n"),
        (".[] |= (select(false) | [1])", "[]\n"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, "[1,2]", &["-c"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, expected, "{filter}");
    }
    Ok(())
}
