//! #7977 step 2: the call machine's bounds, with `SUCCINCTLY_CALL_MACHINE=1`.
//!
//! A loop that never deepens is refused after the shipped `MAX_LOOP_ROUNDS` rounds, a zero-argument
//! `def`'s tail generator after `MAX_LOOP_OUTPUTS` outputs, and a run whose control state passes its
//! byte budget, or whose allocation of it fails, with the shipped recursion wording. Each refusal
//! is the one the shipped evaluators give with the switch off, so every row is checked against
//! that run too, and against the decision log, so a row the machine declined cannot pass. jq spins
//! or runs out of memory on every row (an ADR-0018 rule 4(c) divergence recorded in
//! `docs/compliance/jq/limitations.md`).

use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

const SWITCH: &str = "SUCCINCTLY_CALL_MACHINE";
const LOG: &str = "SUCCINCTLY_DEBUG_CALL_MACHINE_LOG";

/// `(stdout, stderr, exit, decisions)` of `succinctly jq -nc <filter>`, with the switch on when
/// `on`, under the shell prefix `limit` (`ulimit ...`) when there is one.
fn run(
    filter: &str,
    on: bool,
    limit: Option<&str>,
    dir: &Path,
) -> Result<(String, String, i32, String)> {
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
            if on {
                command.env(SWITCH, "1");
            } else {
                command.env_remove(SWITCH);
            }
            command
        },
        None,
    )?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        exit,
        std::fs::read_to_string(&log).unwrap_or_default(),
    ))
}

/// `filter` is taken by the machine and refused with exactly `error` at exit 5, as the shipped
/// evaluators refuse it with the switch off.
fn assert_refused_as_shipped(filter: &str, error: &str, dir: &Path) -> Result<()> {
    let want = format!("jq: error (at <unknown>): {error}\n");
    let (stdout, stderr, exit, decisions) = run(filter, true, None, dir)?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), exit),
        ("", want.as_str(), 5),
        "on: {filter}"
    );
    assert!(
        !decisions.is_empty() && decisions.lines().all(|l| l == "taken"),
        "{filter}: {decisions:?}"
    );
    let (stdout, stderr, exit, _) = run(filter, false, None, dir)?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), exit),
        ("", want.as_str(), 5),
        "off: {filter}"
    );
    Ok(())
}

/// Decision 3 of #7977: a loop that never deepens keeps the shipped `MAX_LOOP_ROUNDS` refusal,
/// whatever carries it. These are the rows step 1 had to skip, as they spun on the machine
/// (`i5379`'s round-budget and collector rows, `i7739`'s `_negate`), and the `$`-argument and
/// forwarded-closure loops the machine makes tail calls of where `main` does not.
#[test]
fn a_loop_that_never_deepens_is_refused_as_the_shipped_evaluators_refuse_it_7977() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (filter, error) in [
        (
            r#"[try (def f: f; f) catch "caught"]"#,
            "f/0 exceeded maximum recursion depth",
        ),
        (
            "def _negate: -.; 1 | -.",
            "_negate/0 exceeded maximum recursion depth",
        ),
        (
            "def _plus(a; b): a + b; 1 | . + 1",
            "_plus/2 exceeded maximum recursion depth",
        ),
        (
            "def f($n): f($n); f(0)",
            "f/1 exceeded maximum recursion depth",
        ),
        (
            "def f(g): f(g); f(.)",
            "f/1 exceeded maximum recursion depth",
        ),
        // A stage in front of the call: the round is charged whatever runs before it.
        (
            "def f: select(true) | (. + 1 | f); 0 | f",
            "f/0 exceeded maximum recursion depth",
        ),
        (
            "[def f: ., (. + 1 | f); 0 | f] | length",
            "f/0: maximum outputs exceeded",
        ),
        // The prelude's loops, in a program the machine takes.
        (
            "def f: f; [0 | until(false; . + 1)]",
            "until: maximum iterations exceeded",
        ),
        (
            "def f: f; [0 | while(true; . + 1)] | length",
            "while: maximum iterations exceeded",
        ),
        // A collector holds what the loop made; the loop's cap still comes first.
        (
            "def f($n): $n, f($n + 1); [f(0)] | length",
            "f/1 exceeded maximum recursion depth",
        ),
        // `range/3` and an eager `repeat` keep the shipped caps and wording.
        (
            "def f: f; [range(0; 2e7; 1)] | length",
            "range: maximum iterations exceeded",
        ),
        (
            "def f: f; [range(2e7)] | length",
            "range: maximum iterations exceeded",
        ),
        (
            "def f: f; [repeat(1)] | length",
            "repeat: maximum iterations exceeded",
        ),
        // A helper's outputs in front of the call are the loop's.
        (
            "def g: .; def f: g, (. + 1 | f); [0 | f] | length",
            "f/0: maximum outputs exceeded",
        ),
    ] {
        assert_refused_as_shipped(filter, error, dir.path())?;
    }
    // A consumer that stops is not charged for what it never asks for.
    let (stdout, stderr, exit, _) = run(
        "first(def f: ., (. + 1 | f); 0 | f)",
        true,
        None,
        dir.path(),
    )?;
    assert_eq!((stdout.as_str(), exit), ("0\n", 0), "{stderr}");
    Ok(())
}

