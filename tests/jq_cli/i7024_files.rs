//! #7024: several regular files are read a window at a time as the one stream jq reads them as, where
//! they were read whole. Every row is what the same files give when the route declines them, which is
//! when a path is `/dev/fd/N`: stdout, stderr and the exit status, with each file's name made one.
//!
//! The files here are cut from one stream at chosen bytes, a token open where a file ends and a
//! multi-byte character split between two of them included, because that is where the files meet.

#![cfg(unix)]

use std::io::Write;
use std::process::Command;

use anyhow::Result;

const LINES: usize = 60_000;

fn lines(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            format!(
                "{{\"id\":{i},\"name\":\"user {i}\",\"ok\":true,\"tags\":[\"a\",\"b\"],\"score\":{}}}",
                i % 1000
            )
        })
        .collect()
}

fn join(lines: &[String]) -> Vec<u8> {
    let mut bytes = lines.join("\n").into_bytes();
    bytes.push(b'\n');
    bytes
}

/// `bytes` cut at the offsets `at` (in order) into files.
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

type Run = (String, String, i32);

fn finish(output: std::process::Output, names: &[String], descriptors: &[usize]) -> Run {
    let name = |text: &[u8]| {
        let mut text = String::from_utf8_lossy(text).into_owned();
        for (n, (path, fd)) in names.iter().zip(descriptors).enumerate() {
            text = text.replace(path.as_str(), &format!("<file {n}>"));
            text = text.replace(&format!("/dev/fd/{fd}"), &format!("<file {n}>"));
        }
        text
    };
    (
        name(&output.stdout),
        name(&output.stderr),
        output.status.code().unwrap_or(-1),
    )
}

/// The files as paths (the windowed route), and as descriptors (the route that reads them whole).
fn both(args: &[&str], files: &[Vec<u8>]) -> Result<(Run, Run)> {
    // Descriptors 3 to 9: `sh` takes a longer number as an operand, and `/dev/fd/3` is a prefix of `/dev/fd/30`.
    assert!(files.len() <= 7, "at most seven files");
    let dir = tempfile::tempdir()?;
    let mut paths = Vec::new();
    for (n, bytes) in files.iter().enumerate() {
        let path = dir.path().join(format!("f{n}.json"));
        std::fs::File::create(&path)?.write_all(bytes)?;
        paths.push(path.to_string_lossy().into_owned());
    }
    let bin = env!("CARGO_BIN_EXE_succinctly");
    let windowed = Command::new(bin)
        .arg("jq")
        .args(args)
        .args(&paths)
        .env("LC_ALL", "C")
        .output()?;
    // The same files on descriptors 3, 4, ...: `sh` opens them and `exec`s the binary.
    let descriptors: Vec<usize> = (3..3 + files.len()).collect();
    let redirects: String = descriptors
        .iter()
        .enumerate()
        .map(|(n, fd)| format!(" {fd}<\"$F{n}\""))
        .collect();
    let operands: Vec<String> = descriptors
        .iter()
        .map(|fd| format!("/dev/fd/{fd}"))
        .collect();
    let mut shell = Command::new("sh");
    shell
        .arg("-c")
        .arg(format!("exec \"$@\"{redirects}"))
        .arg("sh")
        .arg(bin)
        .arg("jq")
        .args(args)
        .args(&operands)
        .env("LC_ALL", "C");
    for (n, path) in paths.iter().enumerate() {
        shell.env(format!("F{n}"), path);
    }
    let whole = shell.output()?;
    Ok((
        finish(windowed, &paths, &descriptors),
        finish(whole, &paths, &descriptors),
    ))
}

/// The two runs, which must be the same; the windowed one is returned.
fn same(args: &[&str], files: &[Vec<u8>]) -> Result<Run> {
    let total: usize = files.iter().map(Vec::len).sum();
    assert!(
        total > 4 * 1024 * 1024,
        "the files must be over the minimum"
    );
    let (windowed, whole) = both(args, files)?;
    assert!(
        windowed.0 == whole.0,
        "{args:?}: stdout differs ({} and {} bytes)",
        windowed.0.len(),
        whole.0.len()
    );
    assert_eq!(windowed.1, whole.1, "{args:?}: stderr");
    assert_eq!(windowed.2, whole.2, "{args:?}: exit status");
    Ok(windowed)
}

