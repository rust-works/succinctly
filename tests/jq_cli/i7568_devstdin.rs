//! #7568 follow-up: `/dev/stdin` and `/dev/fd/0` named as input files. On macOS the path opens as
//! descriptor 0 itself, sharing its offset, so a run that stops early leaves a regular file on
//! standard input where jq's stdio puts it back, as it does for `-`: at the end of the chunk that
//! completed the last item the program took. It was left at the end of the file. On Linux the path
//! opens the file again from its start, and descriptor 0 moves only when a `-` after it is read: the
//! restore read such a path again by its name only on macOS's terms and gave up on it, leaving the
//! file at its end. And `--stream` over such a path probed its first bytes through that shared offset
//! and then read the stream from past them (`Invalid numeric literal at line 1, column 2`).
//!
//! The macOS offsets were captured from `/usr/bin/jq` 1.7.1, reading the shared descriptor's offset
//! back after the run (jq 1.8.2 agrees on every row); standard input holds three lines `{"id":N}`,
//! and `F` is a file holding `{"a":1}` and a newline. No Linux machine was at hand: the Linux offsets
//! were captured from `/usr/bin/jq` 1.7.1 and 1.8.2 on macOS with the path replaced by the file's own
//! name, which opens it again from its start as Linux's `/proc/self/fd` links do.

#![cfg(unix)]

use std::io::{Seek, SeekFrom, Write};
use std::process::{Command, Stdio};

const STDIN: &[u8] = b"{\"id\":0}\n{\"id\":1}\n{\"id\":2}\n";

/// (offset standard input starts at, arguments with `D` for the path, offset jq leaves on macOS, offset
/// it leaves where the path opens the file again: the start, unless a `-` after it is read).
const ROWS: &[(u64, &[&str], u64, u64)] = &[
    (0, &["-c", "-n", "input", "D"], 9, 0),
    (0, &["-c", "-n", "first(inputs)", "D"], 9, 0),
    (0, &["-c", "-n", "[limit(2; inputs)]", "D"], 18, 0),
    (0, &["-c", "-n", "[limit(4; inputs)]", "D"], 27, 0),
    (0, &["-c", "halt", "D"], 9, 0),
    (
        0,
        &["-c", "--stream", "-n", "[limit(3; inputs)]", "D"],
        18,
        0,
    ),
    (0, &["-c", "-R", "-n", "input", "D"], 9, 0),
    (0, &["-c", "-R", "halt", "D"], 9, 0),
    (0, &["-c", "-n", "input", "F", "D"], 0, 0),
    (0, &["-c", "-n", "[limit(2; inputs)]", "F", "D"], 9, 0),
    (0, &["-c", "-n", "input", "D", "F"], 9, 0),
    (0, &["-c", "-n", "input", "D", "D"], 9, 0),
    (0, &["-c", "-n", "input", "D", "-"], 9, 0),
    (0, &["-c", ".", "D"], 27, 0),
    (9, &["-c", "-n", "input", "D"], 18, 9),
    (9, &["-c", "halt", "D"], 18, 9),
    (9, &["-c", "-n", "input", "F", "D"], 9, 9),
    (0, &["-c", "-n", "[limit(2; inputs)]", "D", "-"], 18, 0),
    (0, &["-c", "-n", "[limit(4; inputs)]", "D", "-"], 27, 9),
    (9, &["-c", "-n", "[limit(4; inputs)]", "D", "-"], 27, 18),
    (0, &["-c", "-n", "[limit(5; inputs)]", "F", "D", "-"], 27, 9),
];

/// Runs `succinctly jq args` with `D` replaced by `path`, over a temporary file holding [`STDIN`] on
/// standard input from `offset`; the offset it is left at, and the run's output and status.
fn run(offset: u64, path: &str, args: &[&str]) -> (u64, String, i32) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let named = dir.path().join("F");
    std::fs::write(&named, b"{\"a\":1}\n").expect("written");
    let mut file = tempfile::tempfile().expect("a temporary file");
    file.write_all(STDIN).expect("written");
    file.seek(SeekFrom::Start(offset)).expect("seeked");
    let out = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args.iter().map(|&arg| match arg {
            "F" => named.as_os_str(),
            "D" => path.as_ref(),
            _ => arg.as_ref(),
        }))
        .stdin(Stdio::from(file.try_clone().expect("a duplicate")))
        .stderr(Stdio::inherit())
        .output()
        .expect("ran");
    (
        file.stream_position().expect("the offset"),
        String::from_utf8(out.stdout).expect("UTF-8"),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn dev_stdin_named_as_an_input_file_leaves_stdin_where_jq_does_7568() {
    let mut failures = Vec::new();
    for path in ["/dev/stdin", "/dev/fd/0"] {
        for &(start, args, mac, anew) in ROWS {
            let want = if cfg!(target_os = "macos") { mac } else { anew };
            let (at, _, status) = run(start, path, args);
            if (at, status) != (want, 0) {
                failures.push(format!(
                    "{path} from {start} {args:?}: at {at}, exit {status} (want {want}, exit 0)"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `--stream` reads the stream from where the path's offset stands, not from past the bytes its probe
/// read through it.
#[test]
fn stream_reads_dev_stdin_from_its_offset_7568() {
    for path in ["/dev/stdin", "/dev/fd/0"] {
        let (_, out, status) = run(0, path, &["-c", "--stream", ".", "D"]);
        assert_eq!(
            (out.as_str(), status),
            (
                "[[\"id\"],0]\n[[\"id\"]]\n[[\"id\"],1]\n[[\"id\"]]\n[[\"id\"],2]\n[[\"id\"]]\n",
                0
            ),
            "{path}"
        );
        let (_, out, status) = run(9, path, &["-c", "--stream", "-n", "input", "D"]);
        let want = if cfg!(target_os = "macos") {
            "[[\"id\"],1]\n"
        } else {
            "[[\"id\"],0]\n"
        };
        assert_eq!((out.as_str(), status), (want, 0), "{path}");
        // The same through a shell's redirection of a named file, as a user runs it.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let f = dir.path().join("f");
        std::fs::write(&f, STDIN).expect("written");
        let out = Command::new("sh")
            .arg("-c")
            .arg(r#"exec "$0" jq -c --stream . "$1" < "$2""#)
            .arg(env!("CARGO_BIN_EXE_succinctly"))
            .arg(path)
            .arg(&f)
            .output()
            .expect("ran");
        assert_eq!(
            (
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
                out.status.code()
            ),
            (
                "[[\"id\"],0]\n[[\"id\"]]\n[[\"id\"],1]\n[[\"id\"]]\n[[\"id\"],2]\n[[\"id\"]]\n"
                    .to_string(),
                String::new(),
                Some(0)
            ),
            "{path} through sh"
        );
    }
}
