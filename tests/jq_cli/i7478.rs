//! #7478: a lone input named as a pipe (a FIFO path or `<(cmd)`, `-`, `/dev/stdin` on a pipe) is read the
//! way standard input is read: by a program that reads `input`/`inputs` its front, then a window at a
//! time as the program asks, and by the plain filter and `-R` a window at a time, so `first(inputs)`,
//! `-n input`, `limit` and `halt` over a producer that never ends its stream answer at once, as jq 1.7.1
//! does. The route that reads a file whole read such a source to
//! its end before the program started, which over an endless producer never came. Every row was
//! captured from jq 1.7.1 over the same bytes and spellings (jq 1.8.2 answers the same, but for the value
//! preview in three caught messages, jqlang/jq#3478, where 1.7.1 stays the reference).
//!
//! The held rows' producer writes its records and then holds the pipe open without ending it: a run that
//! reads past what it needs waits for more until the deadline, and fails there instead of hanging.

#![cfg(unix)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What the producer writes.
#[derive(Clone, Copy)]
enum Data {
    /// 60,000 records (2.5 MB, past the lazy route's front and several windows), and then the pipe is
    /// held open.
    Records,
    /// Three documents, and the end of the stream.
    Short,
    /// Three documents with a fault between, and the end of the stream.
    Fault,
}

/// How the run names the pipe.
#[derive(Clone, Copy)]
enum How {
    /// A FIFO made with `mkfifo`, named by its path.
    Fifo,
    /// A pipe on standard input, named `-`.
    Dash,
    /// A pipe on standard input, named `/dev/stdin`.
    DevStdin,
}

fn bytes(data: Data) -> Vec<u8> {
    match data {
        Data::Records => (0..60_000)
            .flat_map(|i| format!("{{\"id\":{i},\"s\":\"{}\"}}\n", "x".repeat(i % 30)).into_bytes())
            .collect(),
        Data::Short => b"{\"id\":1}\n{\"id\":2}\n3\n".to_vec(),
        Data::Fault => b"{\"id\":1}\n}\n{\"id\":3}\n".to_vec(),
    }
}

/// `(data, how, args, (stdout, stderr, exit status))`, the FIFO's path written `<f>`.
type Row = (Data, How, &'static [&'static str], Outcome);

/// stdout, stderr and exit status.
type Outcome = (&'static str, &'static str, i32);