/// Programs that stay on the route (`input_line_number`, `input` and the like take another), and say
/// nothing on stderr.
const PROGRAMS: &[&[&str]] = &[
    &["-c", "."],
    &["-c", "select(.id % 5003 == 0) | [input_filename, .id]"],
    &["-r", "select(.id % 9973 == 0) | .name"],
    &["-c", "[.. | numbers] | length"],
];

/// A runtime error names the line its document was read on, in its file: `(at <file N>:LINE)`.
const ERROR_PROGRAM: &[&str] = &["-c", "select(.id % 4001 == 0) | error(\"boom\")"];

#[test]
fn files_cut_at_line_starts_read_as_one_stream_7024() -> Result<()> {
    let bytes = join(&lines(LINES));
    let third = |n: usize| {
        let at = bytes.len() * n / 3;
        bytes[..at].iter().rposition(|&b| b == b'\n').unwrap() + 1
    };
    let files = cut(&bytes, &[third(1), third(2)]);
    for args in PROGRAMS {
        let (out, err, status) = same(args, &files)?;
        assert_eq!((err.as_str(), status), ("", 0), "{args:?}");
        assert!(!out.is_empty(), "{args:?}");
    }
    // Which file each document was read in, and its line there.
    // (The exit status is that of the last document, as jq's is: the same either way.)
    let (out, err, _) = same(ERROR_PROGRAM, &files)?;
    assert_eq!(out, "");
    assert!(
        err.starts_with("jq: error (at <file 0>:1): boom\n"),
        "{err}"
    );
    assert!(err.contains("(at <file 1>:"), "{err}");
    assert!(err.contains("(at <file 2>:"), "{err}");
    let (out, _, _) = same(
        &[
            "-c",
            "select(.id == 0 or .id == 30000 or .id == 59999) | [input_filename, .id]",
        ],
        &files,
    )?;
    assert!(out.starts_with("[\"<file 0>\",0]\n"), "{out}");
    assert!(out.contains("\"<file 1>\""), "{out}");
    assert!(out.ends_with(",59999]\n"), "{out}");
    Ok(())
}

#[test]
fn a_document_open_where_a_file_ends_continues_in_the_next_7024() -> Result<()> {
    let bytes = join(&lines(LINES));
    let mid = |needle: &[u8], skip: usize| {
        let at = bytes
            .windows(needle.len())
            .position(|w| w == needle)
            .expect("in the stream");
        at + skip
    };
    // Inside a string, a number, a keyword, between a key and its value, after a comma; and a
    // document that spans three files.
    let cuts = [
        mid(b"\"user 20000\"", 5),
        mid(b"\"id\":31234", 8),
        mid(b"\"ok\":tr", 7),
        mid(b"\"id\":41000", 5),
        mid(b"{\"id\":50000", 1),
    ];
    for at in cuts {
        let files = cut(&bytes, &[at]);
        for args in &PROGRAMS[..2] {
            let (_, err, status) = same(args, &files)?;
            assert_eq!((err.as_str(), status), ("", 0), "{args:?} at {at}");
        }
        // The error names the same lines either way.
        let (_, err, _) = same(ERROR_PROGRAM, &files)?;
        assert!(err.starts_with("jq: error (at <file "), "at {at}: {err}");
    }
    // The values completed in the second file are that file's: its name and its lines.
    let at = mid(b"\"user 20000\"", 5);
    let (_, err, _) = same(
        &["-c", "select(.id == 20000) | error(\"x\")"],
        &cut(&bytes, &[at]),
    )?;
    assert_eq!(err, "jq: error (at <file 1>:1): x\n");
    let (out, _, _) = same(
        &["-c", "select(.id == 20000) | [input_filename, .name]"],
        &cut(&bytes, &[at]),
    )?;
    assert_eq!(out, "[\"<file 1>\",\"user 20000\"]\n");
    // A document across three files, the middle one holding none of its own.
    let files = cut(&bytes, &[at, at + 3]);
    let (out, _, _) = same(&["-c", "select(.id == 20000) | input_filename"], &files)?;
    assert_eq!(out, "\"<file 2>\"\n");
    // And across an empty file.
    let files = cut(&bytes, &[at, at, at]);
    same(&["-c", "select(.id == 20000) | input_filename"], &files)?;
    Ok(())
}

