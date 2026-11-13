//! #7024: a program that reads `input`/`inputs` over several regular files, 4 MiB or more between them,
//! reads each file a window at a time, in turn, where it read them all whole. Every row is what the same
//! files give on the route that reads them whole, which they take as named pipes (no regular file):
//! stdout, stderr and the exit status, with each pipe's name made its file's (the harness is
//! `i7024_raw_files`'s).
//!
//! The files meet at line starts, so each later file is read in windows of its own; the shapes past the
//! first window of the third file are a fault, a bad escape and a document too deep for jq's parser
//! (whose message names where in jq's input it is, counted past the files before it). A second set has
//! the files meet inside a document and inside a number, which hands the rest of the stream to the eager
//! reader. The unit differential in `jq_runner.rs` (`several_files_read_in_windows_read_as_read_whole_7024`)
//! cuts streams at every byte; these rows are the end-to-end check at the route's real sizes.

#![cfg(unix)]

use anyhow::Result;

use super::i7024_raw_files::{both, Run};

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

/// About 4.5 MB in five files that meet at line starts: a BOM and a pretty-printed document in the
/// first, an empty file and a blank last one. `insert` goes into the third file at the line start about
/// 2.1 MB in, past its first window.
fn files(insert: &[u8]) -> Vec<Vec<u8>> {
    let mut f0 = b"\xef\xbb\xbf{\n  \"pretty\": [\n    1,\n    2\n  ]\n}\n".to_vec();
    f0.extend(records(0, 20_000));
    let f1 = records(20_000, 20_000);
    let mut f2 = records(40_000, 40_000);
    let at = f2[..f2.len() - 300_000]
        .iter()
        .rposition(|&b| b == b'\n')
        .expect("a line break")
        + 1;
    f2.splice(at..at, insert.iter().copied());
    vec![f0, f1, f2, Vec::new(), b"\n \n".to_vec()]
}

/// [`files`] with the first file ending inside a document the second ends, and the second inside a
/// number the third goes on with (`12` and `34` are read as `1234`).
fn seam_files() -> Vec<Vec<u8>> {
    let mut files = files(b"");
    files[0].extend_from_slice(b"{\"split\":");
    files[1].splice(0..0, *b" \"across\"}\n");
    files[1].extend_from_slice(b"12");
    files[2].splice(0..0, *b"34\n");
    files
}

/// Every document, with the line and file it was read in; the fault's wording on stderr.
const READ_ALL: &[&str] = &["-cn", "inputs | [.id?, input_line_number, input_filename]"];

/// The clean files' rows (each test stays under the 10 s a debug run may take, #5494).
const PROGRAMS: &[&[&str]] = &[
    READ_ALL,
    &["-c", "[.id?, (input | .id?)]"],
    &[
        "-cn",
        "inputs | if .id? == 45678 then error(\"stop\") else empty end",
    ],
];

fn check(args: &[&str], files: &[Vec<u8>], what: &str) -> Result<Run> {
    assert!(files.iter().map(Vec::len).sum::<usize>() >= 4 << 20);
    let (windowed, whole) = both(args, files)?;
    assert_eq!(windowed, whole, "{args:?} over {what}");
    Ok(windowed)
}

#[test]
fn several_clean_files_read_as_read_whole_7024() -> Result<()> {
    for args in PROGRAMS {
        check(args, &files(b""), "clean files")?;
    }
    Ok(())
}

/// A fault past a later file's first window is worded and placed as the whole route words it, the
/// depth refusal's position counted through the files before it.
#[test]
fn a_fault_in_a_later_file_reads_as_read_whole_7024() -> Result<()> {
    let deep = format!("{}{}\n", "[".repeat(300), "]".repeat(300));
    let faults: [(&[u8], &str); 3] = [
        (b"{\"a\":1,}\n", "Expected another key-value pair"),
        (b"\"\\x\"\n", "Invalid escape"),
        (deep.as_bytes(), "Exceeds depth limit for parsing"),
    ];
    for (insert, wording) in faults {
        let (_, stderr, _) = check(READ_ALL, &files(insert), wording)?;
        assert!(stderr.contains(wording), "{stderr}");
    }
    Ok(())
}

#[test]
fn files_that_meet_inside_a_document_read_as_read_whole_7024() -> Result<()> {
    check(
        READ_ALL,
        &seam_files(),
        "files that meet inside a document and a number",
    )?;
    Ok(())
}

/// What the rows rest on: every document of the files is read, the one across the first two files and
/// the number across the next two among them.
#[test]
fn every_document_of_several_files_is_read_7024() -> Result<()> {
    let (windowed, _) = both(
        &["-cn", "[inputs] | length, .[0], .[20001], .[40002]"],
        &seam_files(),
    )?;
    let want = "80003\n{\"pretty\":[1,2]}\n{\"split\":\"across\"}\n1234\n";
    assert_eq!(windowed, (want.to_string(), String::new(), 0));
    Ok(())
}

/// A file whose `stat` size is 0 and holds bytes (`/proc`) is read to its end, as jq reads it.
#[cfg(target_os = "linux")]
#[test]
fn a_file_stat_calls_empty_is_read_to_its_end_7024() -> Result<()> {
    let proc = std::path::Path::new("/proc/sys/kernel/pid_max");
    if !proc.exists() {
        return Ok(());
    }
    let dir = tempfile::tempdir()?;
    let big = dir.path().join("big.json");
    std::fs::write(&big, files(b"").concat())?;
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-cn", "[inputs] | length, .[-1]"])
        .arg(&big)
        .arg(proc)
        .output()?;
    let pid_max = std::fs::read_to_string(proc)?;
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("80002\n{}\n", pid_max.trim())
    );
    Ok(())
}
