//! #7977: the call machine, behind `SUCCINCTLY_CALL_MACHINE`, off by default.
//!
//! The corpus is `scripts/call_machine/cases.py`: 109 programs from the #7957 spikes and 33 from
//! the reviews of #7984 (standard input, several documents, exit statuses, flags, side effects in
//! leaves, pattern names and keys, `range`'s first value), with jq 1.7.1's stdout, stderr and exit status captured into
//! `tests/data/call-machine-corpus-7977.json` by `scripts/call_machine/check.py capture`
//! (`/usr/bin/jq` 1.7.1, macOS arm64). With the switch on, every answer is jq's and every "take"
//! program reaches the machine; with it unset, the machine is never asked, so a program runs as on
//! a build without it.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::{spawn_with_signal_retry, succinctly_bin};

const SWITCH: &str = "SUCCINCTLY_CALL_MACHINE";
const LOG: &str = "SUCCINCTLY_DEBUG_CALL_MACHINE_LOG";

/// Linear recursion 100,000 deep: the shipped evaluators refuse it at their native-stack floor
/// (jq answers it), and the machine answers it with constant native stack.
const DEEP: &str = "def f($n): if $n == 0 then 0 else f($n - 1) + 1 end; f(100000)";

struct Answer {
    stdout: String,
    stderr: String,
    exit: i32,
    /// The machine's routing decisions and compilations, one per line (`taken`, `declined` or
    /// `compiled`), in the order they happened.
    decisions: String,
}

impl Answer {
    /// The routing decisions alone.
    fn routes(&self) -> Vec<&str> {
        self.decisions
            .lines()
            .filter(|line| *line != "compiled")
            .collect()
    }

    /// How many times the run compiled a program for the machine.
    fn compilations(&self) -> usize {
        self.decisions
            .lines()
            .filter(|line| *line == "compiled")
            .count()
    }
}

/// Run `succinctly jq <args>` with the switch set to `switch` (`None`: removed from the
/// environment) and the decision log in `dir`.
fn run(args: &[&str], switch: Option<&str>, dir: &Path) -> Result<Answer> {
    run_with_stdin(args, switch, dir, None)
}

/// [`run`], feeding `stdin` to the run (`None`: no standard input).
fn run_with_stdin(
    args: &[&str],
    switch: Option<&str>,
    dir: &Path,
    stdin: Option<&str>,
) -> Result<Answer> {
    let log = dir.join("decisions.log");
    // A retry after a signal death must not read the decisions of the attempt before.
    let (output, exit) = spawn_with_signal_retry(
        || {
            let _ = std::fs::remove_file(&log);
            let mut command = Command::new(succinctly_bin());
            command.arg("jq").args(args).env(LOG, &log);
            match switch {
                Some(value) => command.env(SWITCH, value),
                None => command.env_remove(SWITCH),
            };
            command
        },
        stdin.map(str::as_bytes),
    )?;
    Ok(Answer {
        stdout: String::from_utf8(output.stdout)?,
        stderr: String::from_utf8(output.stderr)?,
        exit,
        decisions: std::fs::read_to_string(&log).unwrap_or_default(),
    })
}

