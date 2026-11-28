//! #8064: the call machine (`SUCCINCTLY_CALL_MACHINE=1`) is bounded by the memory the process
//! holds, and has no bound on a loop's outputs.
//!
//! The shipped evaluators refuse a zero-argument `def`'s tail loop after 2^20 outputs
//! (`f/0: maximum outputs exceeded`, #5379), to keep a collector from exhausting memory; jq 1.7.1
//! has no such bound. `succinctly jq` gives the machine a probe of the process's resident memory
//! (`/proc/self/statm` on Linux, the physical footprint on macOS) and a limit computed from the
//! memory available to it, and the machine samples it as it runs, refusing past the limit with
//! `memory limit exceeded`. So it answers as jq does where the shipped evaluators refuse a loop's
//! outputs, and refuses in bounded memory where jq grows until its allocation fails. These rows
//! run the probe for real; `SUCCINCTLY_DEBUG_CALL_MACHINE_MEMORY_LIMIT` lowers the limit so they
//! do not fill the test machine.

use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

const SWITCH: &str = "SUCCINCTLY_CALL_MACHINE";
const LOG: &str = "SUCCINCTLY_DEBUG_CALL_MACHINE_LOG";
const MEMORY: &str = "SUCCINCTLY_DEBUG_CALL_MACHINE_MEMORY_LIMIT";

/// 256 MiB: low enough that the rows refused at it are quick and light, high enough above what
/// the binary holds at rest.
const LOW: &str = "268435456";

/// `(stdout, stderr, exit, taken)` of `succinctly jq -nc <filter>` with the switch on when `on`,
/// with the machine's memory limit set to `memory` bytes when given; `taken` is whether the
/// decision log holds only `taken` lines.
fn run(
    filter: &str,
    on: bool,
    memory: Option<&str>,
    dir: &Path,
) -> Result<(String, String, i32, bool)> {
    let log = dir.join("decisions.log");
    let (output, exit) = spawn_with_signal_retry(
        || {
            let _ = std::fs::remove_file(&log);
            let mut command = Command::new(succinctly_bin());
            command.args(["jq", "-nc", filter]);
            match memory {
                Some(bytes) => command.env(MEMORY, bytes),
                None => command.env_remove(MEMORY),
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
/// see (a loop through a local `def`, an `as` source that does not settle). Each grew past 3 GB
/// on a budget that charged a slot per collected value; each is refused here at the memory limit
/// with its own wording, naming the innermost definition on the call chain or none. jq grows on
/// each until memory runs out.
#[test]
fn a_collector_past_the_memory_limit_is_refused_8064() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for filter in [
        "[def f: def m: .+1 | f; ([range(100)] | tostring), m; 0|f] | length",
        "[def f: (.?) as $x | [range(100)], ($x+1|f); 0|f] | length",
        "[def f: def g: [range(100)], (.+1|f); g; 0|f] | length",
        "[def f: {a: [range(100)]}, (.+1|f); 0|f] | length",
        "reduce (def f: ., (.+1|f); 0|f) as $x (null; [$x, [range(100)], .]) | length",
        "foreach (def f: ., (.+1|f); 0|f) as $x (null; [$x, [range(100)], .]; empty)",
        "[def f: \"\\(.)-\\([range(100)])\", (.+1|f); 0|f] | length",
        // A recursion whose frames keep what each built, collecting nothing.
        "def f: [range(1000)] as $a | f, ($a | length); f | empty",
    ] {
        let (stdout, stderr, exit, taken) = run(filter, true, Some(LOW), dir.path())?;
        assert_eq!((stdout.as_str(), exit), ("", 5), "{filter}: {stderr}");
        assert!(
            stderr.starts_with("jq: error (at <unknown>): ")
                && stderr.ends_with("memory limit exceeded\n")
                && !stderr.contains("recursion"),
            "{filter}: {stderr}"
        );
        assert!(taken, "{filter}: declined");
    }
    // Uncatchable, as the shipped resource refusals are.
    let (stdout, stderr, exit, _) = run(
        r#"[try ([def f: {a: [range(100)]}, (.+1|f); 0|f] | length) catch "caught"]"#,
        true,
        Some(LOW),
        dir.path(),
    )?;
    assert_eq!((stdout.as_str(), exit), ("", 5), "{stderr}");
    assert!(stderr.ends_with("memory limit exceeded\n"), "{stderr}");
    Ok(())
}

/// The limit is sized from the memory available, not fixed below what jq answers: an array of
/// 4 x 10^7 numbers, which jq 1.7.1 answers in 1.84 GB, the shipped route and main's machine
/// answer, answers on the machine at the computed limit (#8064's finding 3).
#[test]
fn an_array_jq_answers_answers_at_the_computed_limit_8064() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let filter = "def f: f; [range(10000000), range(10000000), range(10000000), range(10000000)] \
                  | length";
    let (stdout, stderr, exit, taken) = run(filter, true, None, dir.path())?;
    assert_eq!((stdout.as_str(), exit), ("40000000\n", 0), "{stderr}");
    assert!(taken, "declined");
    Ok(())
}

/// A value shared with one the run holds is held once: `$big`, a 2^20-element array, collected
/// and folded two thousand times answers within a 256 MiB limit, as jq answers it, where a copy
/// each would be 64 GB.
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
        let (stdout, stderr, exit, taken) = run(&filter, true, Some(LOW), dir.path())?;
        assert_eq!((stdout.as_str(), exit), (want, 0), "{filter}: {stderr}");
        assert!(taken, "{filter}: declined");
    }
    Ok(())
}
