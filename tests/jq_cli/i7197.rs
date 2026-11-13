//! #7197: a program that reads `input`/`inputs` over more than 32 regular files reads each a window at a
//! time, each file opened when the run reaches it, where such a list was read whole. Every row is what the
//! same files give when the route declines them, which is when a path is not a regular file: stdout, stderr
//! and the exit status, with each file's name made one. The files are handed to that route as named pipes
//! ([`whole_by_descriptor`]): descriptors past 9 opened by `bash` 3.2, macOS's, did not reopen as
//! `/dev/fd/N` reliably (`Resource temporarily unavailable` on CI).
//!
//! The rules the documents meet by are those of the same program over a few files (`i7024_inputs_files.rs`);
//! what this adds is that a path may change before the run gets to it, which it then reads as it stands.

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

type Run = (String, String, i32);

/// The run over `paths` on the route that reads them whole: each path that names a file is given as a
/// named pipe (`mkfifo`) that a thread fills with the file's bytes, which is no regular file and so
/// sends the list to that route, and a path that does not (gone, or a directory) is given as itself.
/// The output names each file by its path again. A pipe the run never opened is opened here once it
/// exits, so its writer ends.
fn whole_by_descriptor(args: &[&str], paths: &[std::path::PathBuf]) -> Result<Run> {
    let dir = tempfile::tempdir()?;
    let mut operands = Vec::new();
    let mut writers = Vec::new();
    for (n, path) in paths.iter().enumerate() {
        if !path.is_file() {
            operands.push(path.clone());
            continue;
        }
        let fifo = dir.path().join(format!("p{n:02}"));
        let made = Command::new("mkfifo").arg(&fifo).status()?;
        assert!(made.success(), "mkfifo {}", fifo.display());
        let bytes = std::fs::read(path)?;
        let target = fifo.clone();
        // Opening a pipe for writing waits for its reader; a write the reader leaves fails, and is
        // no concern of the test.
        writers.push((
            fifo.clone(),
            std::thread::spawn(move || {
                if let Ok(mut pipe) = std::fs::OpenOptions::new().write(true).open(&target) {
                    let _ = pipe.write_all(&bytes);
                }
            }),
        ));
        operands.push(fifo);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .args(&operands)
        .env("LC_ALL", "C")
        .output()?;
    for (fifo, writer) in writers {
        if !writer.is_finished() {
            drop(std::fs::File::open(&fifo));
        }
        writer.join().expect("the writer ends");
    }
    let named = |text: &[u8]| {
        let mut text = String::from_utf8_lossy(text).into_owned();
        for (operand, path) in operands.iter().zip(paths) {
            text = text.replace(&operand.display().to_string(), &path.display().to_string());
        }
        text
    };
    Ok((
        named(&output.stdout),
        named(&output.stderr),
        output.status.code().unwrap_or(-1),
    ))
}

/// The files as paths (the windowed route), and as descriptors (the route that reads them whole).
fn both(args: &[&str], files: &[Vec<u8>]) -> Result<(Run, Run)> {
    let dir = tempfile::tempdir()?;
    let mut paths = Vec::new();
    for (n, bytes) in files.iter().enumerate() {
        let path = dir.path().join(format!("f{n:02}.json"));
        std::fs::File::create(&path)?.write_all(bytes)?;
        paths.push(path);
    }
    let name = |run: Run| -> Run {
        let name = |mut text: String| {
            // The longest first: `f1.json` is not a prefix of `f10.json`, but keep the order safe.
            for (n, path) in paths.iter().enumerate().rev() {
                text = text.replace(&path.display().to_string(), &format!("<file {n}>"));
            }
            text
        };
        (name(run.0), name(run.1), run.2)
    };
    let windowed = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .args(&paths)
        .env("LC_ALL", "C")
        .output()?;
    let windowed = (
        String::from_utf8_lossy(&windowed.stdout).into_owned(),
        String::from_utf8_lossy(&windowed.stderr).into_owned(),
        windowed.status.code().unwrap_or(-1),
    );
    Ok((name(windowed), name(whole_by_descriptor(args, &paths)?)))
}

fn same(args: &[&str], files: &[Vec<u8>]) -> Result<Run> {
    let (windowed, whole) = both(args, files)?;
    assert_eq!(windowed, whole, "{args:?}");
    Ok(windowed)
}

/// `bytes` cut at line starts into `FILES` files of near the same size.
fn cut_at_lines(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut files = Vec::new();
    let mut from = 0;
    for n in 1..FILES {
        let at = bytes[..n * bytes.len() / FILES]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |at| at + 1);
        files.push(bytes[from..at.max(from)].to_vec());
        from = at.max(from);
    }
    files.push(bytes[from..].to_vec());
    files
}