#[test]
fn the_corpus_matches_jq_on_the_call_machine_7977() -> Result<()> {
    let golden =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/call-machine-corpus-7977.json");
    let text = std::fs::read_to_string(&golden).context("the golden corpus")?;
    let rows: Vec<serde_json::Value> = serde_json::from_str(&text)?;
    assert_eq!(
        rows.len(),
        142,
        "the corpus of scripts/call_machine/cases.py"
    );
    let dir = tempfile::tempdir()?;
    let mut wrong = Vec::new();
    for row in &rows {
        let id = row["id"].as_str().context("id")?;
        let mut args: Vec<&str> = row["args"]
            .as_array()
            .context("args")?
            .iter()
            .map(|a| a.as_str().unwrap_or_default())
            .collect();
        args.push(row["filter"].as_str().context("filter")?);
        let stdin = row["stdin"].as_str();
        let got = run_with_stdin(&args, Some("1"), dir.path(), stdin)?;
        let want = (
            row["stdout"].as_str().context("stdout")?,
            row["stderr"].as_str().context("stderr")?,
            row["exit"].as_i64().context("exit")?,
        );
        if (
            got.stdout.as_str(),
            got.stderr.as_str(),
            i64::from(got.exit),
        ) != want
        {
            wrong.push(format!(
                "{id}: want {want:?}, got {:?}",
                (&got.stdout, &got.stderr, got.exit)
            ));
        }
        // The run decides once, the route the corpus says, whatever its number of documents,
        // and compiles once for a program the machine takes (#7977 step 5); a program jq
        // rejects at compile time is never routed.
        let routed_as = match row["route"].as_str() {
            Some("take") => Some("taken"),
            Some("decline") => Some("declined"),
            _ => None,
        };
        let routed = match routed_as {
            Some(route) => {
                got.routes() == [route]
                    && match route {
                        "taken" => got.compilations() == 1,
                        _ => got.compilations() <= 1,
                    }
            }
            None => got.decisions.is_empty(),
        };
        if !routed {
            wrong.push(format!(
                "{id}: want {routed_as:?}, decided {:?}",
                got.decisions
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {}:\n{}",
        wrong.len(),
        rows.len(),
        wrong.join("\n")
    );
    Ok(())
}

#[test]
fn with_the_switch_off_the_machine_is_never_asked_7977() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let on = run(&["-n", DEEP], Some("1"), dir.path())?;
    assert_eq!(
        (on.stdout.as_str(), on.exit),
        ("100000\n", 0),
        "{}",
        on.stderr
    );
    // Decided, and compiled, once, as the route is pinned: the code is kept with the pin and
    // runs the document (#7977 step 5). Before, the first document decided again and `run`
    // compiled once more.
    assert_eq!(on.decisions, "compiled\ntaken\n");
    for switch in [None, Some("0"), Some("")] {
        let off = run(&["-n", DEEP], switch, dir.path())?;
        assert_eq!(off.stdout, "", "{switch:?}");
        assert_eq!(off.exit, 5, "{switch:?}: {}", off.stderr);
        assert!(
            off.stderr.contains("exceeded maximum recursion depth"),
            "{switch:?}: {}",
            off.stderr
        );
        assert_eq!(off.decisions, "", "{switch:?}: no decision is made");
    }
    Ok(())
}

#[test]
fn a_program_the_machine_declines_keeps_the_shipped_route_7977() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // `last` is not the machine's yet; jq answers 2.
    let got = run(
        &[
            "-n",
            "def f($n): if $n == 0 then 1, 2 else f($n - 1) end; last(f(3))",
        ],
        Some("1"),
        dir.path(),
    )?;
    assert_eq!(
        (got.stdout.as_str(), got.exit),
        ("2\n", 0),
        "{}",
        got.stderr
    );
    assert_eq!(got.routes(), ["declined"], "{:?}", got.decisions);
    Ok(())
}

/// #7977 step 5: a routed program over 100,000 NDJSON records decides once and compiles once,
/// as its route is pinned, and every record runs the kept code. Before, each record compiled the
/// program again (100,002 compilations, 2 decisions). The answers are jq 1.7.1's: each record's
/// label value counts on from the record before, as jq numbers labels across inputs.
#[test]
fn a_routed_program_compiles_once_over_100000_records_7977() -> Result<()> {
    const RECORDS: usize = 100_000;
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("records.ndjson");
    let mut text = String::with_capacity(RECORDS * 8);
    for i in 0..RECORDS {
        text.push_str(&format!("{}\n", i % 7));
    }
    std::fs::write(&input, text)?;
    let path = input.to_str().context("utf-8 path")?;
    let filter = "def f($n): if $n == 0 then . else f($n - 1) end; \
                  f(. % 3) as $x | [label $l | try break $l catch .] | [$x, .[0].__jq]";
    let got = run(&["-c", filter, path], Some("1"), dir.path())?;
    assert_eq!(got.exit, 0, "{}", got.stderr);
    let lines: Vec<&str> = got.stdout.lines().collect();
    assert_eq!(lines.len(), RECORDS);
    // jq 1.7.1 on the first three records, and on the last.
    assert_eq!(&lines[..3], ["[0,0]", "[1,1]", "[2,2]"]);
    assert_eq!(
        lines[RECORDS - 1],
        format!("[{},{}]", (RECORDS - 1) % 7, RECORDS - 1)
    );
    assert_eq!(got.decisions, "compiled\ntaken\n");
    Ok(())
}