#[test]
fn a_fault_is_worded_over_the_whole_stream_7024() -> Result<()> {
    let mut faulty = lines(LINES);
    faulty[45_000] = "}".to_string();
    let bytes = join(&faulty);
    let at = bytes.len() / 2;
    let program = &["-c", "select(.id % 10000 == 0) | .id"][..];
    let (out, err, status) = same(program, &cut(&bytes, &[at]))?;
    assert_eq!(out, "0\n10000\n20000\n30000\n40000\n");
    assert_eq!(
        err,
        "jq: parse error: Unmatched '}' at line 45001, column 1\n"
    );
    assert_ne!(status, 0);
    // A fault in the first file stops the run before the second is read.
    let early = bytes.len() / 10;
    let mut faulty = lines(LINES);
    faulty[2_000] = "}".to_string();
    let bytes = join(&faulty);
    let (_, err, _) = same(program, &cut(&bytes, &[early + 1_000_000]))?;
    assert_eq!(
        err,
        "jq: parse error: Unmatched '}' at line 2001, column 1\n"
    );
    // The stream ends inside a document, in a later file.
    let mut cut_off = join(&lines(LINES));
    cut_off.extend_from_slice(b"{\"id\":1,");
    let at = cut_off.len() / 3;
    same(program, &cut(&cut_off, &[at, 2 * at]))?;
    // A bracket opened in one file and never closed, the next file's documents inside it.
    let mut open = join(&lines(30_000));
    let split = open.len();
    open.extend_from_slice(b"[1,\n");
    open.extend(join(&lines(30_000)));
    same(program, &cut(&open, &[split]))?;
    // A depth refusal, named at the position it is in the stream of all the files.
    let mut deep = join(&lines(30_000));
    let split = deep.len();
    deep.extend(std::iter::repeat_n(b'[', 11_000));
    deep.extend_from_slice(b"\n");
    deep.extend(join(&lines(30_000)));
    let (_, err, _) = same(&["-c", "."], &cut(&deep, &[split - 10, split + 5]))?;
    assert!(err.contains("depth"), "{err}");
    Ok(())
}

#[test]
fn a_bom_is_the_front_of_the_stream_and_no_other_7024() -> Result<()> {
    let bytes = join(&lines(LINES));
    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend(&bytes);
    let at = bom.len() / 2;
    same(&["-c", "select(.id == 0) | .name"], &cut(&bom, &[at]))?;
    // Empty files ahead of it.
    let mut files = vec![Vec::new(), Vec::new()];
    files.extend(cut(&bom, &[at]));
    same(&["-c", "select(.id == 0) | .name"], &files)?;
    // A BOM at the front of a later file is a byte of the stream: where it sits between documents, a
    // fault.
    let at = bytes[..at].iter().rposition(|&b| b == b'\n').unwrap() + 1;
    let mut later = cut(&bytes, &[at]);
    let mut second = b"\xef\xbb\xbf".to_vec();
    second.extend(&later[1]);
    later[1] = second;
    let (_, err, status) = same(&["-c", "."], &later)?;
    assert_ne!(status, 0);
    assert!(err.starts_with("jq: parse error:"), "{err}");
    Ok(())
}

#[test]
fn text_that_is_not_utf8_is_substituted_as_the_files_joined_are_7024() -> Result<()> {
    let bytes = join(&lines(LINES));
    let at = bytes
        .windows(12)
        .position(|w| w == b"\"user 30000\"")
        .unwrap();
    let with = |insert: &[u8], at: usize| {
        let mut bytes = bytes.clone();
        bytes.splice(at..at, insert.iter().copied());
        bytes
    };
    let program = &["-c", "select(.id == 30000 or .id == 30001) | .name"][..];
    // A lone invalid byte inside a string, in the file or at its end.
    same(program, &cut(&with(b"\xff", at + 3), &[bytes.len() / 2]))?;
    // A character split between two files, whole once joined.
    let euro = with("\u{20ac}".as_bytes(), at + 3);
    same(program, &cut(&euro, &[at + 4]))?;
    same(program, &cut(&euro, &[at + 5]))?;
    // The same character split, but the bytes after the cut are not its continuation.
    let mut broken = with(b"\xe2\x82", at + 3);
    broken.splice(at + 5..at + 5, *b"a");
    same(program, &cut(&broken, &[at + 5]))?;
    // A run of invalid bytes run across the cut collapses to one replacement, as it does in one file.
    let run = with(b"\xff\xfe\xfd", at + 3);
    for cut_at in [at + 4, at + 5, at + 6] {
        same(program, &cut(&run, &[cut_at]))?;
    }
    Ok(())
}

