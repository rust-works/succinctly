//! #7024: `-R` over a list of more than 32 regular files is read a window at a time as the one stream
//! of lines jq reads, each file opened when the run reaches it, where such a list was read whole. Every
//! row is what the same files give when the route declines them, which is when a path is not a regular
//! file: stdout, stderr and the exit status, with each file's name made one. The files are handed to
//! that route as named pipes ([`whole_by_descriptor`]): descriptors past 9 opened by `bash` 3.2,
//! macOS's, did not reopen as `/dev/fd/N` reliably (`Resource temporarily unavailable` on CI).
//!
//! The rules the lines meet by are those of `-R` over a few files (`i7024_raw_files.rs`): a file's last
//! line without a line break runs on into the next file's first and is read in that one, and each
//! file's bytes are decoded on their own. What this adds is that a path may change before the run gets
//! to it, which it then reads as it stands.

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

/// Lines with the shapes `-R` meets at a file's end: a CRLF line, invalid UTF-8, a BOM, a two-byte
/// character, and a line cut in two by a file boundary (`cut` moves every cut off a line start).
fn lines(count: usize) -> Vec<u8> {
    let mut bytes = text(count);
    bytes.splice(1000..1000, *b"\r\n");
    bytes.splice(200_000..200_000, *b"\xff\xfe");
    bytes.splice(300_000..300_000, *b"\xef\xbb\xbf");
    bytes.splice(400_000..400_000, "é".bytes());
    bytes
}

#[test]
fn raw_over_more_files_than_are_held_open_reads_as_one_stream_7024() -> Result<()> {
    let files = cut(&lines(60_000));
    assert!(files.iter().map(Vec::len).sum::<usize>() >= 4 << 20);
    let (stdout, stderr, status) = same(&["-R", "."], &files)?;
    assert_eq!(
        (stdout.lines().count() > 59_000, stderr.as_str(), status),
        (true, "", 0)
    );
    for args in [
        &["-r", "-R", "."][..],
        &["-R", "length"],
        &["-R", "select(test(\"9$\"))"],
        &["-R", "[input_filename, input_line_number]"],
        &["-R", "-c", "[., input_line_number]"],
        &["-nR", "[inputs] | length"],
        &["-nR", "first(inputs)"],
        &["-nR", "reduce inputs as $l (0; . + ($l | length))"],
        &["-R", "-e", "select(length > 100000)"],
        &["-R", "if length == 77 then halt_error else . end"],
        // A runtime error names the file and line jq's reader is at.
        &[
            "-R",
            "if contains(\"user 45000\") then error(\"stop\") else length end",
        ],
    ] {
        same(args, &files)?;
    }
    // Every file is reached.
    let (stdout, _, _) = same(&["-R", "input_filename"], &files)?;
    assert!(stdout.contains("<file 39>"), "every file is reached");
    Ok(())
}

#[test]
fn raw_over_a_bom_at_the_front_and_files_that_are_not_utf8_7024() -> Result<()> {
    let mut text = b"\xef\xbb\xbf".to_vec();
    text.extend_from_slice(&lines(60_000));
    text.splice(2_000_000..2_000_000, b"\xc3\n\xe2\x82\n".iter().copied());
    let files = cut(&text);
    same(&["-R", "."], &files)?;
    same(&["-R", "length"], &files)?;
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
    /// The last file removed, with a line open at the end of the file before it.
    LastGone,
    /// The last file replaced by a directory, which jq's reader does not report (#6472), with a line
    /// open at the end of the file before it.
    LastDirectory,
    /// Two files in a row removed.
    TwoGone,
    /// File 20 left in place but made unreadable: judged by `stat` before the run, it fails to open when
    /// the run gets there, and is reported as a file that is gone is, in the words of its error.
    MiddleUnreadable,
}

/// 40 files, the first big enough that the run is still in it when its output fills the pipe; the
/// change is made while the run waits to write, and the run is then drained. The answer is what the
/// files give afterwards on the route that reads them whole ([`whole_by_descriptor`]): the run met each
/// change when its reader got there, as jq's does.
fn changed_while_running(change: Change, args: &[&str]) -> Result<(Run, Run)> {
    use std::io::Read;
    let dir = tempfile::tempdir()?;
    let paths: Vec<std::path::PathBuf> = (0..FILES)
        .map(|n| dir.path().join(format!("f{n:02}.txt")))
        .collect();
    for (n, path) in paths.iter().enumerate() {
        let bytes = match n {
            // Just over the 4 MiB the route needs, and more output than a pipe holds.
            0 => {
                let bytes = text(60_000);
                assert!(bytes.len() >= 4 << 20, "{} bytes", bytes.len());
                bytes
            }
            19 => b"line 19\nopen 19 ".to_vec(),
            21 => b"end 21\nline 21\n".to_vec(),
            38 => b"line 38\nopen 38 ".to_vec(),
            n => format!("line {n}\n").into_bytes(),
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
            let other = dir.path().join("other.txt");
            std::fs::write(&other, b"56\nnew 21\n")?;
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
        Change::MiddleUnreadable => {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&paths[20], std::fs::Permissions::from_mode(0o0))?;
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
    // The whole-file route cannot be handed a file it cannot read, so an unreadable file is compared with
    // a missing one, which fails to open the same way but for its words.
    if matches!(change, Change::MiddleUnreadable) {
        std::fs::remove_file(&paths[20])?;
    }
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
        Change::LastDirectory,
        Change::TwoGone,
    ] {
        let programs = [
            &["-R", "."][..],
            &["-R", "[input_filename, .]"],
            &["-R", "-c", "[., input_line_number]"],
        ];
        for args in programs {
            let (windowed, whole) = changed_while_running(change, args)?;
            assert_eq!(windowed, whole, "{change:?} {args:?}");
        }
    }
    // A file that is there but cannot be opened (not when the tests run as root): the same run as a file
    // that is gone, with the words of its error.
    let readable = {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("probe");
        std::fs::write(&path, b"x")?;
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o0))?;
        std::fs::File::open(&path).is_ok()
    };
    if !readable {
        let (windowed, whole) = changed_while_running(Change::MiddleUnreadable, &["-R", "."])?;
        assert!(windowed.1.contains("Permission denied"), "{}", windowed.1);
        let gone = (
            windowed.0,
            windowed
                .1
                .replace("Permission denied", "No such file or directory"),
            windowed.2,
        );
        assert_eq!(gone, whole, "an unreadable file");
    }
    // The cases say what they are meant to: a file that did not open is reported, and the run
    // ends after the line the tail before it makes.
    let (run, _) = changed_while_running(Change::MiddleGone, &["-R", "."])?;
    assert!(run.1.contains("Could not open file"), "{}", run.1);
    assert_eq!(
        (run.0.lines().last(), run.2),
        (Some("\"open 19 end 21\""), 2)
    );
    let (run, _) = changed_while_running(Change::MiddleReplaced, &["-R", "."])?;
    assert!(
        run.0.contains("\"56\"") && run.0.contains("\"new 21\"") && !run.0.contains("end 21"),
        "the file as it now is"
    );
    Ok(())
}
