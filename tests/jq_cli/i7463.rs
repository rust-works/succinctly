//! #7463: `fromstream(f)` and `truncate_stream(f)` hand each value on as the events that make it
//! arrive, as jq's `foreach`-defined builtins do, so `first(fromstream(inputs))` stops reading the
//! input at the first value and `input_line_number`, `input` and an error after a value read the input
//! where that value's last event left it. Both collected every event first: 2 GB for a 52 MB array,
//! no end on an endless pipe, and every line was the input's last. Every row was captured from
//! jq 1.7.1 on the same bytes; jq 1.8.2 answers the same, but for the value previews of four error
//! messages (jqlang/jq#3478, where 1.7.1 stays the reference).

#![cfg(unix)]

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::spawn_jq;

/// Values over several lines, then values one to a line.
const FS: &str = "[1,2]\n{\"a\":\n3}\n4\n5\n";
/// The events of `[1]`, `[2]` and `[3]` one to a line, each one level deeper than the value.
const EV: &str = "[[0,0],1]\n[[0,0]]\n[[1,0],2]\n[[1,0]]\n[[2,0],3]\n[[2,0]]\n";
/// An array of six objects over six lines: the "stream a big array" idiom's input.
const ARR: &str = "[{\"id\":0,\"t\":[\"a\",0]},\n{\"id\":1,\"t\":[\"a\",1]},\n{\"id\":2,\"t\":[\"a\",2]},\n{\"id\":3,\"t\":[\"a\",3]},\n{\"id\":4,\"t\":[\"a\",4]},\n{\"id\":5,\"t\":[\"a\",5]}]\n";

/// `(name, args, file text, (stdout, stderr with the file as `<f>`, exit status))`.
type Row = (&'static str, &'static [&'static str], &'static str, Outcome);

/// stdout, stderr with the file as `<f>`, and the exit status.
type Outcome = (&'static str, &'static str, i32);

const ROWS: &[Row] = &[
    (
        "values_with_lines",
        &["-n", "--stream", "-c", "fromstream(inputs) | [., input_line_number]"],
        FS,
        ("[[1,2],1]\n[{\"a\":3},3]\n[4,4]\n[5,5]\n", "", 0),
    ),
    (
        "first_then_line",
        &["-n", "--stream", "-c", "first(fromstream(inputs)), input_line_number"],
        FS,
        ("[1,2]\n1\n", "", 0),
    ),
    (
        "first_then_input",
        &["-n", "--stream", "-c", "first(fromstream(inputs)), input"],
        FS,
        ("[1,2]\n[[\"a\"],3]\n", "", 0),
    ),
    (
        "limit_then_inputs",
        &["-n", "--stream", "-c", "[limit(2; fromstream(inputs))], [inputs]"],
        FS,
        ("[[1,2],{\"a\":3}]\n[[[],4],[[],5]]\n", "", 0),
    ),
    (
        "error_location",
        &["-n", "--stream", "-c", "fromstream(inputs) | if . == 4 then error(\"E\") else . end"],
        FS,
        ("[1,2]\n{\"a\":3}\n", "jq: error (at <f>:4): E\n", 5),
    ),
    (
        "array_idiom_first",
        &["-n", "--stream", "-c", "first(fromstream(1|truncate_stream(inputs))), input_line_number"],
        ARR,
        ("{\"id\":0,\"t\":[\"a\",0]}\n1\n", "", 0),
    ),
    (
        "array_idiom_lines",
        &["-n", "--stream", "-c", "fromstream(1|truncate_stream(inputs)) | [.id, input_line_number]"],
        ARR,
        ("[0,1]\n[1,2]\n[2,3]\n[3,4]\n[4,5]\n[5,6]\n", "", 0),
    ),
    (
        "array_idiom_limit",
        &["-n", "--stream", "-c", "[limit(3; fromstream(1|truncate_stream(inputs)))], input"],
        ARR,
        ("[{\"id\":0,\"t\":[\"a\",0]},{\"id\":1,\"t\":[\"a\",1]},{\"id\":2,\"t\":[\"a\",2]}]\n[[3,\"id\"],3]\n", "", 0),
    ),
    (
        "truncate_first_then_input",
        &["-n", "--stream", "-c", "first(1|truncate_stream(inputs)), input"],
        ARR,
        ("[[\"id\"],0]\n[[0,\"t\",0],\"a\"]\n", "", 0),
    ),
    (
        "truncate_lines",
        &["-n", "--stream", "-c", "1|truncate_stream(inputs) | [., input_line_number]"],
        FS,
        ("", "", 0),
    ),
    (
        "ndjson_events",
        &["-n", "-c", "fromstream(1|truncate_stream(inputs)) | input_line_number"],
        EV,
        ("2\n4\n6\n", "", 0),
    ),
    (
        "ndjson_events_first",
        &["-n", "-c", "first(fromstream(1|truncate_stream(inputs))), input"],
        EV,
        ("[1]\n[[1,0],2]\n", "", 0),
    ),
    (
        "stream_errors_fault",
        &["-n", "--stream-errors", "-c", "fromstream(inputs)"],
        "1\n12x\n2\n2\n",
        ("1\n", "jq: error (at <f>:2): array ([\"x\"]) and string (\"Invalid nu...) cannot be added\n", 5),
    ),
    (
        "stream_errors_eof",
        &["-n", "--stream-errors", "-c", "fromstream(inputs)"],
        "[1,2",
        ("", "jq: error (at <f>:0): array ([\"x\"]) and string (\"Unfinished...) cannot be added\n", 5),
    ),
    (
        "try_then_rest",
        &["-n", "--stream", "-c", "try (fromstream(inputs) | if . == 4 then error(\"E\") else . end) catch ., [inputs]"],
        FS,
        ("[1,2]\n{\"a\":3}\n\"E\"\n[[[],5]]\n", "", 0),
    ),
    (
        "truncate_raises_late",
        &["-n", "-c", "1|truncate_stream([[0,1],2], 3)"],
        "",
        ("[[1],2]\n", "jq: error (at <unknown>): Cannot index number with number\n", 5),
    ),
    (
        "fromstream_raises_late",
        &["-n", "-c", "fromstream([[0],1], [[0]], \"x\")"],
        "",
        ("[1]\n", "jq: error (at <unknown>): Cannot index string with number\n", 5),
    ),
    (
        "main_loop_inputs",
        &["--stream", "-c", "[., (first(fromstream(inputs)))]"],
        FS,
        ("[[[0],1],[null,2]]\n[[[\"a\"],3],null]\n[[[],4],5]\n", "", 0),
    ),
];

#[test]
fn fromstream_hands_each_value_on_as_its_events_arrive_7463() -> Result<()> {
    for &(name, args, text, expected) in ROWS {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("in.json");
        std::fs::write(&path, text)?;
        let file = path.to_string_lossy().into_owned();
        let mut argv = args.to_vec();
        argv.push(&file);
        let (out, status) = spawn_jq(&argv, None)?;
        let seen = (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).replace(&file, "<f>"),
            status,
        );
        assert_eq!(
            (seen.0.as_str(), seen.1.as_str(), seen.2),
            expected,
            "row {name}"
        );
    }
    Ok(())
}