#[test]
fn files_that_are_empty_or_hold_only_white_space_read_as_the_rest_do_7024() -> Result<()> {
    let bytes = join(&lines(LINES));
    let at = bytes.len() / 2;
    let mut files = vec![Vec::new(), b"  \n\n".to_vec()];
    files.extend(cut(&bytes, &[at]));
    files.push(Vec::new());
    files.push(b"\n".to_vec());
    let (out, err, status) = same(&["-c", "select(.id == 59999) | input_filename"], &files)?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        ("\"<file 3>\"\n", "", 0)
    );
    // Nothing but empty files after the big one, and a token open at the end of the last.
    let mut tail = vec![bytes.clone(), Vec::new()];
    tail.push(Vec::new());
    same(&["-c", "select(.id == 59999) | input_filename"], &tail)?;
    let mut cut_off = bytes.clone();
    cut_off.extend_from_slice(b"12");
    same(&["-c", "."], &[cut_off, Vec::new()])?;
    Ok(())
}

/// jq reads a line 4095 bytes at a time and names the line a document's read ends on: the open tail of
/// one file, long and with a line break in it, finished by a document at the front of the next, puts the
/// reads of the next file's first line at offsets that only the seam knows.
#[test]
fn the_reads_of_a_files_first_line_are_counted_from_where_the_carry_ends_7024() -> Result<()> {
    let mut first = join(&lines(45_000));
    first.extend_from_slice(b"[\n");
    first.extend(std::iter::repeat_n(*b"1,", 2045).flatten());
    let mut second = b"1] 7 [\n".to_vec();
    second.extend(std::iter::repeat_n(*b"1\n", 400_000).flatten());
    second.extend_from_slice(b"]\n");
    second.extend(join(&lines(5_000)));
    let program = &[
        "-c",
        "if type == \"number\" then error(\"n\") else empty end",
    ][..];
    let (_, err, _) = same(program, &[first, second])?;
    assert!(err.starts_with("jq: error (at <file 1>:1): n\n"), "{err}");
    Ok(())
}

/// An input that is also the run's output is read whole, as before: output landing in a file the next
/// window is read from would be read as input, or overwrite what was not yet read.
#[test]
fn a_file_that_is_also_the_output_is_read_whole_7024() -> Result<()> {
    let big = join(&lines(LINES));
    let small = join(&lines(10));
    let dir = tempfile::tempdir()?;
    let (a, b) = (dir.path().join("a.json"), dir.path().join("b.json"));
    std::fs::write(&a, &big)?;
    std::fs::write(&b, &small)?;
    // `1<>b.json` opens the second file read-write as standard output.
    let output = Command::new("sh")
        .arg("-c")
        .arg("exec \"$0\" jq -c . \"$1\" \"$2\" 1<>\"$2\"")
        .arg(env!("CARGO_BIN_EXE_succinctly"))
        .arg(&a)
        .arg(&b)
        .output()?;
    assert!(output.status.success(), "{:?}", output.status);
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    // What the whole-file route writes into b.json: all of a's records, then b's.
    let written = std::fs::read(&b)?;
    let want_lines = lines(LINES).len() + 10;
    assert_eq!(written.iter().filter(|&&c| c == b'\n').count(), want_lines);
    Ok(())
}

/// A regular file whose `stat` size is 0 and which has content (the `/proc` kind) is read to its end,
/// as `std::fs::read` reads it.
#[cfg(target_os = "linux")]
#[test]
fn a_file_whose_size_is_not_known_is_read_to_its_end_7024() -> Result<()> {
    let proc_file = "/proc/sys/vm/swappiness";
    let Ok(value) = std::fs::read_to_string(proc_file) else {
        return Ok(()); // no procfs here
    };
    let big = join(&lines(LINES));
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("big.json");
    std::fs::write(&path, &big)?;
    for files in [
        [path.to_str().unwrap(), proc_file],
        [proc_file, path.to_str().unwrap()],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(["jq", "-c", "."])
            .args(files)
            .output()?;
        assert!(output.status.success(), "{files:?}: {:?}", output.status);
        let text = String::from_utf8_lossy(&output.stdout);
        let value = value.trim();
        assert!(
            text.starts_with(&format!("{value}\n")) || text.ends_with(&format!("\n{value}\n")),
            "{files:?}: the pseudo-file's {value} is missing"
        );
        assert_eq!(text.lines().count(), LINES + 1, "{files:?}");
    }
    Ok(())
}