const ROWS: &[Row] = &[
    (Data::Records, How::Fifo, &["-nc", "first(inputs)"], ("{\"id\":0,\"s\":\"\"}\n", "", 0)),
    (Data::Records, How::Fifo, &["-nc", "input"], ("{\"id\":0,\"s\":\"\"}\n", "", 0)),
    (Data::Records, How::Fifo, &["-nc", "[limit(3; inputs)]"], ("[{\"id\":0,\"s\":\"\"},{\"id\":1,\"s\":\"x\"},{\"id\":2,\"s\":\"xx\"}]\n", "", 0)),
    (Data::Records, How::Fifo, &["-nc", "input, input_filename, input_line_number"], ("{\"id\":0,\"s\":\"\"}\n\"<f>\"\n1\n", "", 0)),
    (Data::Records, How::Fifo, &["-nc", "first(inputs | select(.id == 5000)) | [., input_line_number]"], ("[{\"id\":5000,\"s\":\"xxxxxxxxxxxxxxxxxxxx\"},5001]\n", "", 0)),
    (Data::Records, How::Fifo, &["-c", "halt"], ("", "", 0)),
    (Data::Records, How::Fifo, &["-c", "if .id == 2 then halt else [., input_filename] end"], ("[{\"id\":0,\"s\":\"\"},\"<f>\"]\n[{\"id\":1,\"s\":\"x\"},\"<f>\"]\n", "", 0)),
    (Data::Records, How::Fifo, &["-R", "-nc", "input"], ("\"{\\\"id\\\":0,\\\"s\\\":\\\"\\\"}\"\n", "", 0)),
    (Data::Records, How::Fifo, &["-R", "-nc", "[limit(2; inputs)], input_filename"], ("[\"{\\\"id\\\":0,\\\"s\\\":\\\"\\\"}\",\"{\\\"id\\\":1,\\\"s\\\":\\\"x\\\"}\"]\n\"<f>\"\n", "", 0)),
    (Data::Records, How::Fifo, &["-R", "-c", "halt"], ("", "", 0)),
    (Data::Short, How::Fifo, &["-nc", "[inputs] | length"], ("3\n", "", 0)),
    (Data::Short, How::Fifo, &["-nc", "[inputs | input_filename] | unique"], ("[\"<f>\"]\n", "", 0)),
    (Data::Short, How::Fifo, &["-nc", "try (inputs | .id) catch ."], ("1\n2\n\"Cannot index number with string \\\"id\\\"\"\n", "", 0)),
    (Data::Short, How::Fifo, &["-nc", "reduce inputs as $x (0; . + 1), input_line_number"], ("3\n3\n", "", 0)),
    (Data::Short, How::Fifo, &["-c", "[., input_filename, input_line_number]"], ("[{\"id\":1},\"<f>\",1]\n[{\"id\":2},\"<f>\",2]\n[3,\"<f>\",3]\n", "", 0)),
    (Data::Short, How::Fifo, &["-R", "-c", "[., input_filename]"], ("[\"{\\\"id\\\":1}\",\"<f>\"]\n[\"{\\\"id\\\":2}\",\"<f>\"]\n[\"3\",\"<f>\"]\n", "", 0)),
    (Data::Short, How::Fifo, &["-c", "."], ("{\"id\":1}\n{\"id\":2}\n3\n", "", 0)),
    (Data::Short, How::Fifo, &["-R", "-c", "."], ("\"{\\\"id\\\":1}\"\n\"{\\\"id\\\":2}\"\n\"3\"\n", "", 0)),
    (Data::Fault, How::Fifo, &["-nc", "[inputs] | length"], ("", "jq: error (at <f>:2): Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::Fifo, &["-nc", "[inputs | input_filename] | unique"], ("", "jq: error (at <f>:2): Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::Fifo, &["-nc", "try (inputs | .id) catch ."], ("1\n\"Unmatched '}' at line 2, column 1\"\n", "", 0)),
    (Data::Fault, How::Fifo, &["-nc", "reduce inputs as $x (0; . + 1), input_line_number"], ("", "jq: error (at <f>:2): Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::Fifo, &["-c", "[., input_filename, input_line_number]"], ("[{\"id\":1},\"<f>\",1]\n", "jq: parse error: Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::Fifo, &["-R", "-c", "[., input_filename]"], ("[\"{\\\"id\\\":1}\",\"<f>\"]\n[\"}\",\"<f>\"]\n[\"{\\\"id\\\":3}\",\"<f>\"]\n", "", 0)),
    (Data::Fault, How::Fifo, &["-c", "."], ("{\"id\":1}\n", "jq: parse error: Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::Fifo, &["-R", "-c", "."], ("\"{\\\"id\\\":1}\"\n\"}\"\n\"{\\\"id\\\":3}\"\n", "", 0)),
    (Data::Records, How::Dash, &["-nc", "first(inputs)"], ("{\"id\":0,\"s\":\"\"}\n", "", 0)),
    (Data::Records, How::Dash, &["-nc", "input"], ("{\"id\":0,\"s\":\"\"}\n", "", 0)),
    (Data::Records, How::Dash, &["-nc", "[limit(3; inputs)]"], ("[{\"id\":0,\"s\":\"\"},{\"id\":1,\"s\":\"x\"},{\"id\":2,\"s\":\"xx\"}]\n", "", 0)),
    (Data::Records, How::Dash, &["-nc", "input, input_filename, input_line_number"], ("{\"id\":0,\"s\":\"\"}\n\"<stdin>\"\n1\n", "", 0)),
    (Data::Records, How::Dash, &["-nc", "first(inputs | select(.id == 5000)) | [., input_line_number]"], ("[{\"id\":5000,\"s\":\"xxxxxxxxxxxxxxxxxxxx\"},5001]\n", "", 0)),
    (Data::Records, How::Dash, &["-c", "halt"], ("", "", 0)),
    (Data::Records, How::Dash, &["-c", "if .id == 2 then halt else [., input_filename] end"], ("[{\"id\":0,\"s\":\"\"},\"<stdin>\"]\n[{\"id\":1,\"s\":\"x\"},\"<stdin>\"]\n", "", 0)),
    (Data::Records, How::Dash, &["-R", "-nc", "input"], ("\"{\\\"id\\\":0,\\\"s\\\":\\\"\\\"}\"\n", "", 0)),
    (Data::Records, How::Dash, &["-R", "-nc", "[limit(2; inputs)], input_filename"], ("[\"{\\\"id\\\":0,\\\"s\\\":\\\"\\\"}\",\"{\\\"id\\\":1,\\\"s\\\":\\\"x\\\"}\"]\n\"<stdin>\"\n", "", 0)),
    (Data::Records, How::Dash, &["-R", "-c", "halt"], ("", "", 0)),
    (Data::Short, How::Dash, &["-nc", "[inputs] | length"], ("3\n", "", 0)),
    (Data::Short, How::Dash, &["-nc", "[inputs | input_filename] | unique"], ("[\"<stdin>\"]\n", "", 0)),
    (Data::Short, How::Dash, &["-nc", "try (inputs | .id) catch ."], ("1\n2\n\"Cannot index number with string \\\"id\\\"\"\n", "", 0)),
    (Data::Short, How::Dash, &["-nc", "reduce inputs as $x (0; . + 1), input_line_number"], ("3\n3\n", "", 0)),
    (Data::Short, How::Dash, &["-c", "[., input_filename, input_line_number]"], ("[{\"id\":1},\"<stdin>\",1]\n[{\"id\":2},\"<stdin>\",2]\n[3,\"<stdin>\",3]\n", "", 0)),
    (Data::Short, How::Dash, &["-R", "-c", "[., input_filename]"], ("[\"{\\\"id\\\":1}\",\"<stdin>\"]\n[\"{\\\"id\\\":2}\",\"<stdin>\"]\n[\"3\",\"<stdin>\"]\n", "", 0)),
    (Data::Short, How::Dash, &["-c", "."], ("{\"id\":1}\n{\"id\":2}\n3\n", "", 0)),
    (Data::Short, How::Dash, &["-R", "-c", "."], ("\"{\\\"id\\\":1}\"\n\"{\\\"id\\\":2}\"\n\"3\"\n", "", 0)),
    (Data::Fault, How::Dash, &["-nc", "[inputs] | length"], ("", "jq: error (at <stdin>:2): Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::Dash, &["-nc", "[inputs | input_filename] | unique"], ("", "jq: error (at <stdin>:2): Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::Dash, &["-nc", "try (inputs | .id) catch ."], ("1\n\"Unmatched '}' at line 2, column 1\"\n", "", 0)),
    (Data::Fault, How::Dash, &["-nc", "reduce inputs as $x (0; . + 1), input_line_number"], ("", "jq: error (at <stdin>:2): Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::Dash, &["-c", "[., input_filename, input_line_number]"], ("[{\"id\":1},\"<stdin>\",1]\n", "jq: parse error: Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::Dash, &["-R", "-c", "[., input_filename]"], ("[\"{\\\"id\\\":1}\",\"<stdin>\"]\n[\"}\",\"<stdin>\"]\n[\"{\\\"id\\\":3}\",\"<stdin>\"]\n", "", 0)),
    (Data::Fault, How::Dash, &["-c", "."], ("{\"id\":1}\n", "jq: parse error: Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::Dash, &["-R", "-c", "."], ("\"{\\\"id\\\":1}\"\n\"}\"\n\"{\\\"id\\\":3}\"\n", "", 0)),
    (Data::Records, How::DevStdin, &["-nc", "first(inputs)"], ("{\"id\":0,\"s\":\"\"}\n", "", 0)),
    (Data::Records, How::DevStdin, &["-nc", "input"], ("{\"id\":0,\"s\":\"\"}\n", "", 0)),
    (Data::Records, How::DevStdin, &["-nc", "[limit(3; inputs)]"], ("[{\"id\":0,\"s\":\"\"},{\"id\":1,\"s\":\"x\"},{\"id\":2,\"s\":\"xx\"}]\n", "", 0)),
    (Data::Records, How::DevStdin, &["-nc", "input, input_filename, input_line_number"], ("{\"id\":0,\"s\":\"\"}\n\"/dev/stdin\"\n1\n", "", 0)),
    (Data::Records, How::DevStdin, &["-nc", "first(inputs | select(.id == 5000)) | [., input_line_number]"], ("[{\"id\":5000,\"s\":\"xxxxxxxxxxxxxxxxxxxx\"},5001]\n", "", 0)),
    (Data::Records, How::DevStdin, &["-c", "halt"], ("", "", 0)),
    (Data::Records, How::DevStdin, &["-c", "if .id == 2 then halt else [., input_filename] end"], ("[{\"id\":0,\"s\":\"\"},\"/dev/stdin\"]\n[{\"id\":1,\"s\":\"x\"},\"/dev/stdin\"]\n", "", 0)),
    (Data::Records, How::DevStdin, &["-R", "-nc", "input"], ("\"{\\\"id\\\":0,\\\"s\\\":\\\"\\\"}\"\n", "", 0)),
    (Data::Records, How::DevStdin, &["-R", "-nc", "[limit(2; inputs)], input_filename"], ("[\"{\\\"id\\\":0,\\\"s\\\":\\\"\\\"}\",\"{\\\"id\\\":1,\\\"s\\\":\\\"x\\\"}\"]\n\"/dev/stdin\"\n", "", 0)),
    (Data::Records, How::DevStdin, &["-R", "-c", "halt"], ("", "", 0)),
    (Data::Short, How::DevStdin, &["-nc", "[inputs] | length"], ("3\n", "", 0)),
    (Data::Short, How::DevStdin, &["-nc", "[inputs | input_filename] | unique"], ("[\"/dev/stdin\"]\n", "", 0)),
    (Data::Short, How::DevStdin, &["-nc", "try (inputs | .id) catch ."], ("1\n2\n\"Cannot index number with string \\\"id\\\"\"\n", "", 0)),
    (Data::Short, How::DevStdin, &["-nc", "reduce inputs as $x (0; . + 1), input_line_number"], ("3\n3\n", "", 0)),
    (Data::Short, How::DevStdin, &["-c", "[., input_filename, input_line_number]"], ("[{\"id\":1},\"/dev/stdin\",1]\n[{\"id\":2},\"/dev/stdin\",2]\n[3,\"/dev/stdin\",3]\n", "", 0)),
    (Data::Short, How::DevStdin, &["-R", "-c", "[., input_filename]"], ("[\"{\\\"id\\\":1}\",\"/dev/stdin\"]\n[\"{\\\"id\\\":2}\",\"/dev/stdin\"]\n[\"3\",\"/dev/stdin\"]\n", "", 0)),
    (Data::Short, How::DevStdin, &["-c", "."], ("{\"id\":1}\n{\"id\":2}\n3\n", "", 0)),
    (Data::Short, How::DevStdin, &["-R", "-c", "."], ("\"{\\\"id\\\":1}\"\n\"{\\\"id\\\":2}\"\n\"3\"\n", "", 0)),
    (Data::Fault, How::DevStdin, &["-nc", "[inputs] | length"], ("", "jq: error (at /dev/stdin:2): Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::DevStdin, &["-nc", "[inputs | input_filename] | unique"], ("", "jq: error (at /dev/stdin:2): Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::DevStdin, &["-nc", "try (inputs | .id) catch ."], ("1\n\"Unmatched '}' at line 2, column 1\"\n", "", 0)),
    (Data::Fault, How::DevStdin, &["-nc", "reduce inputs as $x (0; . + 1), input_line_number"], ("", "jq: error (at /dev/stdin:2): Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::DevStdin, &["-c", "[., input_filename, input_line_number]"], ("[{\"id\":1},\"/dev/stdin\",1]\n", "jq: parse error: Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::DevStdin, &["-R", "-c", "[., input_filename]"], ("[\"{\\\"id\\\":1}\",\"/dev/stdin\"]\n[\"}\",\"/dev/stdin\"]\n[\"{\\\"id\\\":3}\",\"/dev/stdin\"]\n", "", 0)),
    (Data::Fault, How::DevStdin, &["-c", "."], ("{\"id\":1}\n", "jq: parse error: Unmatched '}' at line 2, column 1\n", 5)),
    (Data::Fault, How::DevStdin, &["-R", "-c", "."], ("\"{\\\"id\\\":1}\"\n\"}\"\n\"{\\\"id\\\":3}\"\n", "", 0)),
];

/// The run of `args` over `data` named as `how` says: stdout and stderr with the FIFO's path as `<f>`, and
/// the exit status, or `None` when it has not ended within the deadline (it is killed then).
fn run(data: Data, how: How, args: &[&str]) -> Option<(String, String, i32)> {
    let dir = tempfile::tempdir().expect("tempdir");
    let fifo = dir.path().join("ff");
    let held = matches!(data, Data::Records);
    let bytes = bytes(data);
    let (done, over) = std::sync::mpsc::channel::<()>();
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command.arg("jq").args(args).env("LC_ALL", "C");
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let producer;
    let mut child;
    match how {
        How::Fifo => {
            let made = Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .expect("mkfifo ran");
            assert!(made.success(), "mkfifo");
            command.arg(&fifo).stdin(Stdio::null());
            child = command.spawn().expect("spawned");
            let path = fifo.clone();
            producer = std::thread::spawn(move || {
                // Opening the write end waits for the run to open the read end.
                let mut pipe = std::fs::OpenOptions::new()
                    .write(true)
                    .open(path)
                    .expect("opened");
                let _ = pipe.write_all(&bytes);
                if held {
                    let _ = over.recv();
                }
            });
        }
        How::Dash | How::DevStdin => {
            command.arg(if matches!(how, How::Dash) {
                "-"
            } else {
                "/dev/stdin"
            });
            child = command.stdin(Stdio::piped()).spawn().expect("spawned");
            let mut stdin = child.stdin.take().expect("a pipe");
            producer = std::thread::spawn(move || {
                let _ = stdin.write_all(&bytes);
                if held {
                    let _ = over.recv();
                }
            });
        }
    }
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("waited") {
            break Some(status);
        }
        if started.elapsed() > Duration::from_secs(20) {
            child.kill().expect("killed");
            child.wait().expect("reaped");
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (mut out, mut err) = (String::new(), String::new());
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut out)
        .expect("read");
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut err)
        .expect("read");
    drop(done);
    if status.is_none() && matches!(how, How::Fifo) {
        // A producer still waiting for the run to open the FIFO is let go.
        let _ = std::fs::File::open(&fifo);
    }
    producer.join().expect("the producer ended");
    let name = fifo.to_string_lossy().into_owned();
    status.map(|status| {
        (
            out.replace(&name, "<f>"),
            err.replace(&name, "<f>"),
            status.code().unwrap_or(-1),
        )
    })
}

#[test]
fn a_pipe_named_on_the_command_line_is_read_as_standard_input_is_7478() {
    for &(data, how, args, expected) in ROWS {
        let seen = run(data, how, args);
        assert_eq!(
            seen.as_ref()
                .map(|(out, err, status)| (out.as_str(), err.as_str(), *status)),
            Some(expected),
            "{args:?} over {:?}",
            match how {
                How::Fifo => "a FIFO",
                How::Dash => "-",
                How::DevStdin => "/dev/stdin",
            }
        );
    }
}
