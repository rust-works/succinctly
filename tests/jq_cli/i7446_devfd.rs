//! #7446 follow-up: a lone input file named through a descriptor (`jq -c . /dev/fd/7 7<f`) is read to
//! the end it has when the reads get there, as a named file is since #7446 and as jq 1.7.1 and 1.8.2
//! read it: lines a writer appends while the run is held are read. It was read whole before the
//! program ran, so they were dropped. The descriptor is read from where its offset stands where the
//! path shares it (macOS duplicates the descriptor; Linux opens the file again, from its start), and a
//! fault is placed in that stream. Expected outputs are what `/usr/bin/jq` 1.7.1 gives on this Mac.
//!
//! The descriptor is 7: macOS's bash 3.2 cannot open one above 9 reliably.

#![cfg(unix)]

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;

/// Records of the input: 4 KB each, so the file is past the 1 MiB the windowed routes need and the
/// output is far more than a pipe holds.
const RECORDS: usize = 1_100;

fn record(i: usize) -> String {
    format!("{{\"id\":{i},\"pad\":\"{}\"}}\n", "x".repeat(4000))
}

/// `succinctly jq <program> /dev/fd/7`, with descriptor 7 open on `file` where it stands; standard
/// input is the null device.
fn command(program: &[&str], file: std::fs::File) -> Command {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(r#"exec "$0" jq "$@" /dev/fd/7 7<&0 0</dev/null"#)
        .arg(env!("CARGO_BIN_EXE_succinctly"))
        .args(program)
        .stdin(file);
    command
}

/// Kills the process `pid` unless the returned sender is dropped within two minutes: a run that
/// blocks fails its test (its pipe closes, or its status is a signal) instead of hanging the suite.
fn watchdog(pid: u32) -> std::sync::mpsc::Sender<()> {
    let (done, ended) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
            ended.recv_timeout(std::time::Duration::from_secs(120))
        {
            let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
        }
    });
    done
}

/// Runs `program` over `path` as descriptor 7 and, once 64 KiB of the output is read, appends
/// `tail` to the file; the whole output and whether the run succeeded.
fn run_held(program: &[&str], path: &Path, tail: &[u8]) -> Result<(String, bool)> {
    let mut child = command(program, std::fs::File::open(path)?)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let _watchdog = watchdog(child.id());
    let mut stdout = child.stdout.take().expect("piped");
    let mut out = vec![0u8; 65536];
    stdout.read_exact(&mut out)?;
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)?
        .write_all(tail)?;
    stdout.read_to_end(&mut out)?;
    let ok = child.wait()?.success();
    Ok((String::from_utf8(out)?, ok))
}

#[test]
fn lines_appended_to_a_file_named_through_a_descriptor_are_read_7446() -> Result<()> {
    let tail = b"{\"appended\":1}\n{\"appended\":2}\n";
    let cases: [(&[&str], &str); 3] = [
        (&["-c", "."], "{\"appended\":1}\n{\"appended\":2}\n"),
        (&["-c", ".pad // .appended"], "1\n2\n"),
        (
            &["-R", "-c", "."],
            "\"{\\\"appended\\\":1}\"\n\"{\\\"appended\\\":2}\"\n",
        ),
    ];
    for (program, want) in cases {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("in.json");
        std::fs::write(&path, (0..RECORDS).map(record).collect::<String>())?;
        let (out, ok) = run_held(program, &path, tail)?;
        assert!(ok, "{program:?}");
        assert!(
            out.ends_with(want),
            "{program:?}: {}",
            &out[out.len() - 60..]
        );
        assert_eq!(out.lines().count(), RECORDS + 2, "{program:?}");
    }
    Ok(())
}

/// A descriptor that something read part of: where the path shares its offset (macOS) the stream
/// starts there, as jq's does, and the fault at its end is placed by the lines from there; where the
/// path opens the file again (Linux) it starts at the file's start, in both tools.
#[test]
fn a_descriptor_read_part_way_is_read_from_its_offset_where_the_path_shares_it_7446() -> Result<()>
{
    let skipped = 100;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    let text = (0..RECORDS).map(record).collect::<String>() + "{\"bad\":}\n";
    std::fs::write(&path, &text)?;
    let mut file = std::fs::File::open(&path)?;
    let offset = (0..skipped).map(|i| record(i).len()).sum::<usize>();
    file.seek(SeekFrom::Start(offset as u64))?;
    let child = command(&["-c", ".id"], file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let watchdog = watchdog(child.id());
    let out = child.wait_with_output()?;
    drop(watchdog);
    let first = if cfg!(target_os = "macos") {
        skipped
    } else {
        0
    };
    let want: String = (first..RECORDS).map(|i| format!("{i}\n")).collect();
    assert_eq!(String::from_utf8(out.stdout)?, want);
    assert_eq!(
        String::from_utf8(out.stderr)?,
        format!(
            "jq: parse error: Unmatched '}}' at line {}, column 8\n",
            RECORDS - first + 1
        )
    );
    assert_eq!(out.status.code(), Some(5));
    Ok(())
}
