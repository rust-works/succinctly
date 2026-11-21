//! #7446: a lone input file (named, redirected on standard input, read by `-R` or by `inputs`), and each
//! file of a short list under the plain filter, is read to the end it has when the reads get there, not
//! to the length `stat` gave when the run opened it, as jq reads it: lines a writer appends while the run
//! goes are read, and a last document cut short then and completed after is read whole. The expected
//! outputs are what `/usr/bin/jq` 1.7.1 (and jq 1.8.2) give on the same shapes.
//!
//! Each run is held, not timed: the test reads 64 KiB of the output, which leaves the run blocked on the
//! pipe with megabytes of the input unread, makes its change to the file, then reads the rest.

#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Result;

/// Records of the input: 4 KB each, so the file is past the 1 MiB the windowed routes need, the output
/// is far more than a pipe holds, and a debug build reads it quickly.
const RECORDS: usize = 1_100;

fn records() -> String {
    (0..RECORDS)
        .map(|i| format!("{{\"id\":{i},\"pad\":\"{}\"}}\n", "x".repeat(4000)))
        .collect()
}

/// How the run gets the file.
#[derive(Clone, Copy)]
enum Source {
    Named,
    Stdin,
}

/// Runs `succinctly jq <program>` over `path`, and once 64 KiB of the output is read, `change`s the
/// file; the whole output and whether the run succeeded.
fn run_held(
    program: &[&str],
    path: &Path,
    source: Source,
    change: impl FnOnce(&Path) -> Result<()>,
) -> Result<(String, bool)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command.arg("jq").args(program);
    match source {
        Source::Named => {
            command.arg(path);
        }
        Source::Stdin => {
            command.stdin(std::fs::File::open(path)?);
        }
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut out = vec![0u8; 65536];
    stdout.read_exact(&mut out)?;
    change(path)?;
    stdout.read_to_end(&mut out)?;
    let ok = child.wait()?.success();
    Ok((String::from_utf8(out)?, ok))
}

fn append(bytes: &'static [u8]) -> impl FnOnce(&Path) -> Result<()> {
    move |path| {
        let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
        file.write_all(bytes)?;
        Ok(())
    }
}

/// A fresh input of [`RECORDS`] records and then `tail`, in a directory of its own.
fn input(tail: &str) -> Result<(tempfile::TempDir, PathBuf)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    std::fs::write(&path, records() + tail)?;
    Ok((dir, path))
}

/// The issue's spellings: a lone file named, redirected on standard input, read by `-R`, by `inputs` and
/// by `input`, all read the two lines appended while the run is held.
#[test]
fn lines_appended_to_a_lone_file_are_read_7446() -> Result<()> {
    let plain = "{\"appended\":1}\n{\"appended\":2}\n";
    let cases: [(&[&str], Source, &str, usize); 6] = [
        (&["-c", "."], Source::Named, plain, RECORDS + 2),
        (&["-c", "."], Source::Stdin, plain, RECORDS + 2),
        (
            &["-R", "-c", "."],
            Source::Named,
            "\"{\\\"appended\\\":1}\"\n\"{\\\"appended\\\":2}\"\n",
            RECORDS + 2,
        ),
        (&["-n", "-c", "inputs"], Source::Named, plain, RECORDS + 2),
        (&["-n", "-c", "inputs"], Source::Stdin, plain, RECORDS + 2),
        (
            &["-c", "[., input]"],
            Source::Named,
            "[{\"appended\":1},{\"appended\":2}]\n",
            (RECORDS + 2) / 2,
        ),
    ];
    for (program, source, tail, lines) in cases {
        let (_dir, path) = input("")?;
        let (out, ok) = run_held(
            program,
            &path,
            source,
            append(b"{\"appended\":1}\n{\"appended\":2}\n"),
        )?;
        assert!(ok, "{program:?}");
        assert!(
            out.ends_with(tail),
            "{program:?}: {}",
            &out[out.len() - 60..]
        );
        assert_eq!(out.lines().count(), lines, "{program:?}");
    }
    Ok(())
}

/// A last document cut short when the run opened the file, and completed while it is held, is read
/// whole, as jq reads it, not reported as cut short at the length the file had.
#[test]
fn a_last_document_completed_while_the_run_reads_is_read_whole_7446() -> Result<()> {
    for (program, source) in [
        (&["-c", "."][..], Source::Named),
        (&["-c", "."][..], Source::Stdin),
        (&["-n", "-c", "inputs"][..], Source::Named),
    ] {
        let (_dir, path) = input("{\"appended\":")?;
        let (out, ok) = run_held(program, &path, source, append(b"true}\n"))?;
        assert!(ok, "{program:?}");
        assert!(out.ends_with("}\n{\"appended\":true}\n"), "{program:?}");
        assert_eq!(out.lines().count(), RECORDS + 1, "{program:?}");
    }
    Ok(())
}