/// What the shipped evaluators answer, the machine answers, though it bounds its control state
/// and its loops: what `[...]` holds is not control state, a `repeat` a consumer can stop is not
/// bounded, and recursion through a closure that does not compose is not a closure chain.
#[test]
fn the_machine_answers_what_the_shipped_evaluators_answer_7977() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (filter, want) in [
        ("def f: f; [range(1e7), range(1e7)] | length", "20000000\n"),
        ("def f: f; [limit(5000; repeat(1))] | length", "5000\n"),
        ("def f: f; [limit(3; repeat(empty))]", "[]\n"),
        ("def f: f; [range(0; 1.6e7; 1)] | length", "16000000\n"),
        // An update that leaves a fork point able to answer only nothing: four million rounds
        // in constant control state, as on the shipped routes and in jq.
        (
            "def f: f; 0 | until(. >= 4000000; (. + 1) // 0)",
            "4000000\n",
        ),
        (
            "def f: f; [0 | while(. < 4000000; . + 1, empty)] | length",
            "4000000\n",
        ),
    ] {
        for on in [true, false] {
            let (stdout, stderr, exit, decisions) = run(filter, on, None, dir.path())?;
            assert_eq!(
                (stdout.as_str(), exit),
                (want, 0),
                "{filter} on: {on}: {stderr}"
            );
            if on {
                assert!(
                    !decisions.is_empty() && decisions.lines().all(|l| l == "taken"),
                    "{filter}: {decisions:?}"
                );
            }
        }
    }
    // Deeper than the shipped evaluators' native-stack floor, which refuses each: recursion
    // through `map(f)`, #7870's closure chain and a chain of two closures a level, all answered
    // as jq 1.7.1 answers them.
    for (filter, want) in [
        (
            "def f: if . > 0 then [. - 1] | map(f) | .[0] else . end; 20000 | f",
            "0\n",
        ),
        (
            "def f(g; $n): if $n <= 0 then g else f(g + (1, 3); $n - 1) end; \
             first(f((1, 3); 20000)) | type",
            "\"number\"\n",
        ),
        (
            "def f(g; $n): def h(q): f(q + 1; $n - 1); if $n == 0 then g else h(g + 1) end; \
             0 | f(.; 9000)",
            "18000\n",
        ),
    ] {
        let (stdout, stderr, exit, _) = run(filter, true, None, dir.path())?;
        assert_eq!((stdout.as_str(), exit), (want, 0), "{filter}: {stderr}");
    }
    Ok(())
}

/// A tail generator not under a collector prints the outputs the shipped loop prints, 2^20 + 1,
/// before the same refusal.
#[test]
fn a_tail_generator_prints_what_the_shipped_loop_prints_before_its_refusal_7977() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let filter = "def f: ., (. + 1 | f); 0 | f";
    let (stdout, stderr, exit, _) = run(filter, true, None, dir.path())?;
    assert_eq!(exit, 5, "{stderr}");
    assert_eq!(
        stderr,
        "jq: error (at <unknown>): f/0: maximum outputs exceeded\n"
    );
    assert_eq!(stdout.lines().count(), (1 << 20) + 1);
    assert_eq!(stdout.lines().last(), Some("1048576"));
    let (off, _, _, _) = run(filter, false, None, dir.path())?;
    assert_eq!(off, stdout);
    Ok(())
}

/// A recursion that deepens is refused at the byte budget, or at the closure depth when its
/// argument composes (`n + 1`), naming the innermost definition, where jq grows until its
/// allocation fails.
#[test]
fn a_recursion_past_the_byte_budget_is_refused_7977() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for filter in ["def f: [f]; f", "def f(n): f(n + 1) + 1; f(0)"] {
        let (stdout, stderr, exit, decisions) = run(filter, true, None, dir.path())?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), exit),
            (
                "",
                format!(
                    "jq: error (at <unknown>): f/{} exceeded maximum recursion depth\n",
                    usize::from(filter.contains("(n)"))
                )
                .as_str(),
                5
            ),
            "{filter}"
        );
        assert!(decisions.lines().all(|l| l == "taken"), "{decisions:?}");
    }
    Ok(())
}

/// #5634 and #7733 with the switch on: under a cap on the address space, and on the main thread
/// at the process limit, a runaway recursion is still refused, because the machine's stacks grow
/// fallibly (a failed allocation is a refusal, not an abort). The 1.5 GiB cap also holds every
/// budget refusal above within the acceptance bound of #7977 step 2. Linux only: macOS refuses
/// `ulimit -v` and does not count threads against `RLIMIT_NPROC`.
#[cfg(target_os = "linux")]
#[test]
fn the_machine_refuses_under_a_capped_address_space_and_at_the_thread_limit_7977() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // The caps of `i5634.rs` (a debug build reserves 2 GiB of stack, a release build 256 MiB),
    // then 1.5 GiB, then `i7733.rs`'s thread limit.
    let cap = if cfg!(debug_assertions) {
        1024 * 1024
    } else {
        128 * 1024
    };
    for limit in [
        format!("ulimit -v {cap}"),
        format!("ulimit -v {}", 1536 * 1024),
        String::from("ulimit -u 1"),
    ] {
        for filter in [
            "def f: [f]; f",
            "def f(n): f(n + 1) + 1; f(0)",
            "[def f: ., (. + 1 | f); 0 | f] | length",
        ] {
            let (stdout, stderr, exit, decisions) = run(filter, true, Some(&limit), dir.path())?;
            assert_eq!(
                (stdout.as_str(), exit),
                ("", 5),
                "{limit}: {filter}: {stderr}"
            );
            assert!(
                stderr.contains("exceeded maximum recursion depth")
                    || stderr.contains("maximum outputs exceeded"),
                "{limit}: {filter}: {stderr}"
            );
            assert!(decisions.lines().all(|l| l == "taken"), "{decisions:?}");
        }
    }
    Ok(())
}
