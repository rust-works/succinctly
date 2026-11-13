//! #7024: a list of more than 32 regular files is read a window at a time as the one stream jq reads
//! them as, each file opened when the run reaches it, where such a list was read whole. Every row is
//! what the same files give when the route declines them, which is when the paths are `/dev/fd/N`:
//! stdout, stderr and the exit status, with each file's name made one. Descriptors past 9 are opened
//! by `bash`, which reads `exec 10<path` as a redirection where `sh` may not.

#![cfg(unix)]

use std::io::Write;
use std::process::Command;

use anyhow::Result;

/// More files than the route holds open (32), small enough that a token is open where many end.
const FILES: usize = 40;

fn text(lines: usize) -> Vec<u8> {
    (0..lines)
        .flat_map(|i| {
            format!(
                "{{\"id\":{i},\"name\":\"user {i}\",\"ok\":true,\"tags\":[\"a\",\"b\"],\"score\":{}}}\n",
                i % 1000
            )
            .into_bytes()
        })
        .collect()
}

/// `bytes` cut into `FILES` files of near the same size, each cut moved by a few bytes so that most
/// fall inside a document.
fn cut(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut files = Vec::new();
    let mut from = 0;
    for n in 1..FILES {
        let to = n * bytes.len() / FILES + n % 11;
        files.push(bytes[from..to].to_vec());
        from = to;
    }
    files.push(bytes[from..].to_vec());
    files
}

type Run = (String, String, i32);

/// The files as paths (the windowed route), and as descriptors 10 on (the route that reads them
/// whole).
fn both(args: &[&str], files: &[Vec<u8>]) -> Result<(Run, Run)> {
    let dir = tempfile::tempdir()?;
    let mut paths = Vec::new();
    for (n, bytes) in files.iter().enumerate() {
        let path = dir.path().join(format!("f{n:02}.json"));
        std::fs::File::create(&path)?.write_all(bytes)?;
        paths.push(path.to_string_lossy().into_owned());
    }
    let bin = env!("CARGO_BIN_EXE_succinctly");
    let name = |text: &[u8]| {
        let mut text = String::from_utf8_lossy(text).into_owned();
        // The longest first: `/dev/fd/1` is a prefix of `/dev/fd/10`.
        for (n, path) in paths.iter().enumerate().rev() {
            text = text.replace(path.as_str(), &format!("<file {n}>"));
            text = text.replace(&format!("/dev/fd/{}", n + 10), &format!("<file {n}>"));
        }
        text
    };
    let finish = |output: std::process::Output| -> Run {
        (
            name(&output.stdout),
            name(&output.stderr),
            output.status.code().unwrap_or(-1),
        )
    };
    let windowed = Command::new(bin)
        .arg("jq")
        .args(args)
        .args(&paths)
        .env("LC_ALL", "C")
        .output()?;
    let mut script = String::new();
    for (n, path) in paths.iter().enumerate() {
        script.push_str(&format!("exec {}<'{}'\n", n + 10, path));
    }
    script.push_str("exec \"$0\" jq \"$@\"");
    for n in 0..paths.len() {
        script.push_str(&format!(" /dev/fd/{}", n + 10));
    }
    script.push('\n');
    let whole = Command::new("bash")
        .arg("-c")
        .arg(&script)
        .arg(bin)
        .args(args)
        .env("LC_ALL", "C")
        .output()?;
    Ok((finish(windowed), finish(whole)))
}

fn same(args: &[&str], files: &[Vec<u8>]) -> Result<Run> {
    let (windowed, whole) = both(args, files)?;
    assert_eq!(windowed, whole, "{args:?}");
    Ok(windowed)
}

/// 4 MiB or more between the files, so the route takes them.
fn big() -> Vec<u8> {
    let bytes = text(60_000);
    assert!(bytes.len() >= 4 << 20, "{} bytes", bytes.len());
    bytes
}

#[test]
fn more_files_than_are_held_open_read_as_one_stream_7024() -> Result<()> {
    let files = cut(&big());
    let (stdout, stderr, status) = same(&["-c", "."], &files)?;
    assert_eq!(
        (stdout.lines().count(), stderr.as_str(), status),
        (60_000, "", 0)
    );
    let (stdout, _, _) = same(&["input_filename"], &files)?;
    assert!(stdout.contains("<file 39>"), "every file is reached");
    same(&["-e", ".nope"], &files)?;
    same(&[".id | select(. % 9999 == 0)"], &files)?;
    // A runtime error names the file and line jq's reader is at.
    same(
        &["if .id == 45000 then error(\"stop\") else .id end"],
        &files,
    )?;
    Ok(())
}

#[test]
fn a_fault_in_a_file_the_run_left_is_worded_over_the_whole_stream_7024() -> Result<()> {
    let bytes = big();
    for (at, bad) in [
        (bytes.len() / 3, &b"{\"bad\": [1 2]}\n"[..]),
        (bytes.len() / 2, b"}\n"),
        (bytes.len(), b"{\"last\": "),
    ] {
        // At a line start, so the fault is the inserted text's.
        let at = bytes[..at]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |n| n + 1);
        let mut text = bytes[..at].to_vec();
        text.extend_from_slice(bad);
        text.extend_from_slice(&bytes[at..]);
        let files = cut(&text);
        let (_, stderr, status) = same(&["-c", "."], &files)?;
        assert!(stderr.contains("parse error"), "{stderr}");
        assert_ne!(status, 0);
        // A filter that reads one field validates only what it reads (#2103): the fault is still
        // the stream's first.
        same(&[".id"], &files)?;
    }
    Ok(())
}

#[test]
fn a_bom_at_the_front_and_text_that_is_not_utf8_read_as_the_files_joined_7024() -> Result<()> {
    let mut text = b"\xef\xbb\xbf".to_vec();
    text.extend_from_slice(&big());
    let at = text.len() / 2;
    let at = text[..at]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |n| n + 1);
    text.splice(at..at, b"{\"s\":\"\xff\"}\n".iter().copied());
    let files = cut(&text);
    same(&["-c", "."], &files)?;
    same(&[".s // empty"], &files)?;
    Ok(())
}