/// A file cut short while the run is held ends where the reads find its end, with no fault and no
/// crash: the records before the cut, as jq gives them.
#[test]
fn a_lone_file_cut_short_while_the_run_reads_ends_there_7446() -> Result<()> {
    let keep = RECORDS / 2;
    for program in [
        &["-c", "."][..],
        &["-R", "-c", "."],
        &["-n", "-c", "inputs"],
    ] {
        let (_dir, path) = input("")?;
        let (out, ok) = run_held(program, &path, Source::Named, |path| {
            let len = records()
                .lines()
                .take(keep)
                .map(|line| line.len() + 1)
                .sum::<usize>();
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)?
                .set_len(len as u64)?;
            Ok(())
        })?;
        assert!(ok, "{program:?}");
        assert_eq!(out.lines().count(), keep, "{program:?}");
        assert!(out
            .lines()
            .last()
            .is_some_and(|line| line.contains(&format!("{}", keep - 1))));
    }
    Ok(())
}

/// A file replaced by another renamed over its path while the run is held is read as the run opened it,
/// to its end, as jq reads it: both hold the file open, so the new one is not read.
#[test]
fn a_lone_file_replaced_while_the_run_reads_keeps_its_bytes_7446() -> Result<()> {
    let (_dir, path) = input("")?;
    let (out, ok) = run_held(&["-c", "."], &path, Source::Named, |path| {
        let new = path.with_extension("new");
        std::fs::write(&new, "{\"replaced\":true}\n")?;
        std::fs::rename(&new, path)?;
        Ok(())
    })?;
    assert!(ok);
    assert!(!out.contains("replaced"));
    assert_eq!(out.lines().count(), RECORDS);
    Ok(())
}

/// The plain filter over several files reads a file the run is still in to the end it has then, before
/// the next file, as jq does.
#[test]
fn a_line_appended_to_the_file_the_plain_filter_is_in_is_read_7446() -> Result<()> {
    let (dir, path) = input("")?;
    let next = dir.path().join("next.json");
    std::fs::write(&next, "{\"next\":1}\n")?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-c", "."])
        .args([&path, &next])
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut out = vec![0u8; 65536];
    stdout.read_exact(&mut out)?;
    append(b"{\"appended\":1}\n")(&path)?;
    stdout.read_to_end(&mut out)?;
    assert!(child.wait()?.success());
    let out = String::from_utf8(out)?;
    assert!(out.ends_with("}\n{\"appended\":1}\n{\"next\":1}\n"));
    assert_eq!(out.lines().count(), RECORDS + 2);
    Ok(())
}

/// A run whose output is appended to its own input through a pipe (`jq -c . f | tee -a f`) reads what it
/// wrote and goes on while it does, as jq does: here until three times the file has been read back.
#[test]
fn a_run_whose_output_is_appended_to_its_input_reads_on_7446() -> Result<()> {
    let (_dir, path) = input("")?;
    let original = std::fs::metadata(&path)?.len() as usize;
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-c", "."])
        .arg(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut appender = std::fs::OpenOptions::new().append(true).open(&path)?;
    let mut read = 0;
    let mut chunk = vec![0u8; 65536];
    while read < 3 * original {
        let n = stdout.read(&mut chunk)?;
        assert!(
            n > 0,
            "the run ended after {read} bytes, short of what was appended"
        );
        appender.write_all(&chunk[..n])?;
        read += n;
    }
    drop(stdout);
    // The run ends at the closed pipe.
    child.wait()?;
    Ok(())
}

/// Recorded in `docs/compliance/jq/limitations.md`: a `--seq` file is read to the length its records were
/// vouched for up to before the run, so a record appended while it reads is not, where jq reads it.
#[test]
fn a_record_appended_to_a_seq_file_is_not_read_yet_7446() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.seq");
    let text: String = records()
        .lines()
        .map(|line| format!("\u{1e}{line}\n"))
        .collect();
    std::fs::write(&path, text)?;
    let (out, ok) = run_held(
        &["--seq", "-c", "."],
        &path,
        Source::Named,
        append(b"\x1e{\"appended\":1}\n"),
    )?;
    assert!(ok);
    assert!(!out.contains("appended"));
    assert_eq!(out.lines().count(), RECORDS);
    Ok(())
}