#[test]
fn input_over_more_files_than_are_held_open_reads_as_one_stream_7197() -> Result<()> {
    let files = cut_at_lines(&text(60_000));
    assert!(files.iter().map(Vec::len).sum::<usize>() >= 4 << 20);
    for args in [
        &["-c", "-n", "[inputs] | length"][..],
        &["-c", "-n", "first(inputs) | .id"],
        &["-c", "-n", "reduce inputs as $x (0; . + 1)"],
        &["-c", "-n", "[limit(5; inputs | .id)]"],
        &["-c", "[., input] | length"],
        &["-c", "-n", "input_filename, (input | input_filename), (inputs | select(.id == 45000) | input_filename)"],
        &["-c", "-n", "[inputs | select(.id % 9999 == 0) | .id]"],
        &["-c", "-n", "[inputs | input_line_number] | add"],
        // A runtime error names the file and line jq's reader is at.
        &["-c", "-n", "inputs | if .id == 45000 then error(\"stop\") else .id end"],
        &["-c", "-n", "inputs | select(.id > 59990) | .id"],
    ] {
        same(args, &files)?;
    }
    Ok(())
}

#[test]
fn a_bom_at_the_front_is_skipped_when_the_first_file_is_opened_7197() -> Result<()> {
    let mut files = cut_at_lines(&text(60_000));
    files[0].splice(0..0, b"\xef\xbb\xbf".iter().copied());
    for args in [
        &["-c", "-n", "[inputs] | length"][..],
        &["-c", "-n", "first(inputs) | .id"],
    ] {
        let run = same(args, &files)?;
        assert_eq!(run.2, 0, "{args:?}: {}", run.1);
    }
    Ok(())
}

#[test]
fn a_fault_or_an_open_token_hands_the_rest_to_the_eager_reader_7197() -> Result<()> {
    let bytes = text(60_000);
    for (at, bad) in [
        (bytes.len() / 3, &b"{\"bad\": [1 2]}\n"[..]),
        (bytes.len() / 2, b"}\n"),
        (bytes.len(), b"{\"last\": "),
    ] {
        let at = bytes[..at]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |n| n + 1);
        let mut text = bytes[..at].to_vec();
        text.extend_from_slice(bad);
        text.extend_from_slice(&bytes[at..]);
        let files = cut_at_lines(&text);
        for args in [
            &["-c", "-n", "[inputs] | length"][..],
            &["-c", "-n", "try ([inputs] | length) catch ."],
            &["-c", "-n", "first(inputs) | .id"],
        ] {
            same(args, &files)?;
        }
    }
    // A number left open where a file ends goes on in the next.
    let mut files = cut_at_lines(&bytes);
    files[19].extend_from_slice(b"12");
    files[20].splice(0..0, b"34\n".iter().copied());
    for args in [
        &["-c", "-n", "[inputs] | length"][..],
        &["-c", "-n", "[inputs | numbers]"],
    ] {
        same(args, &files)?;
    }
    Ok(())
}

/// What changes at the paths of a long list while the run is in its first file.
#[derive(Clone, Copy, Debug)]
enum Change {
    /// File 20 removed: the tail of file 19 continues in file 21.
    MiddleGone,
    /// File 20 replaced by a directory, which jq reports as `Is a directory`.
    MiddleDirectory,
    /// File 21 replaced by another file, which is read as it now is.
    MiddleReplaced,
    /// The last file removed, with a number open at the end of the file before it.
    LastGone,
    /// The last file replaced by a directory, which jq's reader does not report (#6472).
    LastDirectory,
    /// Two files in a row removed.
    TwoGone,
}

