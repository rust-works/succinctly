//! #5507: `succinctly yq --on-error=continue|stop`, the flag #5471 gave `succinctly jq`.
//!
//! Without the flag yq's rule holds (#2810): the first error the program raises ends the run, the
//! documents and files after it are never evaluated, and what the earlier ones printed stays.
//! `stop` is that rule spelled out. `continue` is a succinctly extension (ADR-0018 rule 5), since
//! yq has no reference behaviour for carrying on: it evaluates every document and file, reports
//! every error and exits 1 if any failed. Under `--inplace`, which already carries on past a
//! failed file and leaves it untouched (#1615), `stop` edits no file after the first failure.

use std::io::Write;

use anyhow::Result;
use tempfile::{NamedTempFile, TempDir};

use crate::{run_yq_files, run_yq_stdin_with_stderr};

const FILTER: &str = ".a | tonumber";

fn errors(stderr: &str) -> usize {
    stderr
        .lines()
        .filter(|line| line.starts_with("Error:"))
        .count()
}

/// (input, default = stop's stdout, continue's stdout, errors under continue)
type Row<'a> = (&'a str, &'a str, &'a str, usize);

/// The error comes early, in the middle and last of a three-document stream.
const STREAM_ROWS: &[Row] = &[
    ("a: x\n---\na: 1\n---\na: 2\n", "", "1\n---\n2\n", 1),
    ("a: 1\n---\na: x\n---\na: 2\n", "1\n", "1\n---\n2\n", 1),
    (
        "a: 1\n---\na: 2\n---\na: x\n",
        "1\n---\n2\n",
        "1\n---\n2\n",
        1,
    ),
    // Two failing documents are two diagnostics under `continue`, one by default.
    ("a: x\n---\na: 1\n---\na: y\n", "", "1\n", 2),
];

/// Every stdout route: the streamed one, the evaluated one (JSON output, `-P`, `--sort-keys`)
/// and the DOM fallback a variable forces (`--arg`).
const ROUTES: &[&[&str]] = &[
    &[],
    &["-o=json", "-I=0"],
    &["-P"],
    &["--sort-keys"],
    &["--arg", "v", "1"],
];

fn with_flag<'a>(route: &[&'a str], flag: &'a str) -> Vec<&'a str> {
    let mut args = route.to_vec();
    if !flag.is_empty() {
        args.push(flag);
    }
    args
}

/// JSON output separates documents by newline instead of `---`.
fn for_route(stdout: &str, route: &[&str]) -> String {
    if route.contains(&"-o=json") {
        stdout.replace("---\n", "")
    } else {
        stdout.to_string()
    }
}

#[test]
fn test_5507_default_and_stop_end_the_run_and_continue_does_not() -> Result<()> {
    for route in ROUTES {
        for (input, stops, continues, continue_errors) in STREAM_ROWS {
            for flag in ["", "--on-error=stop"] {
                let (out, err, code) =
                    run_yq_stdin_with_stderr(FILTER, input, &with_flag(route, flag))?;
                assert_eq!(
                    (out.as_str(), code, errors(&err)),
                    (for_route(stops, route).as_str(), 1, 1),
                    "route {route:?} flag {flag:?} input {input:?}: {err}"
                );
            }
            let (out, err, code) =
                run_yq_stdin_with_stderr(FILTER, input, &with_flag(route, "--on-error=continue"))?;
            assert_eq!(
                (out.as_str(), code, errors(&err)),
                (for_route(continues, route).as_str(), 1, *continue_errors),
                "route {route:?} continue input {input:?}: {err}"
            );
        }
    }
    Ok(())
}

#[test]
fn test_5507_the_flag_changes_nothing_when_no_program_error_is_raised() -> Result<()> {
    let input = "a: 1\n---\na: 2\n";
    for flag in ["", "--on-error=stop", "--on-error=continue"] {
        let (out, err, code) = run_yq_stdin_with_stderr(FILTER, input, &with_flag(&[], flag))?;
        assert_eq!((out.as_str(), err.as_str(), code), ("1\n---\n2\n", "", 0));
    }
    Ok(())
}

#[test]
fn test_5507_json_documents_in_a_stream_follow_the_flag_too() -> Result<()> {
    let input = "{\"a\":\"x\"}\n{\"a\":1}\n{\"a\":2}\n";
    let args = |flag: &'static str| ["-p", "json", flag];
    let (out, _, code) = run_yq_stdin_with_stderr(FILTER, input, &args("--on-error=stop"))?;
    assert_eq!((out.as_str(), code), ("", 1));
    let (out, err, code) = run_yq_stdin_with_stderr(FILTER, input, &args("--on-error=continue"))?;
    assert_eq!((out.as_str(), code, errors(&err)), ("1\n---\n2\n", 1, 1));
    Ok(())
}

