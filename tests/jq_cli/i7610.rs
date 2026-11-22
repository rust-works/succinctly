//! #7610: a program holding a path target the shipped walkers cannot take (`1 |= 1`, `del(1)`,
//! `path(1)`), even in a branch that never runs, runs whole through the path-mode interpreter
//! (#5223), where `reduce`'s `. + $x` copied an object or a string state on every step: `add` of
//! many objects or strings, and `reduce ... (. + {(k): v})` or `(. + "s")`, were quadratic (40,000
//! objects took 8.5 s, jq 0.03 s). #7564 made the array step append in place; an object or a
//! string state that nothing else shares now grows in place too, as jq's does.
//! Expected values were captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 answers the same, except
//! one row of the matrix whose error preview 1.8 words differently (jqlang/jq#3478).

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::{run_jq_full, succinctly_bin};

/// `succinctly jq -nc <filter>`, killed after `limit` so a regression fails instead of hanging.
fn run_jq_null_bounded(filter: &str, limit: Duration) -> Result<(String, String, i32)> {
    let mut child = Command::new(succinctly_bin())
        .args(["jq", "-nc", filter])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if started.elapsed() > limit {
            child.kill()?;
            child.wait()?;
            bail!("`{filter}` ran past {limit:?}: the grow step copies its state again");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

/// Every dead path target sends the program to the interpreter; `add` of 40,000 objects and of
/// 100,000 strings of 100 bytes, and the two `reduce` forms, stay linear there. The copying step
/// took minutes in a debug build; the in-place one takes seconds. The bound sits between.
#[test]
fn test_object_and_string_add_stay_linear_beside_a_dead_path_target_7610() -> Result<()> {
    for dead in ["1 |= 1", "del(1)", "path(1)"] {
        for (program, expected) in [
            (
                "[range(40000) | {(\"k\\(.)\"): .}] | DEAD | add | length",
                "40000\n",
            ),
            (
                "reduce range(40000) as $i ({}; . + {(\"k\\($i)\"): $i}) | DEAD | length",
                "40000\n",
            ),
            (
                "[range(100000) | \"0123456789\" * 10] | DEAD | add | length",
                "10000000\n",
            ),
            (
                "reduce range(100000) as $i (\"\"; . + \"0123456789\" * 10) | DEAD | length",
                "10000000\n",
            ),
        ] {
            let filter = program.replace("DEAD", &format!("if false then ({dead}) else . end"));
            let (stdout, stderr, code) = run_jq_null_bounded(&filter, Duration::from_secs(60))?;
            assert_eq!(
                (stdout.as_str(), code),
                (expected, 0),
                "{filter}: {stderr:?}"
            );
        }
    }
    Ok(())
}

/// The in-place step answers what the copying one did, on both routes: each program with a dead
/// `path(1)` in front (the interpreter) and without (the shipped evaluator). The matrix crosses
/// object, string, `null` and shared (`$s`, `$o`) states with updates that add once, add each
/// output of a generator (only the last is the next state), skip, keep the state, or raise after
/// an output, over sources mixing objects, strings, `null`, `{}`, `""` and values of other kinds.
/// The hand rows pin `path` identity (a state the input shares stays that value only when nothing
/// is added), key order (an existing key keeps its place, a new one goes last), `null` values,
/// and each kind's type error.
#[test]
fn test_the_grow_step_answers_as_jq_7610() -> Result<()> {
    for &(program, stdout, stderr, code) in MATRIX.iter().chain(HAND) {
        for filter in [
            program.to_string(),
            format!("if false then path(1) else . end | {program}"),
        ] {
            let got = run_jq_full(&["-nc", &filter], None)?;
            assert_eq!(
                (got.0.as_str(), got.1.as_str(), got.2),
                (stdout, stderr, code),
                "{filter}"
            );
        }
    }
    Ok(())
}

/// The same `add` reading its input from stdin on the document route, an object with a repeated
/// key among the inputs (jq keeps the last).
#[test]
fn test_object_add_beside_a_dead_target_over_a_document_7610() -> Result<()> {
    let mut input = String::from("[{\"a\":1,\"a\":2},");
    for i in 0..300 {
        input.push_str(&format!("{{\"k{i}\":{i}}},"));
    }
    input.push_str("{\"a\":3,\"b\":4}]");
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "if false then (1 |= 1) else . end | add | [length, .a, .b, .k299, (keys_unsorted | .[0], .[300])]",
        ],
        Some(&input),
    )?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), code),
        ("[302,3,4,299,\"a\",\"k299\"]\n", "", 0)
    );
    Ok(())
}

