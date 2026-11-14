//! #7223: `-R` over 2 to 32 regular files (4 MiB or more between them) reads each file to the end it has
//! when the run reaches it, as jq opens each file when its reader gets there, so a line appended to a later
//! file while the run is still in an earlier one is read. The counts are what `/usr/bin/jq` 1.7.1 gives on
//! the same shape; a list of 33 or more files (opened when reached, #7024) is the control.

#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};

use anyhow::Result;

/// A first file of 80,000 lines (past the 4 MiB the route needs and more output than a pipe holds), then
/// `files - 1` files of one line each. After 64 KiB of the output has been read, which leaves the run
/// blocked on the pipe in the first file, a line is appended to the last file (or to the second one, when
/// `second`); the output is returned.
fn run_with_append(files: usize, second: bool, program: &[&str]) -> Result<String> {
    let dir = tempfile::tempdir()?;
    let paths: Vec<std::path::PathBuf> = (0..files)
        .map(|n| dir.path().join(format!("f{n:03}.txt")))
        .collect();
    for (n, path) in paths.iter().enumerate() {
        let bytes: Vec<u8> = if n == 0 {
            (0..80_000)
                .flat_map(|i| format!("line {i} {}\n", "x".repeat(80)).into_bytes())
                .collect()
        } else {
            format!("one {n}\n").into_bytes()
        };
        if n == 0 {
            assert!(bytes.len() >= 4 << 20, "{} bytes", bytes.len());
        }
        std::fs::write(path, bytes)?;
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(program)
        .args(&paths)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut out = vec![0u8; 65536];
    stdout.read_exact(&mut out)?;
    let target = if second { &paths[1] } else { &paths[files - 1] };
    let mut appended = std::fs::OpenOptions::new().append(true).open(target)?;
    appended.write_all(b"appended\n")?;
    drop(appended);
    stdout.read_to_end(&mut out)?;
    assert!(child.wait()?.success());
    Ok(String::from_utf8(out)?)
}

fn count(out: &str, needle: &str) -> usize {
    out.lines().filter(|line| line.contains(needle)).count()
}

/// The issue's table: the files the route holds open from the start (2 to 32) read the appended line, as
/// the longer list (33 and 40) and jq do.
#[test]
fn a_line_appended_to_the_last_of_several_raw_files_is_read_7223() -> Result<()> {
    for files in [2, 3, 32, 33, 40] {
        let out = run_with_append(files, false, &["-R", "-c", "."])?;
        assert_eq!(count(&out, "appended"), 1, "{files} files");
        // Every line is still there once: 80,000 + the one-liners + the appended one.
        assert_eq!(out.lines().count(), 80_000 + files, "{files} files");
    }
    Ok(())
}

/// The same for the file right after the one the run is in, and for a program that reads the lines with
/// `inputs`.
#[test]
fn a_line_appended_to_the_next_raw_file_and_the_inputs_form_are_read_7223() -> Result<()> {
    let out = run_with_append(4, true, &["-R", "-c", "."])?;
    assert_eq!(count(&out, "appended"), 1);
    let out = run_with_append(4, true, &["-n", "-R", "-c", "inputs"])?;
    assert_eq!(count(&out, "appended"), 1);
    Ok(())
}

/// An empty file between two others is read as nothing, as jq reads it: it has no length to say so now.
#[test]
fn an_empty_file_among_several_raw_files_reads_as_nothing_7223() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let big: String = (0..80_000)
        .map(|i| format!("line {i} {}\n", "x".repeat(80)))
        .collect();
    let paths = [
        dir.path().join("a.txt"),
        dir.path().join("b.txt"),
        dir.path().join("c.txt"),
    ];
    std::fs::write(&paths[0], big)?;
    std::fs::write(&paths[1], b"")?;
    std::fs::write(&paths[2], b"tail\n")?;
    let out = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-R", "-c", "."])
        .args(&paths)
        .output()?;
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout)?;
    assert_eq!(text.lines().count(), 80_001);
    assert_eq!(text.lines().last(), Some("\"tail\""));
    Ok(())
}
