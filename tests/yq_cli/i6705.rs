//! #6705: yq reads a flag wherever it appears on the command line, so `yq '.a' f.yaml -o=json` is a
//! flag after the file, not a second file. `succinctly yq` declared its `files` argument with
//! `trailing_var_arg`, which made every word after the first file a file ("failed to read file:
//! -o=json"); `succinctly jq` had dropped it for the same reason (#3447). Every answer was
//! captured from yq v4.53.3.

use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::TempDir;

fn run_in(dir: &TempDir, args: &[&str]) -> Result<(String, i32)> {
    std::fs::write(dir.path().join("f.yaml"), "a: {b: 1}\nc: [1, 2]\n")?;
    std::fs::write(dir.path().join("g.json"), "{\"x\": [1, 2], \"y\": \"s\"}\n")?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(args)
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn test_yq_reads_a_flag_after_the_file_name_as_a_flag_6705() -> Result<()> {
    let dir = TempDir::new()?;
    for (args, want) in [
        (&[".a", "f.yaml", "-o=json"][..], "{\n  \"b\": 1\n}\n"),
        (&[".a", "f.yaml", "-o", "json"][..], "{\n  \"b\": 1\n}\n"),
        (&[".a", "f.yaml", "-I=4"][..], "{b: 1}\n"),
        (&[".a", "f.yaml", "-r"][..], "{b: 1}\n"),
        (&[".c[]", "f.yaml", "-N"][..], "1\n2\n"),
        (&[".a.b", "f.yaml", "-e"][..], "1\n"),
        (&[".a", "f.yaml", "-P"][..], "b: 1\n"),
        (&[".a", "-o=json", "f.yaml"][..], "{\n  \"b\": 1\n}\n"),
        (&["-o=json", ".a", "f.yaml"][..], "{\n  \"b\": 1\n}\n"),
        (&[".a", "f.yaml", "-o=json", "-I=0"][..], "{\"b\":1}\n"),
        (&[".a", "f.yaml", "-I=0", "-o=json"][..], "{\"b\":1}\n"),
        (&["-I=0", ".a", "f.yaml", "-o=json"][..], "{\"b\":1}\n"),
        (
            &[".a", "f.yaml", "f.yaml", "-o=json"][..],
            "{\n  \"b\": 1\n}\n{\n  \"b\": 1\n}\n",
        ),
        (
            &[".a", "f.yaml", "-o=json", "f.yaml"][..],
            "{\n  \"b\": 1\n}\n{\n  \"b\": 1\n}\n",
        ),
        (&[".x", "g.json", "-I=0"][..], "[1,2]\n"),
        (&[".x[0]", "g.json", "-r"][..], "1\n"),
        (&[".y", "g.json", "-r"][..], "s\n"),
        (&[".y", "g.json", "-o=yaml"][..], "s\n"),
        (&[".a", "f.yaml", "--no-doc"][..], "{b: 1}\n"),
        (&[".a", "f.yaml", "--indent", "4"][..], "{b: 1}\n"),
        (
            &[".a", "f.yaml", "--output-format", "json"][..],
            "{\n  \"b\": 1\n}\n",
        ),
        (&[".a", "f.yaml", "-p=yaml"][..], "{b: 1}\n"),
    ] {
        let (stdout, code) = run_in(&dir, args)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{args:?}");
    }
    Ok(())
}

/// `-e` after the file still sets the exit status from the result (yq exits 1 for a `null`).
#[test]
fn test_yq_exit_status_flag_after_the_file_name_6705() -> Result<()> {
    let dir = TempDir::new()?;
    let (stdout, code) = run_in(&dir, &[".zzz", "f.yaml", "-e"])?;
    assert_eq!((stdout.as_str(), code), ("null\n", 1));
    Ok(())
}

/// `--inplace` after the file edits it.
#[test]
fn test_yq_inplace_flag_after_the_file_name_6705() -> Result<()> {
    let dir = TempDir::new()?;
    let (_, code) = run_in(&dir, &[".a.b = 2", "f.yaml", "-i"])?;
    assert_eq!(code, 0);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("f.yaml"))?,
        "a: {b: 2}\nc: [1, 2]\n"
    );
    Ok(())
}
