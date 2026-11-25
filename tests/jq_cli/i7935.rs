//! #7935: a recursion that collects each level into an array and yields `.[], .` of it
//! (`def f: if . > 0 then [. - 1 | f] | .[], . else . end`) shares each level's array, as jq does.
//!
//! `f(n)` yields `n + 1` values and value `k` holds every earlier one, so written out it is about
//! `2^k` long. The comma `.[], .` had no owned door, so each level's array was written out as JSON
//! and indexed again: `first(50 | f)` reached 2.6 GB in 3 s with no answer, where jq 1.7.1 answers
//! `0` at once in 2.6 MB. Every expected row was captured from jq 1.7.1 (`/usr/bin/jq -nc`); jq
//! 1.8.2 (`/opt/homebrew/bin/jq`) gives the same for each.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::run_jq_null;

const F: &str = "def f: if . > 0 then [. - 1 | f] | .[], . else . end;";

/// `(program, stdout)`, run as `jq -nc PROGRAM`.
const ROWS: &[(&str, &str)] = &[
    ("[3 | f]", "[0,[0],[0,[0]],[0,[0],[0,[0]]]]\n"),
    ("[40 | f] | length", "41\n"),
    (
        "def g: if . > 0 then [. - 1 | g] | ., .[] else . end; [3 | g]",
        "[[[[0],0],[0],0],[[0],0],[0],0]\n",
    ),
    (
        "def g: if . > 0 then {a: (. - 1 | [g])} | .[], . else . end; [2 | g]",
        "[[[0],{\"a\":[0]}],{\"a\":[[0],{\"a\":[0]}]}]\n",
    ),
    ("[1, [2]] | .[], .", "1\n[2]\n[1,[2]]\n"),
    ("{a: 1, b: [2]} | .[], .", "1\n[2]\n{\"a\":1,\"b\":[2]}\n"),
    (
        "5 | try (.[], .) catch .",
        "\"Cannot iterate over number (5)\"\n",
    ),
    ("null | [.[]?, .]", "[null]\n"),
];

#[test]
fn comma_of_iterate_answers_as_jq_7935() -> Result<()> {
    for (program, expected) in ROWS {
        let program = format!("{F} {program}");
        let (out, code) = run_jq_null(&program, &["-c"])?;
        assert_eq!((out.as_str(), code), (*expected, 0), "{program}");
    }
    Ok(())
}

/// The issue's program at depth 50, and two neighbours, under a deadline: before the fix none
/// answered at all, so the child is killed at the deadline rather than left to allocate.
#[test]
fn deep_recursion_shares_each_level_7935() -> Result<()> {
    for (program, expected) in [
        ("first((50 | f))", "0\n"),
        ("[limit(3; 50 | f)]", "[0,[0],[0,[0]]]\n"),
        (
            "def g: if . > 0 then [. - 1 | g] | .[]?, 1, . else . end; first(50 | g)",
            "0\n",
        ),
    ] {
        let program = format!("{F} {program}");
        let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(["jq", "-nc", &program])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                bail!("{program}: no answer within 10 s");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut out = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout.read_to_string(&mut out)?;
        }
        assert_eq!(
            (out.as_str(), status.code()),
            (expected, Some(0)),
            "{program}"
        );
    }
    Ok(())
}