/// 40 files, the first big enough that the run is still in it when its output fills the pipe; the
/// change is made while the run waits to write, and the run is then drained. The answer is what the
/// files give afterwards on the route that reads them whole ([`whole_by_descriptor`]).
fn changed_while_running(change: Change, args: &[&str]) -> Result<(Run, Run)> {
    use std::io::Read;
    let dir = tempfile::tempdir()?;
    let paths: Vec<std::path::PathBuf> = (0..FILES)
        .map(|n| dir.path().join(format!("f{n:02}.json")))
        .collect();
    for (n, path) in paths.iter().enumerate() {
        let bytes = match n {
            // Just over the 4 MiB the route needs, and more output than a pipe holds.
            0 => {
                let bytes = text(60_000);
                assert!(bytes.len() >= 4 << 20, "{} bytes", bytes.len());
                bytes
            }
            19 => b"{\"k\":19}\n12".to_vec(),
            21 => b"34\n{\"k\":21}\n".to_vec(),
            38 => b"{\"k\":38}\n77".to_vec(),
            n => format!("{{\"k\":{n}}}\n").into_bytes(),
        };
        std::fs::write(path, bytes)?;
    }
    let bin = env!("CARGO_BIN_EXE_succinctly");
    let mut child = Command::new(bin)
        .arg("jq")
        .args(args)
        .args(&paths)
        .env("LC_ALL", "C")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("piped");
    let mut first = vec![0u8; 4096];
    stdout.read_exact(&mut first)?;
    match change {
        Change::MiddleGone => std::fs::remove_file(&paths[20])?,
        Change::MiddleDirectory => {
            std::fs::remove_file(&paths[20])?;
            std::fs::create_dir(&paths[20])?;
        }
        Change::MiddleReplaced => {
            let other = dir.path().join("other.json");
            std::fs::write(&other, b"56\n{\"new\":21}\n")?;
            std::fs::rename(&other, &paths[21])?;
        }
        Change::LastGone => std::fs::remove_file(&paths[FILES - 1])?,
        Change::LastDirectory => {
            std::fs::remove_file(&paths[FILES - 1])?;
            std::fs::create_dir(&paths[FILES - 1])?;
        }
        Change::TwoGone => {
            std::fs::remove_file(&paths[22])?;
            std::fs::remove_file(&paths[23])?;
        }
    }
    let mut rest = Vec::new();
    stdout.read_to_end(&mut rest)?;
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .expect("piped")
        .read_to_end(&mut stderr)?;
    let status = child.wait()?;
    first.extend(rest);
    let windowed = (
        String::from_utf8_lossy(&first).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
        status.code().unwrap_or(-1),
    );
    let whole = whole_by_descriptor(args, &paths)?;
    Ok((windowed, whole))
}

#[test]
fn a_path_that_changes_before_the_run_reaches_it_is_read_as_jq_reads_it_7197() -> Result<()> {
    for change in [
        Change::MiddleGone,
        Change::MiddleDirectory,
        Change::MiddleReplaced,
        Change::LastGone,
        Change::LastDirectory,
        Change::TwoGone,
    ] {
        for args in [
            &["-c", "-n", "inputs"][..],
            &["-c", "-n", "inputs | [input_filename, .]"],
            &[
                "-c",
                "-n",
                "inputs | select(type == \"object\") | tojson | length",
            ],
        ] {
            let (windowed, whole) = changed_while_running(change, args)?;
            assert_eq!(windowed, whole, "{change:?} {args:?}");
        }
    }
    // The cases say what they are meant to: a file that did not open is reported, and the run ends.
    let (run, _) = changed_while_running(Change::MiddleGone, &["-c", "-n", "inputs"])?;
    assert!(run.1.contains("Could not open file"), "{}", run.1);
    assert_eq!(run.2, 2);
    Ok(())
}
