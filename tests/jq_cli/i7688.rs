//! #7688: `scripts/jq-windowed-file-fuzz.py` compares a windowed route with a whole-file oracle, and
//! two of its oracle spellings (`/dev/fd/N`, a named pipe) had become windowed themselves, so those
//! modes compared a route with itself. `SUCCINCTLY_DEBUG_JQ_WHOLE_FILE` is the oracle that stays
//! whole-file whatever the gates do, and `SUCCINCTLY_DEBUG_JQ_ROUTE_LOG` names the windowed routes a
//! run took, so the script can tell that its two sides differ in route. These rows pin both, over
//! every source the script's modes read: a named file, several, a descriptor, standard input as a
//! file and as a pipe, and a named pipe.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;

use crate::small_window;

const WHOLE: &str = "SUCCINCTLY_DEBUG_JQ_WHOLE_FILE";
const LOG: &str = "SUCCINCTLY_DEBUG_JQ_ROUTE_LOG";

/// What a run answered, and the routes it logged.
#[derive(Debug, PartialEq)]
struct Run {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    status: i32,
    routes: Vec<String>,
}

/// `n` lines of `{"id":N}`, or of `RS {"id":N} LF` records when `seq`, at least `bytes` long.
fn stream(bytes: usize, seq: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes + 64);
    let mut id = 0;
    while out.len() < bytes {
        if seq {
            out.push(0x1e);
        }
        out.extend_from_slice(format!("{{\"id\":{id}}}\n").as_bytes());
        id += 1;
    }
    out
}

fn command(dir: &Path, args: &[&str], whole: bool, log: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command
        .arg("jq")
        .args(args)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .env_remove(WHOLE)
        .env(LOG, log);
    small_window::apply(&mut command);
    if whole {
        command.env(WHOLE, "1");
    }
    command
}

fn finish(output: std::process::Output, log: &Path) -> Run {
    let routes = std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_file(log);
    Run {
        stdout: output.stdout,
        stderr: output.stderr,
        status: output.status.code().unwrap_or(-1),
        routes,
    }
}

/// How a source is handed to the run.
#[derive(Clone, Copy)]
enum Source {
    /// The paths on the command line.
    Paths,
    /// `/dev/fd/N` for a file held open (the fuzz's `--files 1` oracle).
    Descriptor,
    /// The one file as standard input.
    StdinFile,
    /// The one file's bytes on a pipe on standard input.
    StdinPipe,
}

fn run(dir: &Path, flags: &[&str], files: &[PathBuf], how: Source, whole: bool) -> Result<Run> {
    let log = dir.join(format!("routes-{whole}.log"));
    let _ = std::fs::remove_file(&log);
    let output = match how {
        Source::Paths => {
            let mut args: Vec<String> = flags.iter().map(|f| (*f).to_string()).collect();
            args.extend(files.iter().map(|f| f.display().to_string()));
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            command(dir, &args, whole, &log).output()?
        }
        Source::Descriptor => {
            // The shell opens descriptor 7 on the file, which the binary then names as `/dev/fd/7`.
            let mut cmd = Command::new("sh");
            cmd.arg("-c")
                .arg("exec \"$0\" \"$@\" 7< \"$IN\"")
                .arg(env!("CARGO_BIN_EXE_succinctly"))
                .arg("jq")
                .args(flags)
                .arg("/dev/fd/7")
                .current_dir(dir)
                .env("IN", &files[0])
                .env("LC_ALL", "C")
                .env_remove(WHOLE)
                .env(LOG, &log);
            small_window::apply(&mut cmd);
            if whole {
                cmd.env(WHOLE, "1");
            }
            cmd.output()?
        }
        Source::StdinFile => {
            let mut cmd = command(dir, flags, whole, &log);
            cmd.stdin(std::fs::File::open(&files[0])?);
            cmd.output()?
        }
        Source::StdinPipe => {
            let bytes = std::fs::read(&files[0])?;
            let mut cmd = command(dir, flags, whole, &log);
            crate::fifo_feed::output_stdin(&mut cmd, &bytes)?
        }
    };
    Ok(finish(output, &log))
}

