//! #7977 step 3: the call machine's leaves that collect `.[]` and `range(n)`, with
//! `SUCCINCTLY_CALL_MACHINE=1`. Each row is checked against the run with the switch off, and
//! against the decision log, so a row the machine declined cannot pass.

use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::{spawn_with_signal_retry, succinctly_bin};

const SWITCH: &str = "SUCCINCTLY_CALL_MACHINE";
const LOG: &str = "SUCCINCTLY_DEBUG_CALL_MACHINE_LOG";

/// `(stdout, stderr, exit, decisions)` of `succinctly jq -c <filter>` over `stdin` (`-n` when
/// none), with the switch on when `on`. `decisions` holds the decision log's routing lines.
fn run(
    filter: &str,
    stdin: Option<&str>,
    on: bool,
    dir: &Path,
) -> Result<(String, String, i32, String)> {
    let log = dir.join("decisions.log");
    let (output, exit) = spawn_with_signal_retry(
        || {
            let _ = std::fs::remove_file(&log);
            let mut command = Command::new(succinctly_bin());
            command
                .arg("jq")
                .arg(if stdin.is_some() { "-c" } else { "-nc" });
            command.arg(filter).env(LOG, &log);
            if on {
                command.env(SWITCH, "1");
            } else {
                command.env_remove(SWITCH);
            }
            command
        },
        stdin.map(str::as_bytes),
    )?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        exit,
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .filter(|line| *line != "compiled")
            .map(|line| format!("{line}\n"))
            .collect(),
    ))
}

/// `filter` is taken by the machine and answers `(stdout, stderr, exit)`, as it does with the
/// switch off.
fn assert_as_shipped(
    filter: &str,
    stdin: Option<&str>,
    want: (&str, &str, i32),
    dir: &Path,
) -> Result<()> {
    let (stdout, stderr, exit, decisions) = run(filter, stdin, true, dir)?;
    assert_eq!(decisions, "taken\n", "{filter}");
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), exit),
        want,
        "on: {filter}"
    );
    let (stdout, stderr, exit, _) = run(filter, stdin, false, dir)?;
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), exit),
        want,
        "off: {filter}"
    );
    Ok(())
}

/// A leaf's `range` is the path-mode interpreter's, which keeps the shipped `MAX_RANGE_TOTAL`
/// cap (#5292) with the shipped wording: 2^24 numbers answer, and one more is refused, as with
/// the switch off. jq 1.7.1 has no such cap.
#[test]
fn a_leafs_range_keeps_the_shipped_cap_7977() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = "def r: if false then r else . end; ";
    assert_as_shipped(
        &format!("{r}[range(16777216)] | length"),
        None,
        ("16777216\n", "", 0),
        dir.path(),
    )?;
    assert_as_shipped(
        &format!("{r}[range(16777217)] | length"),
        None,
        (
            "",
            "jq: error (at <unknown>): range: maximum iterations exceeded\n",
            5,
        ),
        dir.path(),
    )
}

/// A pattern that binds a name twice keeps the first store that runs, the last element of an
/// array pattern, as jq does, inside what would otherwise be a leaf: the path-mode interpreter
/// keeps the last store, so the machine compiles such a pattern itself (#7977 step 3 review).
/// Before, the machine answered `[[1]]` and `[[],[[5]],[{"x":7}]]` here.
#[test]
fn a_pattern_that_binds_a_name_twice_keeps_its_first_store_in_a_leaf_7977() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let r = "def r: if false then r else . end; ";
    assert_as_shipped(
        &format!("{r}[[1,2]] | [.[] | [(. as [$x, $x] | $x)?]]"),
        None,
        ("[[2]]\n", "", 0),
        dir.path(),
    )?;
    assert_as_shipped(
        &format!("{r}[.[] | [(. as [$x, $x] | $x)?]]"),
        Some(r#"[{"a":{"x":9},"b":2,"x":3,"y":4},[[5],[6]],[{"x":7},8]]"#),
        ("[[],[[6]],[8]]\n", "", 0),
        dir.path(),
    )
}