/// `-e` reports the failure, not "no matches found": an uncaught error outranks it either way.
#[test]
fn test_5507_exit_status_flag_still_reports_the_error_under_continue() -> Result<()> {
    let (out, err, code) = run_yq_stdin_with_stderr(
        FILTER,
        "a: 1\n---\na: x\n---\na: 2\n",
        &["-e", "--on-error=continue"],
    )?;
    assert_eq!((out.as_str(), code, errors(&err)), ("1\n---\n2\n", 1, 1));
    assert!(!err.contains("no matches found"), "{err}");
    Ok(())
}

fn file(contents: &str) -> Result<NamedTempFile> {
    let mut f = NamedTempFile::new()?;
    write!(f, "{contents}")?;
    Ok(f)
}

/// Files, with the failing one first, in the middle and last. By default the files after the
/// failing one are never read; under `continue` every file is.
#[test]
fn test_5507_files_after_a_failing_one_are_read_only_under_continue() -> Result<()> {
    let ok1 = file("a: 1\n")?;
    let bad = file("a: x\n")?;
    let ok2 = file("a: 2\n")?;
    let rows: [(&[&NamedTempFile], &str, &str); 3] = [
        (&[&bad, &ok1, &ok2], "", "1\n---\n2\n"),
        (&[&ok1, &bad, &ok2], "1\n", "1\n---\n2\n"),
        (&[&ok1, &ok2, &bad], "1\n---\n2\n", "1\n---\n2\n"),
    ];
    for (files, stops, continues) in rows {
        let paths: Vec<&std::path::Path> = files.iter().map(|f| f.path()).collect();
        for flag in ["", "--on-error=stop"] {
            let (out, err, code) = run_yq_files(FILTER, &paths, &with_flag(&[], flag))?;
            assert_eq!((out.as_str(), code, errors(&err)), (stops, 1, 1), "{flag}");
        }
        let (out, err, code) = run_yq_files(FILTER, &paths, &["--on-error=continue"])?;
        assert_eq!((out.as_str(), code, errors(&err)), (continues, 1, 1));
    }
    Ok(())
}

/// `--inplace` carries on past a failed file by default and leaves that file untouched (#1615);
/// `stop` also edits nothing after it, and `continue` is the default there.
#[test]
fn test_5507_inplace_stop_edits_no_file_after_a_failed_one() -> Result<()> {
    let update = ".a |= (tonumber + 1)";
    for (flag, second) in [
        ("", "a: 2\n"),
        ("--on-error=continue", "a: 2\n"),
        ("--on-error=stop", "a: 1\n"),
    ] {
        let dir = TempDir::new()?;
        let bad = dir.path().join("bad.yaml");
        let good = dir.path().join("good.yaml");
        std::fs::write(&bad, "a: x\n")?;
        std::fs::write(&good, "a: 1\n")?;
        let (_, err, code) =
            run_yq_files(update, &[&bad, &good], &with_flag(&["--inplace"], flag))?;
        assert_eq!(code, 1, "{flag}: {err}");
        assert_eq!(std::fs::read_to_string(&bad)?, "a: x\n", "{flag}");
        assert_eq!(std::fs::read_to_string(&good)?, second, "{flag}");
    }
    Ok(())
}

/// `--split-exp` already stops at the first error (#2810); `continue` writes the files after it.
#[test]
fn test_5507_split_exp_writes_the_files_after_an_error_only_under_continue() -> Result<()> {
    let input = "a: 1\n---\na: x\n---\na: 2\n";
    for (flag, written) in [
        ("--on-error=stop", vec!["1"]),
        ("--on-error=continue", vec!["1", "2"]),
    ] {
        let dir = TempDir::new()?;
        let pattern = format!("\"{}/out_\" + (. | tostring)", dir.path().display());
        let (_, err, code) =
            run_yq_stdin_with_stderr(FILTER, input, &["--split-exp", &pattern, flag])?;
        assert_eq!((code, errors(&err)), (1, 1), "{flag}: {err}");
        for n in ["1", "2"] {
            let path = dir.path().join(format!("out_{n}"));
            assert_eq!(path.exists(), written.contains(&n), "{flag} out_{n}");
        }
    }
    Ok(())
}

#[test]
fn test_5507_an_unknown_mode_is_refused() -> Result<()> {
    let (out, err, code) = run_yq_stdin_with_stderr(FILTER, "a: 1\n", &["--on-error=bogus"])?;
    assert_eq!((out.as_str(), code), ("", 2));
    assert!(err.contains("possible values: continue, stop"), "{err}");
    Ok(())
}
