//! #8064: the call machine (`SUCCINCTLY_CALL_MACHINE=1`) bounds what a run holds by bytes, and
//! has no bound on a loop's outputs.
//!
//! The shipped evaluators refuse a zero-argument `def`'s tail loop after 2^20 outputs
//! (`f/0: maximum outputs exceeded`, #5379), to keep a collector from exhausting memory; jq 1.7.1
//! has no such bound. The machine counts the heap a run holds instead, as the allocator counts it
//! (what a collected value holds, nested containers, strings and objects, a `reduce`'s or a
//! `foreach`'s state, each once however many places share it), and refuses past 1.25 GiB with
//! the recursion wording. So it answers as jq does where the shipped evaluators refuse a loop's
//! outputs, and refuses in bounded memory where jq grows until its allocation fails.

use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

const SWITCH: &str = "SUCCINCTLY_CALL_MACHINE";
const LOG: &str = "SUCCINCTLY_DEBUG_CALL_MACHINE_LOG";

/// `(stdout, stderr, exit, taken)` of `succinctly jq -nc <filter>` with the switch on when `on`,
/// under the shell prefix `limit` when there is one; `taken` is whether the decision log holds
/// only `taken` lines.
fn run(
    filter: &str,
    on: bool,
    limit: Option<&str>,
    dir: &Path,
) -> Result<(String, String, i32, bool)> {
    let log = dir.join("decisions.log");
    let (output, exit) = spawn_with_signal_retry(
        || {
            let _ = std::fs::remove_file(&log);
            let mut command = match limit {
                Some(limit) => {
                    let mut sh = Command::new("bash");
                    sh.arg("-c")
                        .arg(format!("{limit} && exec \"$0\" jq -nc \"$1\""))
                        .arg(succinctly_bin())
                        .arg(filter);
                    sh
                }
                None => {
                    let mut direct = Command::new(succinctly_bin());
                    direct.args(["jq", "-nc", filter]);
                    direct
                }
            };
            command.env(LOG, &log);
            command.env(SWITCH, if on { "1" } else { "0" });
            command
        },
        None,
    )?;
    let decisions = std::fs::read_to_string(&log).unwrap_or_default();
    let mut lines = decisions
        .lines()
        .filter(|line| *line != "compiled")
        .peekable();
    let taken = lines.peek().is_some() && lines.all(|line| line == "taken");
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        exit,
        taken,
    ))
}

/// Programs the shipped evaluators refuse for their outputs and jq 1.7.1 answers (captured from
/// `/usr/bin/jq` 1.7.1): the machine answers each as jq does.
#[test]
fn a_loop_past_the_shipped_output_bound_answers_as_jq_does_8064() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (filter, want) in [
        (
            "[def f: if . < 1048577 then ., (. + 1 | f) else . end; 0 | f] | length",
            "1048578\n",
        ),
        (
            "def f: if . < 70 then (range(20000) | -1), (. + 1 | f) else . end; [0 | f] | length",
            "1400001\n",
        ),
        // A helper's outputs in front of the call.
        (
            "def g: range(3); def f: if . < 400000 then g, (. + 1 | f) else empty end; \
             [0 | f] | length",
            "1200000\n",
        ),
        // A loop of loops, which the shipped evaluators nest and refuse at their floor.
        (
            "def h($m): if $m < 16132 then $m, h($m + 1) else empty end; \
             def f: if . < 65 then h(0), (. + 1 | f) else empty end; [0 | f] | length",
            "1048580\n",
        ),
    ] {
        let (stdout, stderr, exit, taken) = run(filter, true, None, dir.path())?;
        assert_eq!((stdout.as_str(), exit), (want, 0), "{filter}: {stderr}");
        assert!(taken, "{filter}: declined");
    }
    // Printed rather than collected, past the 2^20 + 1 lines the shipped loop prints before its
    // refusal.
    let filter = "def f: if . < 1100000 then ., (. + 1 | f) else empty end; 0 | f";
    let (stdout, stderr, exit, _) = run(filter, true, None, dir.path())?;
    assert_eq!(exit, 0, "{stderr}");
    assert_eq!(stdout.lines().count(), 1_100_000);
    assert_eq!(stdout.lines().last(), Some("1099999"));
    let (off, stderr, exit, _) = run(filter, false, None, dir.path())?;
    assert_eq!(
        (off.lines().count(), stderr.as_str(), exit),
        (
            (1 << 20) + 1,
            "jq: error (at <unknown>): f/0: maximum outputs exceeded\n",
            5
        )
    );
    Ok(())
}

/// #8064's memory repros: collectors whose values hold heap of their own (objects, arrays,
/// strings), a `reduce`'s and a `foreach`'s state, and collectors the shipped output bound did not
/// see (a loop through a local `def`, an `as` source that does not settle). Each grew past 3 GB on
/// a budget that charged a slot per collected value; each is refused here with the recursion
/// wording at the heap budget. jq grows on each until memory runs out.
#[test]
fn a_collector_past_the_heap_budget_is_refused_8064() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for filter in [
        "[def f: def m: .+1 | f; ([range(100)] | tostring), m; 0|f] | length",
        "[def f: (.?) as $x | [range(100)], ($x+1|f); 0|f] | length",
        "[def f: def g: [range(100)], (.+1|f); g; 0|f] | length",
        "[def f: {a: [range(100)]}, (.+1|f); 0|f] | length",
        "reduce (def f: ., (.+1|f); 0|f) as $x (null; [$x, [range(100)], .]) | length",
        "foreach (def f: ., (.+1|f); 0|f) as $x (null; [$x, [range(100)], .]; empty)",
        "[def f: \"\\(.)-\\([range(100)])\", (.+1|f); 0|f] | length",
    ] {
        let (stdout, stderr, exit, taken) = run(filter, true, None, dir.path())?;
        assert_eq!((stdout.as_str(), exit), ("", 5), "{filter}: {stderr}");
        assert!(
            stderr.starts_with("jq: error (at <unknown>): ")
                && stderr.ends_with("exceeded maximum recursion depth\n"),
            "{filter}: {stderr}"
        );
        assert!(taken, "{filter}: declined");
    }
    Ok(())
}

/// A value shared with one the run holds is charged once: `$big`, a 2^20-element array, collected
/// and folded two thousand times answers, as jq answers it, where charging each copy would be
/// 64 GB.
#[test]
fn a_shared_value_collected_many_times_answers_8064() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let big = "[range(1048576)] as $big | \
               def f($n): if $n < 2000 then $big, f($n + 1) else empty end;";
    for (tail, want) in [
        ("[f(0)] | length", "2000\n"),
        ("[f(0) | {a: .}] | map(.a | length) | add", "2097152000\n"),
        ("reduce f(0) as $x ([]; . + [$x]) | length", "2000\n"),
    ] {
        let filter = format!("{big} {tail}");
        let (stdout, stderr, exit, taken) = run(&filter, true, None, dir.path())?;
        assert_eq!((stdout.as_str(), exit), (want, 0), "{filter}: {stderr}");
        assert!(taken, "{filter}: declined");
    }
    Ok(())
}
