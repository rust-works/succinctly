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
    (
        "select(false) | ((1 | select(false)) as $y | ($y, 4))",
        "4\n",
    ),
    (
        "select(false) | ((1, (2 | select(false))) as $y | ($y, 4))",
        "1\n4\n",
    ),
    (
        "select(false) | ((1 | select(false)) as $y | (1 | select(false)) as $z | [$y, $z])",
        "[]\n",
    ),
    // #8034's postfix rule holds inside the body: `[key][0]` is one term in yq.
    ("select(false) | (. as $y | [key][0])", ""),
    ("select(false) | (. as $y | [key][1:])", "[]\n"),
    (".[] |= (select(false) | (. as $y | [key][0]))", "[1,2]\n"),
    // A binding that re-emits nothing leaves `map`'s members to the ordinary walk.
    ("map(select(true) | . as $v | {})", "[{},{}]\n"),
    ("map(select(true) | (1 as $y | [$y]))", "[[1],[1]]\n"),
];

/// A nest of bindings whose sources all yield nothing is one body run, however deep: yq answers in
/// a quarter of a second, and a rewrite that doubled per level would build millions of branches.
#[test]
fn a_deep_nest_of_empty_sources_answers_8044() -> Result<()> {
    let bindings: String = (1..=24)
        .map(|i| format!("(1 | select(false)) as $a{i} | "))
        .collect();
    let filter = format!("select(false) | ({bindings}3)");
    let started = std::time::Instant::now();
    assert_rows(INPUT, &[(filter.as_str(), "3\n")])?;
    assert!(started.elapsed() < std::time::Duration::from_secs(30));
    Ok(())
}

#[test]
fn a_binding_after_an_empty_stage_runs_its_body_8044() -> Result<()> {
    assert_rows(INPUT, BINDING_ROWS)
}

/// Only a copy the rewrite makes counts against its budget: a union of 20,000 literals after an
/// empty stage prints all of them, as in yq, and fifteen nested `//` whose left side empties answer
/// promptly (nothing, in both tools).
#[test]
fn a_long_union_and_a_nest_of_alternatives_answer_8044() -> Result<()> {
    let ones = vec!["1"; 20_000].join(", ");
    let filter = format!("select(false) | ({ones})");
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(&filter, INPUT, JSON)?;
    assert_eq!((stderr.as_str(), code), ("", 0));
    assert_eq!(stdout, "1\n".repeat(20_000));
    let mut nest = String::from("(1 | select(false))");
    for _ in 0..15 {
        nest = format!("({nest} // 1)");
    }
    let started = std::time::Instant::now();
    assert_rows(INPUT, &[(format!("select(false) | {nest}").as_str(), "")])?;
    assert!(started.elapsed() < std::time::Duration::from_secs(30));
    Ok(())
}

/// A body that reads its variables is written twice per binding, bound and not, and a level whose
/// rewrite drops its reads (a collect) is written once. yq answers each row below.
#[test]
fn a_nest_whose_body_reads_its_variables_answers_8044() -> Result<()> {
    let nest = |n: usize, body: &str| {
        let bindings: String = (1..=n)
            .map(|i| format!("(1 | select(false)) as $a{i} | "))
            .collect();
        format!("select(false) | ({bindings}{body})")
    };
    let vars = |n: usize| -> String {
        (1..=n)
            .map(|i| format!("$a{i}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let seven = nest(7, &format!("({}, 9)", vars(7)));
    let collected = format!(
        "select(false) | ({}(1 | select(false)) as $b | ([{}], $b, 9))",
        (1..=6)
            .map(|i| format!("(1 | select(false)) as $a{i} | "))
            .collect::<String>(),
        vars(6)
    );
    assert_rows(
        INPUT,
        &[(seven.as_str(), "9\n"), (collected.as_str(), "[]\n9\n")],
    )
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
    (
        "select(false) | (1 as $x ireduce (0; . + ($x | key)))",
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

/// A fold that answered nothing over a context with nodes is not an empty context's fold: its
/// UPDATE dropped the accumulators, so it stays empty where it is an operand or a binding's source.
#[test]
fn a_fold_that_dropped_its_accumulators_stays_empty_8044() -> Result<()> {
    assert_rows(
        INPUT,
        &[
            (
                "(.[] as $x ireduce (0; select(false))) as $q | [$q]",
                "[]\n",
            ),
            ("[.[] as $x ireduce (0; select(false))]", "[]\n"),
            (".[0] = (.[] as $x ireduce (0; select(false)))", "[1,2]\n"),
            ("(.[] as $x ireduce (0; . + $x; .)) as $q | [$q]", "[]\n"),
            ("(.[] as $y | select(false)) as $q | [$q]", "[]\n"),
            // After an empty stage it is INIT, there too.
            (
                "(select(false) | (.[] as $x ireduce (0; . + 1))) as $q | $q",
                "0\n",
            ),
            (
                ".[0] = (select(false) | (.[] as $x ireduce (0; . + 1)))",
                "[0,2]\n",
            ),
            (
                "1 + (select(false) | (.[] as $x ireduce (0; . + 1)))",
                "1\n",
            ),
            ("[select(false) | (.[] as $x ireduce (0; . + 1))]", "[0]\n"),
            ("(select(false) | (. as $y | 3)) as $q | $q", "3\n"),
            ("1 + (select(false) | (. as $y | 3))", "4\n"),
        ],
    )?;
    // Over `[5]` the source's `.[]` folds the member, which UPDATE drops; rewriting the fold as
    // one over an empty context would keep only the `1` branch and answer it.
    assert_rows(
        "[5]\n",
        &[
            (
                "((.[], 1) as $x ireduce (0; select($x == 1) | . + 1)) as $q | [$q]",
                "[]\n",
            ),
            (
                "1 + ((.[], 1) as $x ireduce (0; select($x == 1) | . + 1))",
                "1\n",
            ),
        ],
    )
}

/// Bodies a write, a builtin or a fold evaluates itself (#8002, #8035).
#[test]
fn a_binding_in_a_body_over_an_empty_context_8044() -> Result<()> {
    // `map` runs its body once over the whole list of members, so a member whose `.a` yields
    // nothing (a scalar) leaves the binding an empty context: one `[]` for the list, not one per
    // member. This is why such a body takes the list walk.
    assert_rows("[5, 6]\n", &[("map(.a | . as $x | [$x])", "[[]]\n")])?;
    assert_rows(
        "[{\"a\": 1}, 5]\n",
        &[("map(.a | . as $x | [$x])", "[[1]]\n")],
    )?;
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