/// The route a run over `how` takes by default, and that the switch turns into the whole-file one with
/// the same answer.
fn check(flags: &[&str], files: usize, seq: bool, how: Source, route: &str) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let paths: Vec<PathBuf> = (0..files)
        .map(|n| dir.path().join(format!("in{n}.json")))
        .collect();
    for path in &paths {
        std::fs::write(path, stream(small_window::MIN_BYTES * 2, seq))?;
    }
    let windowed = run(dir.path(), flags, &paths, how, false)?;
    let whole = run(dir.path(), flags, &paths, how, true)?;
    assert_eq!(windowed.routes, [route], "{flags:?} not windowed");
    assert_eq!(whole.routes, Vec::<String>::new(), "{flags:?} forced whole");
    assert_eq!(windowed.stdout, whole.stdout, "{flags:?} stdout");
    assert_eq!(windowed.stderr, whole.stderr, "{flags:?} stderr");
    assert_eq!(windowed.status, whole.status, "{flags:?} status");
    assert!(!windowed.stdout.is_empty(), "{flags:?} answered nothing");
    Ok(())
}

#[test]
fn a_named_file_is_windowed_and_the_switch_reads_it_whole_7688() -> Result<()> {
    check(&["-c", ".id"], 1, false, Source::Paths, "plain")
}

#[test]
fn a_descriptor_is_windowed_and_the_switch_reads_it_whole_7688() -> Result<()> {
    check(&["-c", ".id"], 1, false, Source::Descriptor, "plain")
}

#[test]
fn standard_input_as_a_file_is_windowed_and_the_switch_reads_it_whole_7688() -> Result<()> {
    check(&["-c", ".id"], 1, false, Source::StdinFile, "plain")
}

#[test]
fn a_pipe_is_windowed_and_the_switch_reads_it_whole_7688() -> Result<()> {
    check(&["-c", ".id"], 1, false, Source::StdinPipe, "plain")
}

#[test]
fn several_files_are_windowed_and_the_switch_reads_them_whole_7688() -> Result<()> {
    check(&["-c", ".id"], 2, false, Source::Paths, "plain")
}

#[test]
fn raw_lines_are_windowed_and_the_switch_reads_them_whole_7688() -> Result<()> {
    check(&["-R", "length"], 1, false, Source::Paths, "raw")?;
    check(&["-R", "length"], 1, false, Source::StdinPipe, "raw")
}

#[test]
fn inputs_over_several_files_are_windowed_and_the_switch_reads_them_whole_7688() -> Result<()> {
    check(
        &["-n", "[inputs] | length"],
        2,
        false,
        Source::Paths,
        "inputs",
    )
}

#[test]
fn a_seq_stream_is_windowed_and_the_switch_reads_it_whole_7688() -> Result<()> {
    check(&["--seq", "-c", ".id"], 1, true, Source::Paths, "seq")?;
    check(&["--seq", "-c", ".id"], 1, true, Source::StdinPipe, "seq")
}

/// `0` and the empty string leave the route alone: the switch is a non-empty value but `0`.
#[test]
fn zero_and_empty_leave_the_route_windowed_7688() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("in.json");
    std::fs::write(&file, stream(small_window::MIN_BYTES * 2, false))?;
    for value in ["0", ""] {
        let log = dir.path().join("routes.log");
        let _ = std::fs::remove_file(&log);
        let mut cmd = command(
            dir.path(),
            &["-c", ".id", file.to_str().unwrap()],
            false,
            &log,
        );
        cmd.env(WHOLE, value);
        let got = finish(cmd.output()?, &log);
        assert_eq!(got.routes, ["plain"], "{value:?}");
    }
    Ok(())
}
