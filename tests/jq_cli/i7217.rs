//! #7217: a program that reads `input`/`inputs` over 2 to 32 regular files (4 MiB or more between them)
//! reads each file to the end it has when the run reaches it, as jq opens each file when its reader gets
//! there, so a line appended to a later file while the run is still in an earlier one is read. The
//! expected counts are what `/usr/bin/jq` 1.7.1 gives on the same shape. A list of 33 or more files
//! (#7197) opens each path when the run reaches it and read the line already; it is the control.

#![cfg(unix)]

use std::io::Read;
use std::process::{Command, Stdio};

use anyhow::Result;

/// A first file of 80,000 records (about 5 MB: past the 4 MiB the route needs, and more output than a
/// pipe holds), then `files - 1` files of one line each. After 64 KiB of the output has been read, which
/// leaves the run blocked on the pipe in the first file, a line is appended to the last file (or to the
/// second one, when `second`); the output is returned.
fn run_with_append(files: usize, second: bool, program: &[&str]) -> Result<String> {
    let dir = tempfile::tempdir()?;
    let paths: Vec<std::path::PathBuf> = (0..files)
        .map(|n| dir.path().join(format!("f{n:03}.json")))
        .collect();
    for (n, path) in paths.iter().enumerate() {
        let bytes: Vec<u8> = if n == 0 {
            (0..80_000)
                .flat_map(|i| {
                    format!("{{\"id\":{i},\"pad\":\"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"}}\n")
                        .into_bytes()
                })
                .collect()
        } else {
            format!("{{\"k\":{n}}}\n").into_bytes()
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
    std::io::Write::write_all(&mut appended, b"{\"appended\":true}\n")?;
    drop(appended);
    stdout.read_to_end(&mut out)?;
    assert!(child.wait()?.success());
    Ok(String::from_utf8(out)?)
}

/// The run's output with what `change` does to the last file once 64 KiB of it has been read, and the
/// exit status. A first file of 80,000 records, then a one-line file.
fn run_with_change(change: impl FnOnce(&std::path::Path) -> Result<()>) -> Result<(String, bool)> {
    let dir = tempfile::tempdir()?;
    let first = dir.path().join("f0.json");
    let last = dir.path().join("f1.json");
    let bytes: Vec<u8> = (0..80_000)
        .flat_map(|i| format!("{{\"id\":{i},\"pad\":\"{}\"}}\n", "x".repeat(40)).into_bytes())
        .collect();
    assert!(bytes.len() >= 4 << 20);
    std::fs::write(&first, bytes)?;
    std::fs::write(&last, b"{\"k\":1}\n")?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-c", "-n", "inputs"])
        .args([&first, &last])
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut out = vec![0u8; 65536];
    stdout.read_exact(&mut out)?;
    change(&last)?;
    stdout.read_to_end(&mut out)?;
    let ok = child.wait()?.success();
    Ok((String::from_utf8(out)?, ok))
}

/// Bytes that are not UTF-8, appended after the run checked the files, are read as jq reads them: each
/// is one U+FFFD.
#[test]
fn a_non_utf8_line_appended_to_a_later_file_is_substituted_7217() -> Result<()> {
    let (out, ok) = run_with_change(|last| {
        let mut file = std::fs::OpenOptions::new().append(true).open(last)?;
        std::io::Write::write_all(&mut file, b"\"a\xffb\"\n")?;
        Ok(())
    })?;
    assert!(ok);
    assert!(
        out.ends_with("{\"k\":1}\n\"a\u{fffd}b\"\n"),
        "{}",
        &out[out.len() - 40..]
    );
    Ok(())
}

/// The files are held open from the start, so a path removed while the run is in an earlier file still
/// gives the bytes it had (jq, which opens it when it gets there, reports it as missing: recorded in
/// `docs/compliance/jq/limitations.md`).
#[test]
fn a_file_removed_while_the_run_is_in_an_earlier_one_keeps_its_bytes_7217() -> Result<()> {
    let (out, ok) = run_with_change(|last| Ok(std::fs::remove_file(last)?))?;
    assert!(ok);
    assert!(out.ends_with("{\"k\":1}\n"));
    Ok(())
}

fn count(out: &str, needle: &str) -> usize {
    out.lines().filter(|line| line.contains(needle)).count()
}

/// The issue's table: the files the route holds open from the start (2 to 32) read the appended line, as
/// the longer list (33 and 40, opened when reached) and jq do.
#[test]
fn a_line_appended_to_the_last_of_several_files_is_read_7217() -> Result<()> {
    for files in [2, 3, 32, 33, 40] {
        let out = run_with_append(files, false, &["-c", "-n", "inputs"])?;
        assert_eq!(count(&out, "appended"), 1, "{files} files");
        // Every record is still there once: 80,000 + the one-liners + the appended one.
        assert_eq!(out.lines().count(), 80_000 + files, "{files} files");
    }
    Ok(())
}

/// The same for the file right after the one the run is in, and for the `input` form, which reads one
/// document a run of the program.
#[test]
fn a_line_appended_to_the_next_file_and_the_input_form_are_read_7217() -> Result<()> {
    let out = run_with_append(4, true, &["-c", "-n", "inputs"])?;
    assert_eq!(count(&out, "appended"), 1);
    let out = run_with_append(4, true, &["-c", "[., input]"])?;
    assert_eq!(count(&out, "appended"), 1);
    Ok(())
}
