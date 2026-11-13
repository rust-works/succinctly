//! #7024: `-R` over several regular files, 4 MiB or more between them, is read a window at a time as
//! the one stream of lines jq reads, where it was read whole. Every row is what the same files give on
//! the route that reads them whole, which they take as named pipes (no regular file): stdout, stderr
//! and the exit status, with each pipe's name made its file's.
//!
//! The rules the files meet by, captured from jq 1.7.1: a file's last line without a line break runs
//! on into the next file's first, and is read in the later file (`input_filename`, a runtime error's
//! `(at file:N)`); `input_line_number` counts each file's own line breaks; each file's bytes are decoded
//! on their own, so a character split between two files is two replacement characters; and `-R` keeps
//! a byte-order mark wherever it is.

#![cfg(unix)]

use std::io::Write;
use std::process::Command;

use anyhow::Result;

type Run = (String, String, i32);

fn finish(output: &std::process::Output) -> Run {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

/// The run over `paths` on the route that reads them whole: each file as a named pipe a thread fills
/// with its bytes. A pipe the run never opened is opened once it exits, so its writer ends.
fn whole(args: &[&str], paths: &[std::path::PathBuf]) -> Result<Run> {
    let dir = tempfile::tempdir()?;
    let mut pipes = Vec::new();
    let mut writers = Vec::new();
    for (n, path) in paths.iter().enumerate() {
        let pipe = dir.path().join(format!("p{n:02}"));
        assert!(Command::new("mkfifo").arg(&pipe).status()?.success());
        let bytes = std::fs::read(path)?;
        let target = pipe.clone();
        writers.push((
            pipe.clone(),
            std::thread::spawn(move || {
                if let Ok(mut out) = std::fs::OpenOptions::new().write(true).open(&target) {
                    let _ = out.write_all(&bytes);
                }
            }),
        ));
        pipes.push(pipe);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .args(&pipes)
        .env("LC_ALL", "C")
        .output()?;
    for (pipe, writer) in writers {
        if !writer.is_finished() {
            drop(std::fs::File::open(&pipe));
        }
        writer.join().expect("the writer ends");
    }
    let (mut stdout, mut stderr, status) = finish(&output);
    for (pipe, path) in pipes.iter().zip(paths) {
        let (pipe, path) = (pipe.display().to_string(), path.display().to_string());
        stdout = stdout.replace(&pipe, &path);
        stderr = stderr.replace(&pipe, &path);
    }
    Ok((stdout, stderr, status))
}

/// `files` written out and run both ways; the two answers, which the caller compares.
fn both(args: &[&str], files: &[Vec<u8>]) -> Result<(Run, Run)> {
    let dir = tempfile::tempdir()?;
    let mut paths = Vec::new();
    for (n, bytes) in files.iter().enumerate() {
        let path = dir.path().join(format!("f{n:02}.txt"));
        std::fs::write(&path, bytes)?;
        paths.push(path);
    }
    let windowed = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .args(&paths)
        .env("LC_ALL", "C")
        .output()?;
    Ok((finish(&windowed), whole(args, &paths)?))
}

/// 4.5 MB of lines with the shapes that meet at a file's end: a CRLF line, invalid UTF-8, a BOM in the
/// middle, and a two-byte character the cut after it splits.
fn stream() -> (Vec<u8>, usize) {
    let mut bytes: Vec<u8> = (0..75_000)
        .flat_map(|i| {
            format!(
                "{{\"id\":{i},\"name\":\"user {i}\",\"pad\":\"{}\"}}\n",
                "x".repeat(i % 37)
            )
            .into_bytes()
        })
        .collect();
    assert!(bytes.len() >= 4 << 20, "{} bytes", bytes.len());
    bytes.splice(1000..1000, *b"\r\n");
    bytes.splice(200_000..200_000, *b"\xff\xfe");
    bytes.splice(300_000..300_000, *b"\xef\xbb\xbf");
    let at = 400_000;
    bytes.splice(at..at, "é".bytes());
    (bytes, at + 1)
}

/// `bytes` cut at `at` (in order) into files.
fn cut(bytes: &[u8], at: &[usize]) -> Vec<Vec<u8>> {
    let mut files = Vec::new();
    let mut from = 0;
    for &to in at {
        files.push(bytes[from..to].to_vec());
        from = to;
    }
    files.push(bytes[from..].to_vec());
    files
}

const PROGRAMS: &[&[&str]] = &[
    &["-R", "."],
    &["-cR", "[input_filename, .]"],
    &["-R", "input_line_number"],
    &["-nR", "[inputs] | length"],
    &["-cnR", "[input, input]"],
    &["-cnR", "[inputs | input_filename] | unique"],
    &["-nR", "last(inputs)"],
    &["-nR", "first(inputs), input_line_number"],
    &["-R", "input"],
    &["-cnR", "[inputs] | .[-3:], input_line_number"],
    // A runtime error names the file and line jq's reader is at.
    &[
        "-R",
        "if test(\"user 4321\\\"\") then error(\"stop\") else empty end",
    ],
];

#[test]
fn several_files_read_as_one_stream_of_lines_7024() -> Result<()> {
    let (bytes, split) = stream();
    let len = bytes.len();
    // Cut inside a line, at a line start, inside the split character, and an empty file.
    let mut at = [len / 7, len / 7, split, len / 2 + 3, 3 * len / 4];
    at.sort_unstable();
    let files = cut(&bytes, &at);
    for args in PROGRAMS {
        let (windowed, whole) = both(args, &files)?;
        assert_eq!(windowed, whole, "{args:?}");
    }
    // A last file whose last line has no line break.
    let mut files = cut(&bytes, &[len / 3, 2 * len / 3]);
    files
        .last_mut()
        .expect("a file")
        .extend_from_slice(b"no line break");
    for args in PROGRAMS {
        let (windowed, whole) = both(args, &files)?;
        assert_eq!(windowed, whole, "{args:?}");
    }
    Ok(())
}

/// What the rows rest on: every line of the files is read (the CRLF splice is one more). That the
/// route is taken is `raw_windowed_sources_take_several_regular_files_7024`'s to show.
#[test]
fn every_line_of_several_files_is_read_7024() -> Result<()> {
    let (bytes, _) = stream();
    let len = bytes.len();
    let (windowed, _) = both(
        &["-nR", "[inputs] | length"],
        &cut(&bytes, &[len / 3, 2 * len / 3]),
    )?;
    assert_eq!(windowed, ("75001\n".to_string(), String::new(), 0));
    Ok(())
}

/// The seam rules themselves, as jq 1.7.1 answers them (captured from `/usr/bin/jq` over these files,
/// 4.47 MB between them, so the route takes them): a last line without a line break joined to the next
/// file's first and read there, `input_line_number` per file, a two-byte character split between two files
/// read as two replacement characters, a BOM kept, and a line left open before an empty last file read in
/// that file at line 0, with `input_line_number` 0 at the end.
#[test]
fn the_seams_between_files_are_jqs_7024() -> Result<()> {
    let filler = |tag: &str, n: usize| -> Vec<u8> {
        (0..n)
            .flat_map(|i| format!("filler {tag} {i}\n").into_bytes())
            .collect()
    };
    let mut f0 = filler("a", 100_000);
    f0.extend_from_slice(b"a2");
    let mut f1 = b"b1\nb2\n".to_vec();
    f1.extend(filler("b", 100_000));
    let mut f4 = b"\xa9y\n\xef\xbb\xbfz\n".to_vec();
    f4.extend(filler("c", 100_000));
    f4.extend_from_slice(b"open");
    let files: [(&str, Vec<u8>); 6] = [
        ("f0", f0),
        ("f1", f1),
        ("f2", Vec::new()),
        ("f3", b"x\xc3".to_vec()),
        ("f4", f4),
        ("f5", Vec::new()),
    ];
    let dir = tempfile::tempdir()?;
    for (name, bytes) in &files {
        std::fs::write(dir.path().join(name), bytes)?;
    }
    let run = |args: &[&str]| -> Result<Run> {
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("jq")
            .args(args)
            .args(files.iter().map(|(name, _)| name))
            .current_dir(dir.path())
            .env("LC_ALL", "C")
            .output()?;
        Ok(finish(&output))
    };
    let seams = run(&[
        "-cR",
        "select(startswith(\"filler\") | not) | [input_filename, input_line_number, .]",
    ])?;
    let want: String = [
        "[\"f1\",1,\"a2b1\"]\n",
        "[\"f1\",2,\"b2\"]\n",
        "[\"f4\",1,\"x\u{fffd}\u{fffd}y\"]\n",
        "[\"f4\",2,\"\u{feff}z\"]\n",
        "[\"f5\",0,\"open\"]\n",
    ]
    .concat();
    assert_eq!(seams, (want, String::new(), 0));
    let count = run(&["-cnR", "[inputs] | length, input_line_number"])?;
    assert_eq!(count, ("300005\n0\n".to_string(), String::new(), 0));
    Ok(())
}
