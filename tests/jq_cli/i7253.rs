//! #7253: a `--seq` run over more than 32 regular files (#7221) opens each file when it reaches it, so a file
//! that opened when the list was judged can be gone when the run gets there. jq's main loop ends after the next
//! value once its reader has met a file that did not open, and the run that has handed the files from the first
//! that is not plain records to the route that reads them whole must carry that failure over
//! (`hand_on_seq_files_tail`): it reports the failure, prints the tail's first value, stops, and exits 2.
//!
//! The file is removed while the run is blocked writing to a pipe nobody is reading: the test reads one line of
//! output, which the run writes only once it has judged the list and begun it, removes the file, and then
//! reads on. Each file before the gone one is some 100 KB of output, far more than a pipe holds, so the run is
//! still many files short of it at the removal and reaches it afterwards, which no sleep could promise.
//! Every row is compared with the same files when the file was never there (the judgement turns that list
//! down, so the whole-file route runs it) and pins what `/usr/bin/jq` 1.7.1 gave over them, captured by hand: the values
//! before the file, the failure, the first value after it, and exit 2.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Result;

type Run = (String, String, i32);

/// More than the 32 files a list can have and still be held open from the start: a list this long is opened
/// by path, each file when the run reaches it, so every one of them, the gone one included, is closed until then.
const FILES: usize = 40;
const EACH: usize = 1_000;
/// The file that is gone when the run reaches it, far enough along for the run to be blocked on its output
/// well before it.
const GONE: usize = 30;

const ARGS: [&str; 3] = ["--seq", "-c", "[input_filename, .id]"];

fn records(from: usize, count: usize) -> Vec<u8> {
    (from..from + count)
        .flat_map(|i| format!("\x1e{{\"id\":{i},\"pad\":\"{}\"}}\n", "x".repeat(90)).into_bytes())
        .collect()
}

/// 40 files of framed records, each of the `unframed` files without its first record separator, so that it
/// is not plain `RS value LF` records and ends the windows.
fn files(unframed: &[usize]) -> Vec<Vec<u8>> {
    (0..FILES)
        .map(|n| {
            let mut bytes = records(n * EACH, EACH);
            if unframed.contains(&n) {
                bytes.remove(0);
            }
            bytes
        })
        .collect()
}

/// Writes every file but `missing` into `dir`, and returns all the paths, that one's included.
fn write(dir: &Path, files: &[Vec<u8>], missing: Option<usize>) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for (n, bytes) in files.iter().enumerate() {
        let path = dir.join(format!("f{n:02}.seq"));
        if missing != Some(n) {
            std::fs::File::create(&path)?.write_all(bytes)?;
        }
        paths.push(path);
    }
    Ok(paths)
}

/// What the run said, each file's name made one.
fn named(paths: &[PathBuf], bytes: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    for (n, path) in paths.iter().enumerate().rev() {
        text = text.replace(&path.display().to_string(), &format!("<file {n}>"));
    }
    text
}

fn command(paths: &[PathBuf]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command.arg("jq").args(ARGS).args(paths).env("LC_ALL", "C");
    command
}

/// The run over the files with `GONE` absent from the start.
fn never_there(files: &[Vec<u8>]) -> Result<Run> {
    let dir = tempfile::tempdir()?;
    let paths = write(dir.path(), files, Some(GONE))?;
    let output = command(&paths).output()?;
    Ok((
        named(&paths, &output.stdout),
        named(&paths, &output.stderr),
        output.status.code().unwrap_or(-1),
    ))
}

/// A child that is killed and waited for when the test ends early, so that a failed assertion leaves no run
/// blocked on a pipe behind it.
struct Reaped(std::process::Child);

impl std::ops::Deref for Reaped {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Reaped {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The run over every file, with `GONE` removed after the run has begun and before it reaches it.
fn removed_mid_run(files: &[Vec<u8>]) -> Result<Run> {
    let dir = tempfile::tempdir()?;
    let paths = write(dir.path(), files, None)?;
    let mut child = Reaped(
        command(&paths)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
    );
    let mut stdout = BufReader::new(child.stdout.take().expect("piped"));
    let mut first = Vec::new();
    stdout.read_until(b'\n', &mut first)?;
    assert!(
        !first.is_empty(),
        "the run printed nothing before the file was removed"
    );
    // The run is blocked on a full pipe in the files before this one.
    std::fs::remove_file(&paths[GONE])?;
    let mut rest = Vec::new();
    stdout.read_to_end(&mut rest)?;
    first.extend(rest);
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .expect("piped")
        .read_to_end(&mut stderr)?;
    let status = child.wait()?;
    Ok((
        named(&paths, &first),
        named(&paths, &stderr),
        status.code().unwrap_or(-1),
    ))
}

/// A run whose file `GONE` goes missing mid-run gives what the same files give without it, and what jq gives:
/// every value of the files before it, the failure, the first value after it, and exit 2.
fn pinned(name: &str, unframed: &[usize]) -> Result<()> {
    let files = files(unframed);
    let (out, err, code) = removed_mid_run(&files)?;
    assert_eq!(
        (&err[..], code),
        (
            "jq: error: Could not open file <file 30>: No such file or directory\n",
            2
        ),
        "{name}"
    );
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines.len(),
        GONE * EACH + 1,
        "{name}: one value past the gap"
    );
    assert_eq!(lines[0], "\u{1e}[\"<file 0>\",0]", "{name}");
    assert_eq!(
        lines[GONE * EACH - 1],
        "\u{1e}[\"<file 29>\",29999]",
        "{name}: the last value before the file that is gone"
    );
    assert_eq!(
        lines[GONE * EACH],
        "\u{1e}[\"<file 31>\",31000]",
        "{name}: the first value after it, which jq's reader fetched when it met the failure"
    );
    assert_eq!((out, err, code), never_there(&files)?, "{name}");
    Ok(())
}

/// (a) The file after the one that is gone is the one that is not plain records: the windows stop at it, and it
/// is read whole after the failure, where no value stands between them.
#[test]
fn the_file_after_one_that_did_not_open_is_the_one_handed_on_7253() -> Result<()> {
    pinned("hand-off file follows", &[GONE + 1])
}

/// (b) Plain files stand between the one that is gone and the one that is not plain: the windows meet the
/// failure themselves, so the run stops after the first value past it, and never reaches the hand-off.
#[test]
fn plain_files_between_the_failure_and_the_handed_on_file_7253() -> Result<()> {
    pinned("plain files between", &[GONE + 3])
}
