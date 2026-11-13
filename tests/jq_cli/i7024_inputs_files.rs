//! #7024: a program that reads `input`/`inputs` over several regular files, 4 MiB or more between them,
//! reads each file a window at a time, in turn, where it read them all whole. Every row is what the same
//! files give on the route that reads them whole, which they take as named pipes (no regular file):
//! stdout, stderr and the exit status, with each pipe's name made its file's.
//!
//! The shapes that meet at a file's end or past the first window are the ones the windowed route has
//! to word as the whole one does: a document one file starts and the next ends, a number one file ends
//! and the next goes on with (jq reads the files as one stream of bytes), a fault and a document too
//! deep for jq's parser (whose message names where in jq's input it is) in a later file past the first
//! window, a pretty-printed document, a BOM at the start, and a blank and an empty file.

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
        let path = dir.path().join(format!("f{n:02}.json"));
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

/// `n` NDJSON records from `first` on, about 60 bytes each.
fn records(first: usize, n: usize) -> Vec<u8> {
    (first..first + n)
        .flat_map(|i| {
            format!(
                "{{\"id\":{i},\"name\":\"user {i}\",\"pad\":\"{}\"}}\n",
                "x".repeat(i % 37)
            )
            .into_bytes()
        })
        .collect()
}

/// About 4.5 MB in four files: a BOM and a pretty-printed document in the first, a document the first
/// starts and the second ends, a number the second ends and the third goes on with (`12` and `34` are
/// read as `1234`), an empty file, and a blank last one. `insert` goes into the third file 2.5 MB in,
/// past the first window.
fn files(insert: &[u8]) -> Vec<Vec<u8>> {
    let mut f0 = b"\xef\xbb\xbf{\n  \"pretty\": [\n    1,\n    2\n  ]\n}\n".to_vec();
    f0.extend(records(0, 20_000));
    f0.extend_from_slice(b"{\"split\":");
    let mut f1 = b" \"across\"}\n".to_vec();
    f1.extend(records(20_000, 20_000));
    f1.extend_from_slice(b"12");
    let mut f2 = b"34\n".to_vec();
    f2.extend(records(40_000, 40_000));
    let at = f2.len() - 300_000;
    f2.splice(at..at, insert.iter().copied());
    vec![f0, f1, f2, Vec::new(), b"\n \n".to_vec()]
}

const PROGRAMS: &[&[&str]] = &[
    &["-n", "[inputs] | length"],
    &["-cn", "[inputs | input_filename] | unique"],
    &["-cn", "inputs | [.id?, input_line_number, input_filename]"],
    &["-cn", "last(inputs) | [., input_line_number, $__loc__]"],
    &["-cn", "first(inputs), input_line_number"],
    &["-cn", "[limit(5; inputs)]"],
    &["-c", "[., input]"],
    &["-n", "reduce inputs as $x (0; . + 1)"],
    &["-cn", "[inputs | numbers]"],
    // A runtime error names the file and line jq's reader is at.
    &[
        "-cn",
        "inputs | if .id? == 45678 then error(\"stop\") else empty end",
    ],
    // A parse error's wording, caught.
    &["-n", "try ([inputs] | length) catch ."],
];

/// The rows for a stream with a fault: a program that reads every document stops at it, and these say
/// where it was and how it is worded.
const FAULT_PROGRAMS: &[&[&str]] = &[
    &["-cn", "inputs | [.id?, input_line_number, input_filename]"],
    &["-c", "[., input]"],
    &["-n", "try ([inputs] | length) catch ."],
];

#[test]
fn several_files_read_a_window_at_a_time_read_as_read_whole_7024() -> Result<()> {
    let deep = format!("{}{}\n", "[".repeat(300), "]".repeat(300));
    let inserts: [(&[u8], &[&[&str]]); 4] = [
        (b"", PROGRAMS),
        (b"{\"a\":1,}\n", FAULT_PROGRAMS),
        (b"\"\\x\"\n", FAULT_PROGRAMS),
        (deep.as_bytes(), FAULT_PROGRAMS),
    ];
    for (insert, programs) in inserts {
        let files = files(insert);
        assert!(files.iter().map(Vec::len).sum::<usize>() >= 4 << 20);
        for args in programs {
            let (windowed, whole) = both(args, &files)?;
            assert_eq!(
                windowed,
                whole,
                "{args:?} with {:?}",
                String::from_utf8_lossy(&insert[..insert.len().min(20)])
            );
        }
    }
    Ok(())
}

/// What the rows rest on: every document of the files is read, the one across the first two files and
/// the number across the next two among them.
#[test]
fn every_document_of_several_files_is_read_7024() -> Result<()> {
    let (windowed, _) = both(
        &["-cn", "[inputs] | length, .[0], .[20001], .[40002]"],
        &files(b""),
    )?;
    let want = "80003\n{\"pretty\":[1,2]}\n{\"split\":\"across\"}\n1234\n";
    assert_eq!(windowed, (want.to_string(), String::new(), 0));
    Ok(())
}
