//! #7564: a program holding a path target the shipped walkers cannot take (`1 |= 1`, `del(1)`,
//! `path(1)`, `path(tostream)`), even in a branch that never runs, runs whole through the
//! path-mode interpreter (#5223), where `reduce`'s `. + $x` copied the array state on every step:
//! `add` and `flatten` of an array of arrays were quadratic in the result (over 40 s for a
//! 1600 x 1600 grid, jq 0.6 s). A step that only appends now appends in place, as jq's does.
//! Expected values were captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 answers the same.

use std::io::Write;
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
            bail!("`{filter}` ran past {limit:?}: the append step copies its state again");
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

/// Every dead path target the issue lists sends the program to the interpreter; `add`,
/// `flatten | add` and `flatten | length` of a 1500 x 400 array of arrays (600,000 elements)
/// stay linear there. The copying step took about 1500 * 300,000 element copies per program,
/// minutes in a debug build; the in-place one takes about a second. The bound sits between.
#[test]
fn test_add_and_flatten_stay_linear_beside_a_dead_path_target_7564() -> Result<()> {
    for dead in ["1 |= 1", "del(1)", "path(1)", "path(tostream .[0])", "."] {
        for (tail, expected) in [
            ("add | length", "600000\n"),
            ("flatten | add", "0\n"),
            ("flatten | length", "600000\n"),
        ] {
            let filter = format!(
                "[range(1500) | [range(400) | 0]] | if false then ({dead}) else . end | {tail}"
            );
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

/// The literal target still raises jq's error when its branch runs.
#[test]
fn test_a_literal_path_target_raises_when_it_runs_7564() -> Result<()> {
    for filter in [
        "[[1,2],[3]] | (1 |= 1)",
        "del(1)",
        "path(1)",
        "1 = 1",
        "1 += 1",
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (
                "",
                "jq: error (at <unknown>): Invalid path expression with result 1\n",
                5
            ),
            "{filter}"
        );
    }
    Ok(())
}

/// The in-place step answers what the copying one did, on both routes: each with a dead
/// `path(1)` in front (the interpreter) and without (the shipped evaluator). A generator
/// operand adds each output to the state, not to the previous sum; `null` and `[]` add nothing;
/// an update with no output leaves `null`; a scalar operand raises with the state as it was; an
/// error after an output is the generator's; and a state the input shares stays that value when
/// nothing is added, which `path` sees.
#[test]
fn test_the_append_step_answers_as_jq_7564() -> Result<()> {
    let cases: &[(&str, &str, &str, i32)] = &[
        (
            "reduce range(2) as $i ([0]; . + ([$i], [$i*10]))",
            "[0,0,10]\n",
            "",
            0,
        ),
        (
            "reduce (null, [], [1], [[2]]) as $x ([]; . + $x)",
            "[1,[2]]\n",
            "",
            0,
        ),
        (
            "[[1,[2]],[[3,[4]]]] | flatten, flatten(1)",
            "[1,2,3,4]\n[1,[2],[3,[4]]]\n",
            "",
            0,
        ),
        (
            "reduce range(3) as $i ([]; if $i == 1 then . else . + [$i] end)",
            "[0,2]\n",
            "",
            0,
        ),
        ("reduce range(3) as $i ([]; empty)", "null\n", "", 0),
        (
            "[1] as $a | reduce range(2) as $i ($a; . + (null, [$i]))",
            "[1,0,1]\n",
            "",
            0,
        ),
        ("[[1,2],[3],null,[]] | add", "[1,2,3]\n", "", 0),
        (
            r#"reduce range(2) as $i ([]; . + ([1], 2, error("x")))"#,
            "",
            "jq: error (at <unknown>): array ([]) and number (2) cannot be added\n",
            5,
        ),
        (
            r#"reduce range(2) as $i ([]; . + ([1], error("x")))"#,
            "",
            "jq: error (at <unknown>): x\n",
            5,
        ),
        (
            r#"reduce range(2) as $i ([]; . + {"a": 1})"#,
            "",
            "jq: error (at <unknown>): array ([]) and object ({\"a\":1}) cannot be added\n",
            5,
        ),
        (
            "[[1]] | path(.[0] | reduce (null,[]) as $x (.; . + $x))",
            "[0]\n",
            "",
            0,
        ),
        (
            "[[1]] | path(.[0] | reduce (1) as $x (.; . + ([2], null)))",
            "[0]\n",
            "",
            0,
        ),
        (
            "[[1]] | path(.[0] | reduce ([1]) as $x (.; . + $x))",
            "",
            "jq: error (at <unknown>): Invalid path expression with result [1,1]\n",
            5,
        ),
        (
            "[[1],[2]] | path(add)",
            "",
            "jq: error (at <unknown>): Invalid path expression with result [1,2]\n",
            5,
        ),
    ];
    for &(program, stdout, stderr, code) in cases {
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

/// The same programs read their input from stdin on the document route.
#[test]
fn test_add_beside_a_dead_target_over_a_document_7564() -> Result<()> {
    let mut input = Vec::new();
    write!(input, "[")?;
    for i in 0..300 {
        if i > 0 {
            write!(input, ",")?;
        }
        write!(input, "[{i},{i}]")?;
    }
    write!(input, "]")?;
    let input = String::from_utf8(input)?;
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "if false then (1 |= 1) else . end | add | length"],
        Some(&input),
    )?;
    assert_eq!((stdout.as_str(), code), ("600\n", 0), "{stderr:?}");
    Ok(())
}
