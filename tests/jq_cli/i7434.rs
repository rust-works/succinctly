//! #7434: a `-f` program file that holds a NUL byte is refused before it is compiled, as jq 1.8.2
//! refuses it (CVE-2026-41256, upstream commit 5a015deae3): `jq: program file contains NUL bytes` on
//! stderr, nothing on stdout, exit status 2. jq 1.7.1 treats the file as a C string and ends the
//! program at the NUL, so it is the one version that disagrees; the pinned 1.7.1 answers are in the
//! limitations page. Every row below was captured from jq 1.8.2.

use anyhow::Result;

use crate::run_jq_full;

const REFUSED: &str = "jq: program file contains NUL bytes\n";

/// Program files that jq 1.8.2 refuses: the NUL trailing, in the middle, first, alone, inside a
/// comment, after text that would not compile, and one of several.
const PROGRAMS: [&[u8]; 9] = [
    b".a\0| .b",
    b"1\0garbage (",
    b"# c\0\n.a",
    b".a\0",
    b"\0",
    b"\0.a",
    b".a\0\0.b",
    b".a\n\0\n.b",
    b"def f: .a; f\0",
];

fn write_program(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> Result<String> {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes)?;
    Ok(path.to_string_lossy().into_owned())
}

#[test]
fn a_program_file_with_a_nul_is_refused_7434() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for (i, program) in PROGRAMS.iter().enumerate() {
        let path = write_program(&dir, &format!("p{i}.jq"), program)?;
        for flags in [
            vec!["-c", "-f"],
            vec!["-n", "-f"],
            vec!["-c", "--from-file"],
        ] {
            let mut args = flags.clone();
            args.push(&path);
            let (out, err, status) = run_jq_full(&args, Some("{\"a\":7,\"b\":8}"))?;
            assert_eq!(out, "", "{program:?} {flags:?}: stdout");
            assert_eq!(err, REFUSED, "{program:?} {flags:?}: stderr");
            assert_eq!(status, 2, "{program:?} {flags:?}: exit status");
        }
    }
    Ok(())
}

#[test]
fn the_nul_is_refused_before_the_input_is_opened_7434() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = write_program(&dir, "p.jq", b".a\0")?;
    let (out, err, status) = run_jq_full(&["-f", &path, "/no/such/input.json"], None)?;
    assert_eq!((out.as_str(), err.as_str(), status), ("", REFUSED, 2));
    Ok(())
}

#[test]
fn a_program_file_read_from_stdin_with_a_nul_is_refused_7434() -> Result<()> {
    let (out, err, status) = run_jq_full(&["-c", "-f", "/dev/stdin"], Some(".a\0| .b"))?;
    assert_eq!((out.as_str(), err.as_str(), status), ("", REFUSED, 2));
    Ok(())
}

#[test]
fn a_program_file_without_a_nul_still_runs_7434() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = write_program(&dir, "ok.jq", b".a | . + 1")?;
    let (out, err, status) = run_jq_full(&["-c", "-f", &path], Some("{\"a\":7}"))?;
    assert_eq!((out.as_str(), err.as_str(), status), ("8\n", "", 0));
    Ok(())
}
