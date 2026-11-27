//! #8044: yq's empty-stage rule (#6943) through a binding or a fold. yq's `variableLoopSingleChild`
//! (`pkg/yqlib/operator_variables.go`, v4.53.3) evaluates a binding's source over the context and
//! then its body over that same context, once per source output with the variable bound, or once
//! with nothing bound when the source yields nothing; `reduceOperator` (`operator_reduce.go`) is its
//! INIT over the context when its source yields nothing. So over an empty context a binding or a
//! fold is asked as a stage of its own: `select(false) | (. as $y | 3)` is `3`, and
//! `select(false) | (.[] as $x ireduce (0; . + 1))` is `0`. Both used to print nothing here.
//!
//! Every row was captured from yq v4.53.3, as `yq -o=json -I0 FILTER` on `[1, 2]` unless it says
//! otherwise. `succinctly jq` is unchanged: jq's pipe loops zero times over an empty stage.

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin_with_stderr};

const INPUT: &str = "[1, 2]\n";
const JSON: &[&str] = &["-o", "json", "-I0"];

fn assert_rows(input: &str, rows: &[(&str, &str)]) -> Result<()> {
    for (filter, expected) in rows {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, input, JSON)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (*expected, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// A binding after a stage that matched nothing.
const BINDING_ROWS: &[(&str, &str)] = &[
    // The source yields nothing, so the body runs once with nothing bound.
    ("select(false) | (. as $y | 3)", "3\n"),
    ("select(false) | (. as $y | [.])", "[]\n"),
    ("select(false) | (. as $y | [$y])", "[]\n"),
    ("select(false) | (. as $y | (3, .))", "3\n"),
    ("select(false) | (. as $y | 3 | . + 1)", "4\n"),
    ("select(false) | (. as $y | 3) | . + 1", "4\n"),
    ("select(false) | . as $y | 3", "3\n"),
    ("select(false) | (. as $a | . as $b | 3)", "3\n"),
    ("select(false) | (. as $y | select(false) | 3)", "3\n"),
    ("select(false) | (. as $y | \"a\" as $z | $z)", "\"a\"\n"),
    ("select(false) | ({\"a\": .} as $y | 3)", "3\n"),
    (".[] | select(false) | (. as $y | 3)", "3\n"),
    // The variable is unbound, and what reads the empty context answers nothing.
    ("select(false) | (. as $y | $y)", ""),
    ("select(false) | (. as $y | $y + 1)", ""),
    ("select(false) | (. as $y | .)", ""),
    ("select(false) | ({\"a\": .} as $y | $y)", ""),
    ("select(false) | (3 as $y | .)", ""),
    ("select(false) | (. as $y | 3 | key)", ""),
    // A source that re-emits is bound, once per output, and the variable is a value.
    ("select(false) | (1 as $y | $y)", "1\n"),
    ("select(false) | (1 as $y | [$y])", "[]\n"),
    ("select(false) | (1 as $y | $y + 1)", "2\n"),
    ("select(false) | (1 as $y | {\"a\": $y})", "{\"a\":1}\n"),
    ("select(false) | (\"x\" as $y | {$y: 1})", "{\"x\":1}\n"),
    ("select(false) | (1 as $y | {$y: 1})", "{\"1\":1}\n"),
    ("select(false) | ({\"a\": 1} as $y | $y.a)", "1\n"),
    ("select(false) | ([.] as $y | $y)", "[]\n"),
    ("select(false) | ((1, 2) as $y | $y)", "1\n2\n"),
    ("select(false) | ((1, 2) as $y | 3)", "3\n3\n"),
    ("select(false) | (1 as $y | . as $z | $y)", "1\n"),
    ("select(false) | (1 as $y | $y | key)", ""),
    // A source that empties on its own leaves the body unbound.
    ("select(false) | ((1 | select(false)) as $y | 3)", "3\n"),
    ("select(false) | ((1 | select(false)) as $y | $y)", ""),
    (
        "select(false) | ((1, (2 | select(false))) as $y | $y)",
        "1\n",
    ),
];

#[test]
fn a_binding_after_an_empty_stage_runs_its_body_8044() -> Result<()> {
    assert_rows(INPUT, BINDING_ROWS)
}

/// A fold after a stage that matched nothing.
const FOLD_ROWS: &[(&str, &str)] = &[
    // The source yields nothing, so the fold is INIT's answer over the empty context.
    ("select(false) | (.[] as $y ireduce (0; . + 1))", "0\n"),
    ("select(false) | (.[] as $x ireduce ([]; . + [$x]))", "[]\n"),
    ("select(false) | (.[] as $x ireduce (.; 1))", ""),
    (
        "select(false) | (.[] as $y ireduce (select(false); [.]))",
        "",
    ),
    (
        "select(false) | (.[] as $x ireduce (0; . + ($x | key)))",
        "0\n",
    ),
    // A source that re-emits is folded, from an empty list when INIT answers nothing (#8035).
    ("select(false) | ((1, 2) as $x ireduce (0; . + $x))", "3\n"),
    (
        "select(false) | ((1, 2) as $x ireduce (select(false); [.]))",
        "[[]]\n",
    ),
    (
        "select(false) | ((1, 2) as $x ireduce (select(false); 5))",
        "5\n",
    ),
    ("select(false) | ((1, 2) as $x ireduce (.; 1))", "1\n"),
];

#[test]
fn a_fold_after_an_empty_stage_is_its_init_8044() -> Result<()> {
    assert_rows(INPUT, FOLD_ROWS)
}

/// Bodies a write, a builtin or a fold evaluates itself (#8002, #8035).
#[test]
fn a_binding_in_a_body_over_an_empty_context_8044() -> Result<()> {
    assert_rows(
        INPUT,
        &[
            ("map(select(false) | (. as $y | 3))", "[3]\n"),
            (".[] |= (select(false) | (. as $y | 3))", "[3,3]\n"),
            (".[0] = (select(false) | (. as $y | 3))", "[3,2]\n"),
            (".[] as $x ireduce (select(false); . as $y | [.])", "[[]]\n"),
            (".[] as $x ireduce (select(false); 1 as $y | $y)", "1\n"),
            (
                ".[] as $x ireduce (0; select(false) | (. as $y | 3))",
                "3\n",
            ),
        ],
    )?;
    assert_rows(
        "[1, 2, 3]\n",
        &[(
            ".[] as $x ireduce (select(false); . as $y | [.])",
            "[[[]]]\n",
        )],
    )?;
    assert_rows(
        "a: 1\nb: [1, 2]\n",
        &[
            (
                ".b[] |= (select(. > 5) | (1 as $v | $v + 1))",
                "{\"a\":1,\"b\":[2,2]}\n",
            ),
            (
                ".c = (.b[] | select(. > 5) | (. as $v | [.]))",
                "{\"a\":1,\"b\":[1,2],\"c\":[]}\n",
            ),
            (
                ".b | map(select(. > 5) | (.[] as $x ireduce (0; . + 1)))",
                "[0]\n",
            ),
            (
                "with_entries(select(false) | (. as $y | {\"key\": \"z\", \"value\": 3}))",
                "{\"z\":3,\"z\":3}\n",
            ),
        ],
    )
}

/// Two documents on stdin: under `--eval-all` the whole list is one context, so the body runs once;
/// without it each document is its own.
#[test]
fn eval_all_binding_and_fold_after_an_empty_stage_8044() -> Result<()> {
    let docs = "a: 1\n---\na: 2\n";
    for (filter, all, each) in [
        ("select(false) | (. as $y | 3)", "3\n", "3\n3\n"),
        ("select(.a > 5) | (1 as $y | $y)", "1\n", "1\n1\n"),
        (
            "select(false) | (.a as $y ireduce (0; . + 1))",
            "0\n",
            "0\n0\n",
        ),
    ] {
        for (args, expected) in [(&["ea", "-o", "json", "-I0"][..], all), (JSON, each)] {
            let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, docs, args)?;
            assert_eq!(
                (stdout.as_str(), stderr.as_str(), code),
                (expected, "", 0),
                "{filter} {args:?}"
            );
        }
    }
    Ok(())
}

/// jq's pipe loops zero times over an empty stage, binding and fold included (jq 1.7.1).
#[test]
fn jq_mode_is_unchanged_8044() -> Result<()> {
    for (filter, expected) in [
        ("select(false) | (. as $y | 3)", ""),
        ("select(false) | (1 as $y | $y)", ""),
        ("select(false) | reduce .[] as $x (0; . + 1)", ""),
        ("[.[] | select(false) | (. as $y | 3)]", "[]\n"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, "[1,2]", &["-c"])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}
