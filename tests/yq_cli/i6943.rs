//! #6943: a stage that builds a node of its own (a literal, a constructor, an operator over literals,
//! `error("x")`) is emitted once when the stage before it matched nothing, as yq's `valueOperator` does
//! when its context holds no node: `.a[] | 1` over `a: []` is `1`, and so is `select(false) | 1`. It is
//! the pipe's own context list that is empty, not a node's: `.d[] | select(false) | 1` is one `1`
//! however many `d` holds, and `.d[] | 1 | select(false)` stays empty because the `1` was emitted. A
//! stage after the re-emitted one can re-emit in its turn. `map(f)` is `[.[] | f]` to that rule.
//! `succinctly jq` never does: jq's pipe loops zero times.
//!
//! Every row was captured from yq v4.53.3 (`-o=json -I0`) over `a: []` / `b: {}` / `c: 1` /
//! `d: [{x: 1}]` and kept only when both tools agree. Left out because they still differ: the
//! wording of `.a.b | 1` (an error in both), a re-emission in a write's right side
//! (`.x = (.a[] | 1)`), in `map_values` and in `|=` bodies, and after `to_entries`.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "a: []\nb: {}\nc: 1\nd: [{x: 1}]\n";

/// (filter, yq's stdout).
const ROWS: &[(&str, &str)] = &[
    (".a[] | 1", "1\n"),
    (".b[] | 1", "1\n"),
    (".a[] | \"x\"", "\"x\"\n"),
    (".a[] | [1]", "[]\n"),
    (".a[] | {\"k\":1}", "{\"k\":1}\n"),
    (".a[] | (\"x\", \"y\")", "\"x\"\n\"y\"\n"),
    (".a[] | 1, 2", "1\n2\n"),
    (".a[] | 1 + 1", "2\n"),
    (".a[] | \"a\" + \"b\"", "\"ab\"\n"),
    (".a[] | 1 | . + 1", "2\n"),
    ("select(false) | 1", "1\n"),
    (".zz[] | 1", "1\n"),
    (".zz[]? | 1", "1\n"),
    ("[.a[] | 1]", "[1]\n"),
    ("(.a[] | 1), 2", "1\n2\n"),
    (".a[] | .+1", ""),
    (".a[] | length", ""),
    (".a | map(1)", "[]\n"),
    (".a[] as $x | 1", "1\n"),
    (".c.b | 1", "1\n"),
    (".d[] | select(.x == 2) | 1", "1\n"),
    (".d[] | select(.x == 2) | [1]", "[]\n"),
    (".d[] | select(.x == 2) | {\"k\": 2}", "{\"k\":2}\n"),
    (".d[] | select(.x == 1) | 1", "1\n"),
    (".a[] | 1 | 2", "2\n"),
    (".a[] | 1 | select(false)", ""),
    (".d[] | 1 | select(false)", ""),
    (".d[] | 1 | select(false) | 2", "2\n"),
    (".d[] | select(false) | 1 | select(false) | 2", "2\n"),
    (".a[] | (1 | select(false)) | 2", "2\n"),
    (".a[] | (1, .)", "1\n"),
    (".a[] | -1", "-1\n"),
    (".a[] | 1 // 2", "1\n"),
    (".a[] | 1 == 1", "true\n"),
    (".a[] | not", ""),
    (".a[] | \"x\" | length", "1\n"),
    (".d[] | .x | 5", "5\n"),
    (".d[] | .zz | 5", "5\n"),
    (".d[] | select(false) | 1", "1\n"),
    (".. | select(false) | 1", "1\n"),
    ("map(select(false) | 1)", "[1]\n"),
    ("map(1)", "[1,1,1,1]\n"),
    (".d | map(select(.x == 2) | \"n\")", "[\"n\"]\n"),
    ("[.d[] | select(.x == 2) | \"n\"]", "[\"n\"]\n"),
    (".c | select(false) | 1 | select(false) | 2", "2\n"),
    (".a[] | {}", "{}\n"),
    (".a[] | {\"k\": .c}", ""),
    (".a[] | [.c]", "[]\n"),
];

#[test]
fn a_stage_that_builds_a_node_re_emits_over_an_empty_context_6943() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// The same rule over `--eval-all`'s list of documents, and over JSON input.
#[test]
fn the_rule_holds_for_eval_all_and_json_input_6943() -> Result<()> {
    for (filter, expected) in [
        (".a[] | 1", "1\n"),
        ("[.a[] | 1]", "[1]\n"),
        (".d[] | select(.x == 2) | \"n\"", "\"n\"\n"),
        ("(.a[] | 1), 2", "1\n2\n"),
        (".a[] | (1, 2)", "1\n2\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0", "--eval-all"])?;
        assert_eq!(code, 0, "--eval-all {filter}");
        assert_eq!(stdout, expected, "--eval-all {filter}");
        let json = "{\"a\":[],\"b\":{},\"c\":1,\"d\":[{\"x\":1}]}";
        let (stdout, code) = run_yq_stdin(filter, json, &["-p=json", "-o=json", "-I0"])?;
        assert_eq!(code, 0, "-p=json {filter}");
        assert_eq!(stdout, expected, "-p=json {filter}");
    }
    Ok(())
}
