//! #2810: yq stops at the first error the program raises, and keeps what earlier documents printed.
//!
//! The issue assumed yq discards a whole run's output on a late error. It does not: its example
//! (`select(.a==2) | error("x")`) fails in document 1 as well, because yq's `error` fires even
//! when `select` yields nothing. Every row below fails in one document only, captured from yq
//! v4.53.3.

use std::io::Write;

use anyhow::Result;
use tempfile::{NamedTempFile, TempDir};

use crate::{run_yq_files, run_yq_stdin_with_stderr};

fn error_lines(stderr: &str) -> usize {
    stderr
        .lines()
        .filter(|line| line.starts_with("Error:"))
        .count()
}

/// An error in a later document leaves the earlier documents' output in place: yq prints `1`,
/// `---`, `2` and then fails on the third document. The error is the only line of its kind.
#[test]
fn test_2810_a_later_documents_error_keeps_the_earlier_documents_output() -> Result<()> {
    let input = "a: 1\n---\na: 2\n---\na: x\n";
    for (filter, extra, want) in [
        (".a | tonumber", &[][..], "1\n---\n2\n"),
        (".a | tonumber", &["-o=json", "-I=0"][..], "1\n2\n"),
        (".a | tonumber", &["-P"][..], "1\n---\n2\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, input, extra)?;
        assert_eq!(code, 1, "`{filter}` {extra:?}: {stderr}");
        assert_eq!(stdout, want, "`{filter}` {extra:?}");
        assert_eq!(error_lines(&stderr), 1, "`{filter}` {extra:?}: {stderr}");
    }
    Ok(())
}

/// The first error ends the run: no later document is evaluated, so a failure in the first one
/// leaves nothing on stdout and one diagnostic, where each later document used to print too.
#[test]
fn test_2810_the_first_error_ends_the_run() -> Result<()> {
    let cases: &[(&str, &str, &[&str])] = &[
        (".a | tonumber", "a: x\n---\na: 1\n---\na: 2\n", &[]),
        (".a | tonumber", "a: x\n---\na: 1\n---\na: 2\n", &["-P"]),
        (
            ".a | tonumber",
            "a: x\n---\na: 1\n---\na: 2\n",
            &["-o=json"],
        ),
        (".a | keys", "a: 5\n---\na: {k: 1}\n", &[]),
        (
            r".a | keys | .[] | tonumber",
            "a: {k: 1}\n---\na: {2: 1}\n",
            &[],
        ),
    ];
    for (filter, input, extra) in cases {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, input, extra)?;
        assert_eq!(code, 1, "`{filter}` {extra:?}: {stderr}");
        assert_eq!(stdout, "", "`{filter}` {extra:?}");
        assert_eq!(error_lines(&stderr), 1, "`{filter}` {extra:?}: {stderr}");
    }
    // An error that is raised in two documents is still reported once.
    let (_, stderr, code) =
        run_yq_stdin_with_stderr(r#".missing // error("boom")"#, "a: 1\n---\nb: 2\n", &[])?;
    assert_eq!(code, 1);
    assert_eq!(stderr.matches("boom").count(), 1, "{stderr}");
    Ok(())
}

/// The run is across files too: a failing second file keeps the first file's output and the
/// third file is never read; a failing first file prints nothing of the others.
#[test]
fn test_2810_an_error_in_a_file_ends_the_run_across_files() -> Result<()> {
    let mut good = NamedTempFile::new()?;
    writeln!(good, "a: 1")?;
    let mut bad = NamedTempFile::new()?;
    writeln!(bad, "a: x")?;
    let mut later = NamedTempFile::new()?;
    writeln!(later, "a: 3")?;
    for extra in [&[][..], &["-P"][..]] {
        let (stdout, stderr, code) = run_yq_files(
            ".a | tonumber",
            &[good.path(), bad.path(), later.path()],
            extra,
        )?;
        assert_eq!((code, stdout.as_str()), (1, "1\n"), "{extra:?}: {stderr}");
        assert_eq!(error_lines(&stderr), 1, "{extra:?}: {stderr}");

        let (stdout, stderr, code) = run_yq_files(
            ".a | tonumber",
            &[bad.path(), good.path(), later.path()],
            extra,
        )?;
        assert_eq!((code, stdout.as_str()), (1, ""), "{extra:?}: {stderr}");
        assert_eq!(error_lines(&stderr), 1, "{extra:?}: {stderr}");
    }
    Ok(())
}

/// Runs that raise no error are unchanged, and a failure to decode a value is not an error the
/// program raised: the document after it is still reached (#355, #1615, #1817).
#[test]
fn test_2810_runs_without_a_program_error_are_unchanged() -> Result<()> {
    let (stdout, _, code) = run_yq_stdin_with_stderr(".a | tonumber", "a: 1\n---\na: 2\n", &[])?;
    assert_eq!((code, stdout.as_str()), (0, "1\n---\n2\n"));

    let (stdout, _, code) =
        run_yq_stdin_with_stderr(r"select(.a == 2) | .a", "a: 1\n---\na: 2\n---\na: 3\n", &[])?;
    assert_eq!((code, stdout.as_str()), (0, "2\n"));

    let (stdout, stderr, code) = run_yq_stdin_with_stderr(
        ".",
        "a: 1\n---\nb: \"bad \\q escape\"\n---\nc: 3\n",
        &["--arg", "z", "y"],
    )?;
    assert_eq!(code, 1, "{stderr}");
    assert!(
        stdout.contains("a: 1\n") && stdout.contains("c: 3\n"),
        "{stdout:?}"
    );
    Ok(())
}

/// `--split-exp` stops at the first error too: yq writes `out_0.yml` for the first document and
/// nothing for the third (v4.53.3, `a: 1`, `a: x`, `a: 3`), and with the failure first it writes
/// no file at all.
#[test]
fn test_2810_split_exp_stops_at_the_first_error() -> Result<()> {
    for (input, files) in [
        ("a: 1\n---\na: x\n---\na: 3\n", vec!["out_0.yml"]),
        ("a: x\n---\na: 1\n", vec![]),
    ] {
        let dir = TempDir::new()?;
        let pattern = format!(
            "\"{}/out_\" + ($index|tostring) + \".yml\"",
            dir.path().display()
        );
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(".a | tonumber", input, &["--split-exp", &pattern])?;
        assert_eq!((code, stdout.as_str()), (1, ""), "{stderr}");
        assert_eq!(error_lines(&stderr), 1, "{stderr}");
        let mut written: Vec<String> = std::fs::read_dir(dir.path())?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect::<std::io::Result<_>>()?;
        written.sort();
        assert_eq!(written, files, "{input:?}");
    }
    Ok(())
}

/// The DOM fallback `--sort-keys` takes over an aliased document is a route of its own: the
/// second document fails, so the first one stays and the third is never evaluated; with the
/// failure first nothing prints. (`-S` is not a yq flag, so these two rows are not captured
/// from yq; they pin that this route follows the others.)
#[test]
fn test_2810_the_sort_keys_fallback_stops_at_the_first_error() -> Result<()> {
    let late = "a: &x {k: 1}\nb: *x\n---\na: &y 2\nb: *y\n---\na: &z {k: 3}\nb: *z\n";
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(".a | keys", late, &["-S"])?;
    assert_eq!((code, stdout.as_str()), (1, "- k\n"), "{stderr}");
    assert_eq!(error_lines(&stderr), 1, "{stderr}");

    let first = "a: &x 1\nb: *x\n---\na: &y {k: 1}\nb: *y\n";
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(".a | keys", first, &["-S"])?;
    assert_eq!((code, stdout.as_str()), (1, ""), "{stderr}");
    assert_eq!(error_lines(&stderr), 1, "{stderr}");
    Ok(())
}

/// An evaluator resource cap is reported but is not an error the program raised, so the run
/// still reaches the next document.
#[test]
fn test_2810_a_resource_cap_does_not_end_the_run() -> Result<()> {
    let (_, stderr, code) = run_yq_stdin_with_stderr(
        "[range(100000000)] | length",
        "a: 1\n---\na: 2\n",
        &["--jq-extensions"],
    )?;
    assert_eq!(code, 1, "{stderr}");
    assert_eq!(
        stderr.matches("maximum iterations exceeded").count(),
        2,
        "{stderr}"
    );
    Ok(())
}