/// Captured from `/usr/bin/jq` 1.7.1 by crossing the states, updates and sources described on
/// [`test_the_grow_step_answers_as_jq_7610`].
const MATRIX: &[(&str, &str, &str, i32)] = &[
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({}; . + $x)",
        "{\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({}; . + $x)",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({}; . + $x)",
        "",
        "jq: error (at <unknown>): object ({\"a\":1}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({}; . + $x)",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({}; . + $x)",
        "{}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({}; . + $x)",
        "{\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({}; . + ($x, $x))",
        "{\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({}; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({}; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): object ({\"a\":1}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({}; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({}; . + ($x, $x))",
        "{}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({}; . + ($x, $x))",
        "{\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({}; if $x == null then . else . + $x end)",
        "{\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({}; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({}; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): object ({\"a\":1}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({}; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({}; if $x == null then . else . + $x end)",
        "{}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({}; if $x == null then . else . + $x end)",
        "{\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({}; .)",
        "{}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({}; .)",
        "{}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({}; .)",
        "{}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({}; .)",
        "{}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({}; .)",
        "{}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({}; .)",
        "{}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({}; . + ($x, {\"z\":0}))",
        "{\"z\":0}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({}; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({}; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): object ({\"z\":0}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({}; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({}; . + ($x, {\"z\":0}))",
        "{\"z\":0}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({}; . + ($x, {\"z\":0}))",
        "{\"z\":0}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({}; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({}; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({}; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({}; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({}; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({}; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({}; . + ($x | if . == 1 then error(\"e\") else . end))",
        "{\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({}; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({}; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): e\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({}; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): object ({}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({}; . + ($x | if . == 1 then error(\"e\") else . end))",
        "{}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({}; . + ($x | if . == 1 then error(\"e\") else . end))",
        "{\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({\"b\":1,\"a\":2}; . + $x)",
        "{\"b\":1,\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({\"b\":1,\"a\":2}; . + $x)",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({\"b\":1,\"a\":2}; . + $x)",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":1}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({\"b\":1,\"a\":2}; . + $x)",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({\"b\":1,\"a\":2}; . + $x)",
        "{\"b\":1,\"a\":2}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({\"b\":1,\"a\":2}; . + $x)",
        "{\"b\":1,\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({\"b\":1,\"a\":2}; . + ($x, $x))",
        "{\"b\":1,\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({\"b\":1,\"a\":2}; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({\"b\":1,\"a\":2}; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":1}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({\"b\":1,\"a\":2}; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({\"b\":1,\"a\":2}; . + ($x, $x))",
        "{\"b\":1,\"a\":2}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({\"b\":1,\"a\":2}; . + ($x, $x))",
        "{\"b\":1,\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({\"b\":1,\"a\":2}; if $x == null then . else . + $x end)",
        "{\"b\":1,\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({\"b\":1,\"a\":2}; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({\"b\":1,\"a\":2}; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":1}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({\"b\":1,\"a\":2}; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({\"b\":1,\"a\":2}; if $x == null then . else . + $x end)",
        "{\"b\":1,\"a\":2}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({\"b\":1,\"a\":2}; if $x == null then . else . + $x end)",
        "{\"b\":1,\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({\"b\":1,\"a\":2}; .)",
        "{\"b\":1,\"a\":2}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({\"b\":1,\"a\":2}; .)",
        "{\"b\":1,\"a\":2}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({\"b\":1,\"a\":2}; .)",
        "{\"b\":1,\"a\":2}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({\"b\":1,\"a\":2}; .)",
        "{\"b\":1,\"a\":2}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({\"b\":1,\"a\":2}; .)",
        "{\"b\":1,\"a\":2}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({\"b\":1,\"a\":2}; .)",
        "{\"b\":1,\"a\":2}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({\"b\":1,\"a\":2}; . + ($x, {\"z\":0}))",
        "{\"b\":1,\"a\":2,\"z\":0}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({\"b\":1,\"a\":2}; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({\"b\":1,\"a\":2}; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":...) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({\"b\":1,\"a\":2}; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({\"b\":1,\"a\":2}; . + ($x, {\"z\":0}))",
        "{\"b\":1,\"a\":2,\"z\":0}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({\"b\":1,\"a\":2}; . + ($x, {\"z\":0}))",
        "{\"b\":1,\"a\":2,\"z\":0}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({\"b\":1,\"a\":2}; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({\"b\":1,\"a\":2}; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({\"b\":1,\"a\":2}; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({\"b\":1,\"a\":2}; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({\"b\":1,\"a\":2}; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({\"b\":1,\"a\":2}; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ({\"b\":1,\"a\":2}; . + ($x | if . == 1 then error(\"e\") else . end))",
        "{\"b\":1,\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ({\"b\":1,\"a\":2}; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ({\"b\":1,\"a\":2}; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): e\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ({\"b\":1,\"a\":2}; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): object ({\"b\":1,\"a\":2}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ({\"b\":1,\"a\":2}; . + ($x | if . == 1 then error(\"e\") else . end))",
        "{\"b\":1,\"a\":2}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ({\"b\":1,\"a\":2}; . + ($x | if . == 1 then error(\"e\") else . end))",
        "{\"b\":1,\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"\"; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"\"; . + $x)",
        "\"xy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"\"; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"\"; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"x\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"\"; . + $x)",
        "\"\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"\"; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"\"; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"\"; . + ($x, $x))",
        "\"xy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"\"; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"\"; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"x\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"\"; . + ($x, $x))",
        "\"\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"\"; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"\"; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"\"; if $x == null then . else . + $x end)",
        "\"xy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"\"; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"\"; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"x\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"\"; if $x == null then . else . + $x end)",
        "\"\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"\"; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"\"; .)",
        "\"\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"\"; .)",
        "\"\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"\"; .)",
        "\"\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"\"; .)",
        "\"\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"\"; .)",
        "\"\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"\"; .)",
        "\"\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"\"; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"\"; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"z\":0}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"\"; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"\"; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"z\":0}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"\"; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"z\":0}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"\"; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"\"; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"\"; . + (\"t\", $x))",
        "\"xy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"\"; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"\"; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"x\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"\"; . + (\"t\", $x))",
        "\"\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"\"; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"\"; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"\"; . + ($x | if . == 1 then error(\"e\") else . end))",
        "\"xy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"\"; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"\"; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"x\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"\"; . + ($x | if . == 1 then error(\"e\") else . end))",
        "\"\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"\"; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"s\"; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"s\"; . + $x)",
        "\"sxy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"s\"; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"s\"; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"sx\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"s\"; . + $x)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"s\"; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"s\"; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"s\"; . + ($x, $x))",
        "\"sxy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"s\"; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"s\"; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"sx\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"s\"; . + ($x, $x))",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"s\"; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"s\"; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"s\"; if $x == null then . else . + $x end)",
        "\"sxy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"s\"; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"s\"; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"sx\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"s\"; if $x == null then . else . + $x end)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"s\"; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"s\"; .)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"s\"; .)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"s\"; .)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"s\"; .)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"s\"; .)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"s\"; .)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"s\"; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"s\"; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"z\":0}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"s\"; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"s\"; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"z\":0}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"s\"; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"z\":0}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"s\"; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"s\"; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"s\"; . + (\"t\", $x))",
        "\"sxy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"s\"; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"s\"; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"sx\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"s\"; . + (\"t\", $x))",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"s\"; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (\"s\"; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (\"s\"; . + ($x | if . == 1 then error(\"e\") else . end))",
        "\"sxy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (\"s\"; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (\"s\"; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"sx\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (\"s\"; . + ($x | if . == 1 then error(\"e\") else . end))",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (\"s\"; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (null; . + $x)",
        "{\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (null; . + $x)",
        "\"xy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (null; . + $x)",
        "",
        "jq: error (at <unknown>): object ({\"a\":1}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (null; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"x\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (null; . + $x)",
        "null\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (null; . + $x)",
        "{\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (null; . + ($x, $x))",
        "{\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (null; . + ($x, $x))",
        "\"xy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (null; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): object ({\"a\":1}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (null; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"x\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (null; . + ($x, $x))",
        "null\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (null; . + ($x, $x))",
        "{\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (null; if $x == null then . else . + $x end)",
        "{\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (null; if $x == null then . else . + $x end)",
        "\"xy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (null; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): object ({\"a\":1}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (null; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"x\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (null; if $x == null then . else . + $x end)",
        "null\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (null; if $x == null then . else . + $x end)",
        "{\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (null; .)",
        "null\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (null; .)",
        "null\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (null; .)",
        "null\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (null; .)",
        "null\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (null; .)",
        "null\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (null; .)",
        "null\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (null; . + ($x, {\"z\":0}))",
        "{\"z\":0}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (null; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): object ({\"z\":0}) and string (\"\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (null; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): object ({\"z\":0}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (null; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): object ({\"z\":0}) and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (null; . + ($x, {\"z\":0}))",
        "{\"z\":0}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (null; . + ($x, {\"z\":0}))",
        "{\"z\":0}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (null; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"a\":1}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (null; . + (\"t\", $x))",
        "\"xy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (null; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"a\":1}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (null; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"x\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (null; . + (\"t\", $x))",
        "null\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (null; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"a\":{\"p\":1}}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x (null; . + ($x | if . == 1 then error(\"e\") else . end))",
        "{\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x (null; . + ($x | if . == 1 then error(\"e\") else . end))",
        "\"xy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x (null; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): e\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x (null; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"x\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x (null; . + ($x | if . == 1 then error(\"e\") else . end))",
        "null\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x (null; . + ($x | if . == 1 then error(\"e\") else . end))",
        "{\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($s; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($s; . + $x)",
        "\"sxy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($s; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($s; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"sx\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($s; . + $x)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($s; . + $x)",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($s; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($s; . + ($x, $x))",
        "\"sxy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($s; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($s; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"sx\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($s; . + ($x, $x))",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($s; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($s; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($s; if $x == null then . else . + $x end)",
        "\"sxy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($s; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($s; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"sx\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($s; if $x == null then . else . + $x end)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($s; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($s; .)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($s; .)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($s; .)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($s; .)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($s; .)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($s; .)",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($s; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($s; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"z\":0}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($s; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($s; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"z\":0}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($s; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"z\":0}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($s; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($s; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($s; . + (\"t\", $x))",
        "\"sxy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($s; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($s; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"sx\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($s; . + (\"t\", $x))",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($s; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($s; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($s; . + ($x | if . == 1 then error(\"e\") else . end))",
        "\"sxy\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($s; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($s; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"sx\") and array ([1]) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($s; . + ($x | if . == 1 then error(\"e\") else . end))",
        "\"s\"\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($s; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): string (\"s\") and object ({\"a\":{\"p\":1}}) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($o; . + $x)",
        "{\"q\":1,\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($o; . + $x)",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($o; . + $x)",
        "",
        "jq: error (at <unknown>): object ({\"q\":1,\"a\":1}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($o; . + $x)",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($o; . + $x)",
        "{\"q\":1}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($o; . + $x)",
        "{\"q\":1,\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($o; . + ($x, $x))",
        "{\"q\":1,\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($o; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($o; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1,\"a\":1}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($o; . + ($x, $x))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($o; . + ($x, $x))",
        "{\"q\":1}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($o; . + ($x, $x))",
        "{\"q\":1,\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($o; if $x == null then . else . + $x end)",
        "{\"q\":1,\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($o; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($o; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): object ({\"q\":1,\"a\":1}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($o; if $x == null then . else . + $x end)",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($o; if $x == null then . else . + $x end)",
        "{\"q\":1}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($o; if $x == null then . else . + $x end)",
        "{\"q\":1,\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($o; .)",
        "{\"q\":1}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($o; .)",
        "{\"q\":1}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($o; .)",
        "{\"q\":1}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($o; .)",
        "{\"q\":1}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($o; .)",
        "{\"q\":1}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($o; .)",
        "{\"q\":1}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($o; . + ($x, {\"z\":0}))",
        "{\"q\":1,\"z\":0}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($o; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($o; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1,\"z\":0}) and number (1) cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($o; . + ($x, {\"z\":0}))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($o; . + ($x, {\"z\":0}))",
        "{\"q\":1,\"z\":0}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($o; . + ($x, {\"z\":0}))",
        "{\"q\":1,\"z\":0}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($o; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($o; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($o; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($o; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($o; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($o; . + (\"t\", $x))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"t\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, null, {}, {\"c\":3}, {\"a\":4}) as $x ($o; . + ($x | if . == 1 then error(\"e\") else . end))",
        "{\"q\":1,\"a\":4,\"c\":3}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", null, \"\", \"y\") as $x ($o; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":1}, 1) as $x ($o; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): e\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (\"x\", [1]) as $x ($o; . + ($x | if . == 1 then error(\"e\") else . end))",
        "",
        "jq: error (at <unknown>): object ({\"q\":1}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce (null, empty) as $x ($o; . + ($x | if . == 1 then error(\"e\") else . end))",
        "{\"q\":1}\n",
        "",
        0,
    ),
    (
        "\"s\" as $s | {\"q\":1} as $o | reduce ({\"a\":{\"p\":1}}, {\"a\":{\"q\":2}}) as $x ($o; . + ($x | if . == 1 then error(\"e\") else . end))",
        "{\"q\":1,\"a\":{\"q\":2}}\n",
        "",
        0,
    ),
];

const HAND: &[(&str, &str, &str, i32)] = &[
    (
        "{\"a\":{\"x\":1}} | path(.a | reduce (null,{}) as $x (.; . + $x))",
        "[\"a\"]\n",
        "",
        0,
    ),
    (
        "{\"a\":{\"x\":1}} | path(.a | reduce ({\"y\":1}) as $x (.; . + $x))",
        "",
        "jq: error (at <unknown>): Invalid path expression with result {\"x\":1,\"y\":1}\n",
        5,
    ),
    (
        "{\"a\":\"s\"} | path(.a | reduce (null) as $x (.; . + $x))",
        "[\"a\"]\n",
        "",
        0,
    ),
    (
        "{\"a\":\"s\"} | path(.a | reduce (\"\") as $x (.; . + $x))",
        "",
        "jq: error (at <unknown>): Invalid path expression with result \"s\"\n",
        5,
    ),
    (
        "{\"a\":\"s\"} | path(.a | reduce (\"t\") as $x (.; . + $x))",
        "",
        "jq: error (at <unknown>): Invalid path expression with result \"st\"\n",
        5,
    ),
    (
        "[{\"b\":1,\"a\":2},{\"a\":3,\"c\":4}] | add",
        "{\"b\":1,\"a\":3,\"c\":4}\n",
        "",
        0,
    ),
    ("[\"a\",\"b\",null,\"c\"] | add", "\"abc\"\n", "", 0),
    ("[{\"a\":1},{\"a\":null}] | add", "{\"a\":null}\n", "", 0),
    (
        "{\"b\":1,\"a\":2} | reduce ({\"a\":9},{\"c\":3}) as $x (.; . + $x)",
        "{\"b\":1,\"a\":9,\"c\":3}\n",
        "",
        0,
    ),
    (
        "reduce range(3) as $i ({}; if $i == 1 then . else . + {(\"k\\($i)\"): $i} end)",
        "{\"k0\":0,\"k2\":2}\n",
        "",
        0,
    ),
    (
        "[{\"a\":1},\"x\"] | add",
        "",
        "jq: error (at <unknown>): object ({\"a\":1}) and string (\"x\") cannot be added\n",
        5,
    ),
    (
        "[\"x\",{\"a\":1}] | add",
        "",
        "jq: error (at <unknown>): string (\"x\") and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    (
        "reduce range(4) as $i (\"\"; . + (\"ab\" * $i))",
        "\"abababababab\"\n",
        "",
        0,
    ),
    (
        "{\"a\":[1]} as $v | reduce range(2) as $i ({}; . + $v) | .a == $v.a",
        "true\n",
        "",
        0,
    ),
];
