//! #5745: a fold whose UPDATE (or EXTRACT) reads no loop variable runs it as written instead of
//! as a copy rebuilt per step. A copy of an operand with no loop variable in it is the operand, so
//! every row's answer is the one it had; what the rows guard is the boundary of the skip: an
//! operand that reads a loop variable (or binds the same name itself), a definition, a `?//`
//! chain, several INIT forks, a `break` and a generator step still take the rebuilt copy's answer.
//! Outputs are captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_stdin;

/// `(input, filter, stdout)`, every one exiting 0 under `-c`.
const ROWS: &[(&str, &str, &str)] = &[
    ("[1,2,3]", "reduce .[] as $e (0; .+1)", "3\n"),
    ("[1,2,3]", "reduce .[] as $e (0; .+$e)", "6\n"),
    ("[1,2,3]", "[foreach .[] as $e (0; .+1)]", "[1,2,3]\n"),
    ("[1,2,3]", "[foreach .[] as $e (0; .+1; .*2)]", "[2,4,6]\n"),
    ("[1,2,3]", "[foreach .[] as $e (0; .+$e; .*10)]", "[10,30,60]\n"),
    ("[1,2,3]", "[foreach .[] as $e (0; .+1; [., $e])]", "[[1,1],[2,2],[3,3]]\n"),
    ("[[1],2]", "reduce .[] as [$a] ?// $a (0; .+1)", "2\n"),
    ("[[1],2]", "[foreach .[] as [$a] ?// $a (0; .+1; .)]", "[1,2]\n"),
    ("[1,2,3]", "reduce .[] as $e ({}; .[\"k\"] += 1)", "{\"k\":3}\n"),
    ("[1,2,3]", "reduce .[] as $e (0; def f: .+1; f)", "3\n"),
    ("[1,2,3]", "reduce range(3) as $i (0; reduce range(2) as $j (.; .+1))", "6\n"),
    ("[1,2,3]", "[limit(2; foreach .[] as $e (0; .+1))]", "[1,2]\n"),
    ("[1,2,3]", "[.[] | try (reduce . as $e (0; error(\"boom\"))) catch .]", "[\"boom\",\"boom\",\"boom\"]\n"),
    ("[1,2,3]", "reduce .[] as $e (0; if . > 1 then . else .+1 end)", "2\n"),
    ("[1,2,3]", "reduce .[] as $e (0; . as $x | $x + 1)", "3\n"),
    ("[1,2,3]", "reduce .[] as $e (0; . + ([.] | length))", "3\n"),
    ("[1,2,3]", "reduce .[] as $e (0; 1 as $e | . + $e)", "3\n"),
    ("[1,2,3]", "[foreach .[] as $e (0; .+1; 1 as $e | . + $e)]", "[2,3,4]\n"),
    ("[1,2,3]", "[foreach .[] as $e (0; .+$e; (.,.))]", "[1,1,3,3,6,6]\n"),
    ("[1,2,3]", "reduce .[] as $e ([]; . + [1])", "[1,1,1]\n"),
    ("[1,2,3]", "reduce .[] as $e (null; .+1)", "3\n"),
    ("[1,2,3]", "[foreach .[] as $e (0; empty; .)]", "[]\n"),
    ("[1,2,3]", "reduce .[] as $e (0; ., .+1)", "3\n"),
    ("[1,2,3]", "label $out | reduce .[] as $e (0; if . == 2 then ., break $out else .+1 end)", ""),
    ("[1,2,3]", "[foreach range(3) as $e ((0,10); .+1)]", "[1,2,3,11,12,13]\n"),
    ("[1,2,3]", "[foreach range(3) as $e ((0,10); .+1; .*$e)]", "[0,2,6,0,12,26]\n"),
];

#[test]
fn fold_steps_that_read_no_loop_variable_answer_as_jq_5745() -> Result<()> {
    for (input, filter, expected) in ROWS {
        let (out, code) = run_jq_stdin(filter, input, &["-c"])?;
        assert_eq!((out.as_str(), code), (*expected, 0), "{filter}");
    }
    Ok(())
}

/// The benchmark shape of the issue: `reduce`/`foreach` over `--stream` events.
#[test]
fn folds_over_stream_events_answer_as_jq_5745() -> Result<()> {
    let input = r#"{"a":[1,2],"b":3}"#;
    for (filter, expected) in [
        ("reduce (inputs|select(length==2)) as $e (0;.+1)", "3\n"),
        (
            "[foreach (inputs|select(length==2)) as $e (0;.+1; [., $e[1]])]",
            "[[1,1],[2,2],[3,3]]\n",
        ),
        ("[foreach inputs as $e (0;.+1; .)]", "[1,2,3,4,5]\n"),
    ] {
        let (out, code) = run_jq_stdin(filter, input, &["-c", "-n", "--stream"])?;
        assert_eq!((out.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}
