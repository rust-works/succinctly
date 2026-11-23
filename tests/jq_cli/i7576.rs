//! #7576: an input `succinctly jq` reads whole that there is no room for (a `ulimit -v` smaller than
//! the file) was reported as a file that would not open (`jq: error: Could not open file f: out of
//! memory`, exit 2), and the run went on as if it were empty: `-s length` printed `0`, `-c -s .`
//! printed `[]`. On standard input the read stopped where the room did (`-R -s length` printed the
//! length it got) or a later copy aborted with Rust's `memory allocation of ... failed`. jq 1.7.1
//! answers every one of these routes with `jq: error: cannot allocate memory` and `abort()` (exit
//! 134 to a shell), captured on x86_64 Linux under `ulimit -v 400000` over a 300 MiB NDJSON file
//! (jq 1.8.2's `memory_exhausted` is the same, src/jv_alloc.c; not run, no 1.8.2 on Linux).
//!
//! The file is sparse: an allocation that fails needs no bytes of it, and jq's answer did not
//! depend on them (every route below fails at the reservation, before a byte is parsed).

#![cfg(unix)]

use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::no_core_dump::without_core_dump;

/// jq 1.7.1's whole stderr for an allocation that fails.
const CANNOT_ALLOCATE: &str = "jq: error: cannot allocate memory\n";

/// The signal `abort()` raises.
const SIGABRT: i32 = 6;

/// A sparse file of `len` bytes in `dir`, or `None` where the file system refuses that length.
fn sparse_file(dir: &Path, len: u64) -> Option<PathBuf> {
    let path = dir.join("big.json");
    let file = std::fs::File::create(&path).expect("created");
    file.set_len(len).ok()?;
    Some(path)
}

/// Run `succinctly jq <args>` with `stdin`, under `ulimit -v <kib>` when one is given and with core
/// dumps off (`RLIMIT_CORE` of 1 byte, which a piped `core_pattern` honours where it does not honour
/// `ulimit -c 0`, #7869; the wrapper must not set `-c` itself): `(stdout, stderr, terminating signal)`.
fn run(kib: Option<usize>, args: &[&str], stdin: Stdio) -> (String, String, Option<i32>) {
    let cap = kib.map_or(String::new(), |kib| format!("ulimit -v {kib} && "));
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(format!("{cap}exec \"$0\" jq \"$@\""))
        .arg(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = without_core_dump(&mut command).output().expect("ran");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.signal(),
    )
}

/// The routes that read a named file whole, each as jq 1.7.1 was run on it (`F` is the file). The
/// `--seq` rows are the recorded divergence: jq streams a `--seq` file (`Unfinished abandoned text at
/// EOF at line 8530001, column 0`, exit 0), where `succinctly jq` reads a stream with no RS whole.
const FILE_ROUTES: &[&[&str]] = &[
    &["-s", "length", "F"],
    &["-c", "-s", ".", "F"],
    &["-s", "-n", "input|length", "F"],
    &["-n", "-s", "[inputs]|length", "F"],
    &["--stream", "-s", "length", "F"],
    &["-R", "-s", "length", "F"],
    &["-R", "-n", "-s", "input|length", "F"],
    &["-s", "length", "F", "F"],
    &["-n", "--slurpfile", "a", "F", "$a|length"],
    &["-n", "--rawfile", "a", "F", "$a|length"],
    &["-n", "-f", "F"],
    &["--seq", "-c", ".", "F"],
    &["--seq", "-s", "length", "F"],
];

fn with_file<'a>(args: &[&'a str], file: &'a str) -> Vec<&'a str> {
    args.iter()
        .map(|&arg| if arg == "F" { file } else { arg })
        .collect()
}

fn assert_cannot_allocate(label: &str, (stdout, stderr, signal): (String, String, Option<i32>)) {
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), signal),
        ("", CANNOT_ALLOCATE, Some(SIGABRT)),
        "{label}"
    );
}

/// Under the cap of the issue: every route that reads the file whole, standard input among them.
#[cfg(target_os = "linux")]
#[test]
fn an_input_read_whole_with_no_room_is_jqs_cannot_allocate_memory_7576() {
    const KIB: usize = 400_000;
    let dir = tempfile::tempdir().expect("tempdir");
    let big = sparse_file(dir.path(), 1 << 30).expect("a 1 GiB sparse file");
    let small = dir.path().join("small.json");
    std::fs::write(&small, "{\"a\":1}\n").expect("written");
    let (big, small) = (big.to_str().unwrap(), small.to_str().unwrap());
    for args in FILE_ROUTES {
        let argv = with_file(args, big);
        assert_cannot_allocate(&format!("{argv:?}"), run(Some(KIB), &argv, Stdio::null()));
    }
    // A file that fits before it, or after it, is no help: the run reads both before it runs.
    for argv in [["-s", "length", small, big], ["-s", "length", big, small]] {
        assert_cannot_allocate(&format!("{argv:?}"), run(Some(KIB), &argv, Stdio::null()));
    }
    // Standard input, redirected from the file and as `-`.
    for argv in [
        &["-s", "length"][..],
        &["-c", "-s", "."],
        &["-R", "-s", "length"],
        &["-s", "length", "-"],
        &["--seq", "-c", "."],
    ] {
        let stdin = Stdio::from(std::fs::File::open(big).expect("opened"));
        assert_cannot_allocate(&format!("{argv:?} < big"), run(Some(KIB), argv, stdin));
    }
}

/// The same answer with no cap at all, for a file longer than any allocation, on every Unix: 8 TiB
/// (ext4 refuses a file of 16 TiB or more), where the file system allows a sparse one and this
/// process cannot reserve that much either.
#[test]
fn a_file_longer_than_any_allocation_is_jqs_cannot_allocate_memory_7576() {
    let len = 8u64 << 40;
    if Vec::<u8>::new()
        .try_reserve_exact(usize::try_from(len).unwrap_or(usize::MAX))
        .is_ok()
    {
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(big) = sparse_file(dir.path(), len) else {
        return;
    };
    let big = big.to_str().unwrap();
    for args in FILE_ROUTES {
        let argv = with_file(args, big);
        assert_cannot_allocate(&format!("{argv:?}"), run(None, &argv, Stdio::null()));
    }
}

/// A file that fits is read as before under the same cap (the plain route reads it in windows, and
/// `-s` holds it whole).
#[cfg(target_os = "linux")]
#[test]
fn an_input_that_fits_the_cap_is_still_read_7576() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("fits.json");
    let lines: String = (0..1000).map(|n| format!("{{\"id\":{n}}}\n")).collect();
    std::fs::write(&path, lines).expect("written");
    let path = path.to_str().unwrap();
    let (stdout, stderr, signal) = run(Some(400_000), &["-s", "length", path], Stdio::null());
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), signal),
        ("1000\n", "", None)
    );
    let (stdout, stderr, signal) = run(
        Some(400_000),
        &["-n", "--rawfile", "a", path, "$a|length"],
        Stdio::null(),
    );
    assert_eq!(
        (stdout.as_str(), stderr.as_str(), signal),
        ("10890\n", "", None)
    );
}
