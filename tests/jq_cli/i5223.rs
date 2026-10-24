//! #5223: `succinctly dev jq-path-mode`, the dev-only entry to the register-carrying
//! path-mode interpreter (`src/jq/path_mode.rs`). Its own semantics are pinned by its unit
//! tests; these pin the entry's contract with the three-way harness
//! (`scripts/jq-path-mode-3way.py`): jq's input handling and exit codes, and the
//! `path-mode: unsupported` line it classifies rows by. Expected
//! stdout and exit codes are jq 1.7.1's for the same argv.
//!
//! The interpreter reads storage identity, which the `unshared-containers` holdout does not
//! have, so these pin the shipped shape only.
#![cfg(not(feature = "unshared-containers"))]

use std::process::Command;

use anyhow::Result;

use crate::{assert_path_rows_both_routes_3749, spawn_with_signal_retry, succinctly_bin};

/// Run the dev entry with `args` (options, then the filter), returning stdout, stderr and the
/// exit code.
fn dev_entry(args: &[&str], stdin: Option<&str>) -> Result<(String, String, i32)> {
    let (output, code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(succinctly_bin());
            command.args(["dev", "jq-path-mode"]).args(args);
            command
        },
        stdin.map(str::as_bytes),
    )?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        code,
    ))
}

/// Every input value runs in turn, the exit status is the last input's, and a malformed value
/// ends the read with jq's status 5 after the values before it.
#[test]
fn the_dev_entry_reads_inputs_as_jq_5223() -> Result<()> {
    for (input, filter, stdout, code) in [
        ("1 2", ". + 1", "2\n3\n", 0),
        (r#""x" 3"#, ". + 1", "4\n", 0),
        (r#"3 "x""#, ". + 1", "4\n", 5),
        ("{", ".", "", 5),
        ("1 [1,", ".", "1\n", 5),
        (" \n", ".", "", 0),
        (r#"{"a":[1,2]}"#, "path(.a[])", "[\"a\",0]\n[\"a\",1]\n", 0),
    ] {
        let (out, _, rc) = dev_entry(&["-c", "--", filter], Some(input))?;
        assert_eq!((out.as_str(), rc), (stdout, code), "{input} | {filter}");
    }
    Ok(())
}

/// `-r`, `-n`, a file argument, an error payload that is not a string, and `halt`.
#[test]
fn the_dev_entry_takes_jqs_options_and_reports_its_errors_5223() -> Result<()> {
    let (out, _, rc) = dev_entry(&["-r", ".a"], Some(r#"{"a":"x"}"#))?;
    assert_eq!((out.as_str(), rc), ("x\n", 0));
    let (out, _, rc) = dev_entry(&["-n", "-c", "[., 1]"], None)?;
    assert_eq!((out.as_str(), rc), ("[null,1]\n", 0));
    let (out, err, rc) = dev_entry(&["-n", "error({})"], None)?;
    assert_eq!((out.as_str(), rc), ("", 5));
    assert!(err.contains("(not a string): {}"), "{err}");
    let (out, err, rc) = dev_entry(&["-n", r#"error("boom")"#], None)?;
    assert_eq!((out.as_str(), rc), ("", 5));
    assert!(err.contains("jq: error (at <unknown>): boom"), "{err}");
    let (out, _, rc) = dev_entry(&["-n", "1, halt, 2"], None)?;
    assert_eq!((out.as_str(), rc), ("1\n", 0));

    let mut file = tempfile::NamedTempFile::new()?;
    std::io::Write::write_all(&mut file, br#"{"a":1}"#)?;
    let path = file.path().to_str().expect("utf-8 temp path");
    let (out, _, rc) = dev_entry(&["-c", "path(.a)", path], None)?;
    assert_eq!((out.as_str(), rc), ("[\"a\"]\n", 0));
    let (_, _, rc) = dev_entry(&[".", "/nonexistent/5223.json"], None)?;
    assert_eq!(rc, 2);
    Ok(())
}

/// The lines the three-way harness classifies by: a declined construct exits 64 with
/// `path-mode: unsupported`, and a compile error exits 3 as jq does.
#[test]
fn the_dev_entry_marks_declined_runs_5223() -> Result<()> {
    let (_, err, rc) = dev_entry(&["-n", "at_offset(0)"], None)?;
    assert_eq!(rc, 64);
    assert!(err.contains("path-mode: unsupported: "), "{err}");
    let (_, _, rc) = dev_entry(&["-n", "$a"], None)?;
    assert_eq!(rc, 3);
    let (out, err, rc) = dev_entry(&["-c", r#"[scan("x")]"#], Some(r#""x""#))?;
    assert_eq!((out.as_str(), rc), ("[\"x\"]\n", 0));
    assert!(err.is_empty(), "{err}");
    // A C builtin that hands back its input keeps the path, as in jq 1.7.1.
    let (out, err, rc) = dev_entry(&["-c", r#"path(.a | ltrimstr("z"))"#], Some(r#"{"a":"x"}"#))?;
    assert_eq!((out.as_str(), rc), ("[\"a\"]\n", 0));
    assert!(err.is_empty(), "{err}");
    Ok(())
}

/// `input` and `inputs` draw from the same queue the dev entry's own inputs come from, and
/// `halt_error` writes its message and halts with jq's status. Stdout and exit codes are
/// jq 1.7.1's.
#[test]
fn the_dev_entry_reads_the_input_queue_and_halts_as_jq_5223() -> Result<()> {
    let (out, err, rc) = dev_entry(&["-c", "[., input]"], Some("1 2 3"))?;
    assert_eq!((out.as_str(), rc), ("[1,2]\n", 5));
    assert!(err.contains("break"), "{err}");
    let (out, _, rc) = dev_entry(&["-n", "-c", "[inputs]"], Some("1 2 3"))?;
    assert_eq!((out.as_str(), rc), ("[1,2,3]\n", 0));
    let (out, err, rc) = dev_entry(&["-n", r#""bye\n" | halt_error"#], None)?;
    assert_eq!((out.as_str(), err.as_str(), rc), ("", "bye\n", 5));
    let (_, err, rc) = dev_entry(&["-n", r#"{"a":1} | halt_error(3)"#], None)?;
    assert_eq!((err.as_str(), rc), ("{\"a\":1}\n", 3));
    Ok(())
}

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5223() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":{"b":1}}"#,
            r"path(reduce (.a,.a) as $x (.; .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":{\"b\":1}}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(reduce (.a,.a) as $x (.; .a))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":{\"b\":1}}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(reduce (.a,.a) as $x (.; .a)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"a\":{\"b\":1}}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(reduce .a as $v (.; .))",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(reduce .a as $v (.; .))",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(reduce .a as $v (.; .)) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(foreach (1,2) as $x (.; .a; .b))",
            "[\"a\",\"b\"]\n",
            "Invalid path expression near attempt to access element \"a\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(foreach (1,2) as $x (.; .a; .b))",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(foreach (1,2) as $x (.; .a; .b)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"a\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(foreach .a as $v (.; .; .))",
            "",
            "Invalid path expression with result {\"a\":{\"b\":1}}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(foreach .a as $v (.; .; .))",
            "",
            "Invalid path expression with result {\"a\":{\"b\":1}}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(foreach .a as $v (.; .; .)) |= 9",
            "",
            "Invalid path expression with result {\"a\":{\"b\":1}}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(.a as {b:$v} | .)",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(.a as {b:$v} | .)",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(.a as {b:$v} | .) |= 9",
            "",
            "Invalid path expression near attempt to access element \"b\" of {\"b\":1}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path([{} | .a])",
            "",
            "Invalid path expression near attempt to access element \"a\" of {}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del([{} | .a])",
            "",
            "Invalid path expression near attempt to access element \"a\" of {}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"([{} | .a]) |= 9",
            "",
            "Invalid path expression near attempt to access element \"a\" of {}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | [.a] | $x)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | [.a] | $x)",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | [.a] | $x) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | try (.a | error) catch $x)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | try (.a | error) catch $x)",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | try (.a | error) catch $x) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | (.a | empty) // $x)",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | (.a | empty) // $x)",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | (.a | empty) // $x) |= 9",
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | .a | (1 + (.b)) | $x)",
            "",
            "Invalid path expression with result {\"a\":{\"b\":1}}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | .a | (1 + (.b)) | $x)",
            "",
            "Invalid path expression with result {\"a\":{\"b\":1}}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | .a | (1 + (.b)) | $x) |= 9",
            "",
            "Invalid path expression with result {\"a\":{\"b\":1}}",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"path(.a | {c:2} | getpath(["c"]))"#,
            "",
            "Invalid path expression with result 2",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"del(.a | {c:2} | getpath(["c"]))"#,
            "",
            "Invalid path expression with result 2",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r#"(.a | {c:2} | getpath(["c"])) |= 9"#,
            "",
            "Invalid path expression with result 2",
            5,
        ),
        (
            r#"{"a":"x"}"#,
            r#"path(.a | ltrimstr("z"))"#,
            "[\"a\"]\n",
            "",
            0,
        ),
        (r#"{"a":"x"}"#, r#"del(.a | ltrimstr("z"))"#, "{}\n", "", 0),
        (
            r#"{"a":"x"}"#,
            r#"(.a | ltrimstr("z")) |= 9"#,
            "{\"a\":9}\n",
            "",
            0,
        ),
        (
            r#"{"a":"x"}"#,
            r#"path(.a | ltrimstr("x"))"#,
            "",
            "Invalid path expression with result \"\"",
            5,
        ),
        (
            r#"{"a":"x"}"#,
            r#"del(.a | ltrimstr("x"))"#,
            "",
            "Invalid path expression with result \"\"",
            5,
        ),
        (
            r#"{"a":"x"}"#,
            r#"(.a | ltrimstr("x")) |= 9"#,
            "",
            "Invalid path expression with result \"\"",
            5,
        ),
        (
            r#"{"a":"x"}"#,
            r"path(.a | ascii_downcase)",
            "",
            "Invalid path expression near attempt to iterate through [120]",
            5,
        ),
        (
            r#"{"a":"x"}"#,
            r"del(.a | ascii_downcase)",
            "",
            "Invalid path expression near attempt to iterate through [120]",
            5,
        ),
        (
            r#"{"a":"x"}"#,
            r"(.a | ascii_downcase) |= 9",
            "",
            "Invalid path expression near attempt to iterate through [120]",
            5,
        ),
        (
            r#"{"a":1,"b":1}"#,
            r"path(.a as $v | .b | select(. == $v) | $v)",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":1,"b":1}"#,
            r"del(.a as $v | .b | select(. == $v) | $v)",
            "",
            "Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":1,"b":1}"#,
            r"(.a as $v | .b | select(. == $v) | $v) |= 9",
            "",
            "Invalid path expression with result 1",
            5,
        ),
    ])
}
