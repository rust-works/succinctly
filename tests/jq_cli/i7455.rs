//! #7455: a run that stops early (`-n input`, `first(inputs)`, `limit`, `halt`) leaves a regular
//! file on standard input where jq 1.7.1 leaves it: just past the `fgets` chunk (a line, or 4095
//! bytes of a longer one) that completed the last item it read, so the next process reading the same
//! descriptor starts there. It used to be left at the end of the file, or of the window the run had
//! read. Every offset below was captured from `/usr/bin/jq` 1.7.1 over the same bytes, read back
//! from the shared descriptor after the run.

#![cfg(unix)]

use std::io::{Seek, SeekFrom, Write};
use std::process::{Command, Stdio};

/// `n` lines `{"id":0}` .. `{"id":n-1}`.
fn records(n: usize) -> Vec<u8> {
    (0..n)
        .flat_map(|i| format!("{{\"id\":{i}}}\n").into_bytes())
        .collect()
}

/// A file holding `bytes`, at `offset`.
fn file_at(bytes: &[u8], offset: u64) -> std::fs::File {
    let mut file = tempfile::tempfile().expect("a temporary file");
    file.write_all(bytes).expect("written");
    file.seek(SeekFrom::Start(offset)).expect("seeked");
    file
}

/// `succinctly jq ARGS` reading `file` (the same open file, so the same offset) as its standard
/// input: stdout and exit status.
fn run(file: &std::fs::File, args: &[&str]) -> (String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .stdin(Stdio::from(file.try_clone().expect("a duplicate")))
        .output()
        .expect("ran");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

/// Where `args` leaves `bytes` on standard input, started at `offset`.
fn offset_after(bytes: &[u8], offset: u64, args: &[&str]) -> u64 {
    let mut file = file_at(bytes, offset);
    run(&file, args);
    file.stream_position().expect("the offset")
}

#[test]
fn a_second_process_reads_the_document_after_the_first_ones_7455() {
    let file = file_at(&records(3), 0);
    assert_eq!(
        run(&file, &["-c", "-n", "input"]),
        ("{\"id\":0}\n".into(), 0)
    );
    assert_eq!(
        run(&file, &["-c", "-n", "input"]),
        ("{\"id\":1}\n".into(), 0)
    );
    assert_eq!(run(&file, &["-c", "halt"]), (String::new(), 0));
    assert_eq!(run(&file, &["-c", "-n", "[inputs]"]), ("[]\n".into(), 0));
}

#[test]
fn an_early_stop_leaves_the_file_after_the_line_of_its_last_read_7455() {
    let three = records(3);
    for (args, want) in [
        (&["-c", "-n", "input"][..], 9),
        (&["-c", "-n", "first(inputs)"], 9),
        (&["-c", "-n", "limit(2; inputs)"], 18),
        (&["-c", "-n", "input | input"], 18),
        // Nothing read, nothing moved.
        (&["-c", "-n", "limit(0; inputs)"], 0),
        (&["-c", "-n", "1"], 0),
        (&["-c", "halt"], 9),
        (&["-c", "if .id == 1 then halt else . end"], 18),
        (&["-c", "\"x\\n\" | halt_error"], 9),
        (&["-R", "-n", "input"], 9),
        (&["-R", "halt"], 9),
        (&["--stream", "-c", "-n", "input"], 9),
        (
            &[
                "--stream",
                "-c",
                "-n",
                "first(inputs | select(length == 1))",
            ],
            9,
        ),
        // A run that reads to the end stays there.
        (&["-c", "."], 27),
        (&["-c", "-n", "[inputs]"], 27),
        (&["-c", "if .id == 1 then error(\"e\") else . end"], 27),
    ] {
        assert_eq!(offset_after(&three, 0, args), want, "{args:?}");
    }
    // From where the file stood when the run started.
    assert_eq!(offset_after(&three, 9, &["-c", "-n", "input"]), 18);
    assert_eq!(
        offset_after(&three, 9, &["-c", "-n", "limit(0; inputs)"]),
        9
    );
    // A last value with no line break after it completes at the end, where the read stands.
    assert_eq!(
        offset_after(b"1\n2", 0, &["-c", "if . == 2 then halt else . end"]),
        3
    );
    // A line of several values is read whole, and blank lines after a value's line are not.
    assert_eq!(
        offset_after(b"1 2 3\n4 5\n", 0, &["-c", "-n", "limit(2; inputs)"]),
        6
    );
    assert_eq!(
        offset_after(b"{\"id\":0}\n   \n\n", 0, &["-c", "-n", "input"]),
        9
    );
    // A `--stream` event inside a container is handed out at the `,` or closer after it.
    assert_eq!(
        offset_after(
            b"{\n  \"id\": 0\n}\n[1]\n",
            0,
            &["--stream", "-c", "-n", "input"]
        ),
        14
    );
}

#[test]
fn a_long_line_is_read_4095_bytes_at_a_time_7455() {
    // jq 1.7.1's `fgets` chunk; 1.8.2 reads 4091 bytes at a time, and leaves 4091 here.
    let mut long = b"{\"id\":0} ".repeat(2000);
    long.extend_from_slice(b"\n{\"id\":1}\n");
    assert_eq!(offset_after(&long, 0, &["-c", "-n", "input"]), 4095);
    assert_eq!(offset_after(&long, 0, &["-R", "-n", "input"]), 18_001);
    // A value longer than a chunk, alone on its line, still leaves the line's end.
    let mut big = format!("{{\"id\":0,\"p\":\"{}\"}}\n", "x".repeat(70_000)).into_bytes();
    big.extend_from_slice(&records(2));
    assert_eq!(offset_after(&big, 0, &["-c", "-n", "input"]), 70_016);
}

#[test]
fn the_windowed_routes_leave_the_file_where_jq_does_7455() {
    // 1.3 MB reads a front of 256 KiB, and 6.9 MB is read a window at a time.
    for count in [100_000, 500_000] {
        let bytes = records(count);
        for (args, want) in [
            (&["-c", "-n", "input"][..], 9),
            (&["-c", "-n", "limit(2; inputs)"], 18),
            (&["-c", "halt"], 9),
            (&["-c", "if .id == 1 then halt else . end"], 18),
            (&["-R", "halt"], 9),
            (&["--stream", "-c", "-n", "input"], 9),
        ] {
            assert_eq!(offset_after(&bytes, 0, args), want, "{count} {args:?}");
        }
        let file = file_at(&bytes, 0);
        assert_eq!(run(&file, &["-c", "-n", "input"]).0, "{\"id\":0}\n");
        assert_eq!(run(&file, &["-c", "-n", "input"]).0, "{\"id\":1}\n");
    }
}

#[test]
fn a_pipe_on_standard_input_stops_early_as_before_7455() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-c", "-n", "input"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawned");
    child
        .stdin
        .take()
        .expect("the pipe")
        .write_all(&records(3))
        .expect("written");
    let output = child.wait_with_output().expect("ran");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "{\"id\":0}\n");
    assert_eq!(output.status.code(), Some(0));
}