/// `succinctly jq ARGS` with a producer of `line` on standard input that never ends its stream: it
/// writes 16 MiB of lines and then holds the pipe open until the run is over, so a run that reads past
/// what it needs waits for more input until `limit`, and is killed then (`None`), with its memory
/// bounded by those 16 MiB. Otherwise its stdout and exit status. jq 1.7.1 ends at once.
fn over_an_endless_pipe(args: &[&str], line: &[u8], limit: Duration) -> Option<(String, i32)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawned");
    let mut stdin = child.stdin.take().expect("a pipe");
    let block = line.repeat(4096);
    let (done, over) = std::sync::mpsc::channel::<()>();
    let producer = std::thread::spawn(move || {
        let mut written = 0;
        // A broken pipe (the run is over) ends the writes.
        while written < 16 << 20 && stdin.write_all(&block).is_ok() {
            written += block.len();
        }
        // The stream never ends while the run goes on.
        let _ = over.recv();
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("waited") {
            break Some(status);
        }
        if started.elapsed() > limit {
            child.kill().expect("killed");
            child.wait().expect("reaped");
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        std::io::Read::read_to_string(&mut stdout, &mut out).expect("read");
    }
    drop(done);
    producer.join().expect("the producer ended");
    status.map(|status| (out, status.code().unwrap_or(-1)))
}

#[test]
fn first_of_fromstream_ends_on_an_endless_pipe_7463() {
    let limit = Duration::from_secs(20);
    for (args, line, want) in [
        (
            &["-n", "--stream", "-c", "first(fromstream(inputs))"][..],
            &b"[1,2]\n"[..],
            "[1,2]\n",
        ),
        (
            &[
                "-n",
                "--stream",
                "-c",
                "first(fromstream(1|truncate_stream(inputs)))",
            ],
            b"[{\"id\":1},{\"id\":2}]\n",
            "{\"id\":1}\n",
        ),
        (
            &["-n", "--stream", "-c", "[limit(3; fromstream(inputs))]"],
            b"7\n",
            "[7,7,7]\n",
        ),
    ] {
        assert_eq!(
            over_an_endless_pipe(args, line, limit),
            Some((want.to_string(), 0)),
            "{args:?}"
        );
    }
}
