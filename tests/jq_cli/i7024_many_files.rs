//! #7024: a list of more than 32 regular files is read a window at a time as the one stream jq reads
//! them as, each file opened when the run reaches it, where such a list was read whole. Every row is
//! what the same files give when the route declines them, which is when a path is not a regular file:
//! stdout, stderr and the exit status, with each file's name made one. The files are handed to that
//! route as named pipes ([`whole_by_descriptor`]): descriptors past 9 opened by `bash` 3.2, macOS's,
//! did not reopen as `/dev/fd/N` reliably (`Resource temporarily unavailable` on CI).

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
    /// File 30 removed, after a fault in file 20 that ends the run first.
    GoneAfterAFault,
    /// The last file replaced by a directory, which jq's reader does not report (#6472), with a
    /// number open at the end of the file before it.
    LastDirectory,
}

/// 40 files, the first big enough that the run is still in it when its output fills the pipe; the
/// change is made while the run waits to write, and the run is then drained. The answer is what the
/// files give afterwards on the route that reads them whole ([`whole_by_descriptor`]): the run met each
/// change when its reader got there, as jq's does.
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
            20 if matches!(change, Change::GoneAfterAFault) => b"{\"k\":20} }\n".to_vec(),
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
        Change::GoneAfterAFault => std::fs::remove_file(&paths[30])?,
        Change::LastDirectory => {
            std::fs::remove_file(&paths[FILES - 1])?;
            std::fs::create_dir(&paths[FILES - 1])?;
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
fn a_path_that_changes_before_the_run_reaches_it_is_read_as_jq_reads_it_7024() -> Result<()> {
    for change in [
        Change::MiddleGone,
        Change::MiddleDirectory,
        Change::MiddleReplaced,
        Change::LastGone,
        Change::GoneAfterAFault,
        Change::LastDirectory,
    ] {
        // A halt on the value held before a last file that changed (#6683).
        let last = matches!(change, Change::LastGone | Change::LastDirectory);
        let halts = [&["-c", "if . == 77 then halt_error else . end"][..]];
        let programs = [&["-c", "."][..], &["-c", "[input_filename, .]"]];
        for args in programs.iter().chain(halts.iter().filter(|_| last)) {
            let (windowed, whole) = changed_while_running(change, args)?;
            assert_eq!(windowed, whole, "{change:?} {args:?}");
        }
    }
    // The cases say what they are meant to: a file that did not open is reported, and the run
    // ends after the value the tail before it makes.
    let (run, _) = changed_while_running(Change::MiddleGone, &["-c", "."])?;
    assert!(run.1.contains("Could not open file"), "{}", run.1);
    assert_eq!((run.0.lines().last(), run.2), (Some("1234"), 2));
    let (run, _) = changed_while_running(Change::MiddleReplaced, &["-c", "."])?;
    assert!(
        run.0.contains("{\"new\":21}") && run.0.contains("1256"),
        "the file as it now is"
    );
    let (run, _) = changed_while_running(Change::GoneAfterAFault, &["-c", "."])?;
    assert!(
        !run.1.contains("Could not open"),
        "the reader stops at the fault: {}",
        run.1
    );
    Ok(())
}
