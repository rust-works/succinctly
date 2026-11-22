//! #7698: on Linux a lone regular file named by a path whose `stat` is the file standard input is open
//! on (`jq . /dev/stdin < f`, `/dev/fd/0`, `/proc/self/fd/0`) was read whole before the program ran,
//! where the same file as standard input or as `/dev/fd/7` is read a window at a time; so was a file
//! named by its own path with the same file on standard input (`jq . f < f`), on every platform. The gate
//! turned down every path whose file is standard input's, for fear the path shares descriptor 0's
//! offset. On Linux those paths are `/proc/self/fd` links that open the file again from its start, so
//! they are read as any named file is, and descriptor 0 is left where it stood, as jq leaves it. The
//! gate now asks whether a new open of the path shares the offset rather than whose file it is.
//!
//! Each row runs twice, once on the default routes and once with every input read whole
//! (`SUCCINCTLY_DEBUG_JQ_WHOLE_FILE`, the route the gate used to send these paths to), and the two must
//! agree on output, errors, exit status and the offset left on standard input; the route log
//! (`SUCCINCTLY_DEBUG_JQ_ROUTE_LOG`) says the default run was windowed. On macOS `/dev/stdin` and
//! `/dev/fd/0` share the offset, and their `stat` names the descriptor file system, so they were read a
//! window at a time from where the offset stands before this change too (#7446, #7568): those rows
//! assert that they still are and still answer as the whole-file route does. The fault's wording was
//! captured from jq 1.7.1 on Linux and on macOS (jq 1.8.2 agrees).

#![cfg(unix)]

use std::io::{Seek, SeekFrom};
use std::process::{Command, Stdio};

use anyhow::Result;

use crate::small_window;

const WHOLE: &str = "SUCCINCTLY_DEBUG_JQ_WHOLE_FILE";
const LOG: &str = "SUCCINCTLY_DEBUG_JQ_ROUTE_LOG";

/// What a run answered, the windowed routes it logged, and where it left standard input.
#[derive(Debug, PartialEq)]
struct Run {
    stdout: String,
    stderr: String,
    status: i32,
    routes: Vec<String>,
    offset: u64,
}

/// Lines of `{"id":N}` (records `RS {"id":N} LF` when `seq`) of at least twice the windowed routes'
/// smallest file, and a line jq's parser refuses at the end when `fault`.
fn text(seq: bool, fault: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut id = 0;
    while out.len() < 2 * small_window::MIN_BYTES {
        if seq {
            out.push(0x1e);
        }
        out.extend_from_slice(format!("{{\"id\":{id}}}\n").as_bytes());
        id += 1;
    }
    if fault {
        out.extend_from_slice(b"{\"bad\":}\n");
    }
    out
}

/// `succinctly jq args` with `D` replaced by `path` (or by the file's own name when `path` is `F`), over
/// a file holding `bytes` on standard input from `offset`.
fn run(bytes: &[u8], offset: u64, args: &[&str], path: &str, whole: bool) -> Result<Run> {
    let dir = tempfile::tempdir()?;
    let named = dir.path().join("in.json");
    std::fs::write(&named, bytes)?;
    let log = dir.path().join("routes.log");
    let mut file = std::fs::File::open(&named)?;
    file.seek(SeekFrom::Start(offset))?;
    let path = if path == "F" {
        named.as_os_str()
    } else {
        std::ffi::OsStr::new(path)
    };
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command
        .arg("jq")
        .args(
            args.iter()
                .map(|&arg| if arg == "D" { path } else { arg.as_ref() }),
        )
        .env("LC_ALL", "C")
        .env_remove(WHOLE)
        .env(LOG, &log)
        .stdin(Stdio::from(file.try_clone()?));
    small_window::apply(&mut command);
    if whole {
        command.env(WHOLE, "1");
    }
    let out = command.output()?;
    Ok(Run {
        stdout: String::from_utf8(out.stdout)?,
        stderr: String::from_utf8(out.stderr)?,
        status: out.status.code().unwrap_or(-1),
        routes: std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect(),
        offset: file.stream_position()?,
    })
}

/// The spellings of a path naming standard input this platform has, and whether a new open of each
/// shares descriptor 0's offset (macOS duplicates the descriptor).
fn spellings() -> Vec<(&'static str, bool)> {
    let mac = cfg!(target_os = "macos");
    let mut out = vec![("/dev/stdin", mac), ("/dev/fd/0", mac)];
    if cfg!(target_os = "linux") {
        out.push(("/proc/self/fd/0", false));
    }
    out
}

/// A program of each windowed route a lone named file takes, the route's name, whether its text is
/// `--seq` records, and the first line it answers when it reads the file from its start.
const PROGRAMS: &[(&[&str], &str, bool, &str)] = &[
    (&["-c", ".id", "D"], "plain", false, "0\n"),
    (&["-c", ".", "D"], "plain", false, "{\"id\":0}\n"),
    (&["-R", "length", "D"], "raw", false, "8\n"),
    (&["--seq", "-c", ".id", "D"], "seq", true, "\u{1e}0\n"),
];

/// Every program over `path` with standard input from `offset`: the route is windowed, and the run
/// answers and leaves standard input as the whole-file route does. Where the path opens the file anew,
/// the file is read from its start and standard input is left where it stood.
fn check(path: &str, shared: bool, offset: u64, fault: bool) -> Result<()> {
    for &(args, route, seq, first) in PROGRAMS {
        let bytes = text(seq, fault);
        let windowed = run(&bytes, offset, args, path, false)?;
        let whole = run(&bytes, offset, args, path, true)?;
        let what = format!("{path} from {offset} {args:?} fault={fault}");
        assert_eq!(windowed.routes, [route], "{what}: route");
        assert_eq!(whole.routes, Vec::<String>::new(), "{what}: forced whole");
        assert_eq!(
            Run {
                routes: Vec::new(),
                ..windowed
            },
            whole,
            "{what}"
        );
        if !shared {
            assert!(
                whole.stdout.starts_with(first),
                "{what}: {:?}",
                &whole.stdout[..whole.stdout.len().min(20)]
            );
            assert_eq!(whole.offset, offset, "{what}: offset");
        }
    }
    Ok(())
}

#[test]
fn a_path_naming_stdin_is_windowed_where_it_opens_the_file_anew_7698() -> Result<()> {
    for (path, shared) in spellings() {
        check(path, shared, 0, false)?;
    }
    Ok(())
}

/// Standard input that something has read part of: on Linux the path still opens the file from its
/// start, as jq reads it there, and the offset stays where it was.
#[test]
fn a_path_naming_stdin_read_part_way_is_read_from_its_start_7698() -> Result<()> {
    for (path, shared) in spellings() {
        check(path, shared, 9, false)?;
    }
    Ok(())
}

/// A parse error at the end is placed and worded as the whole-file route, and jq, word it.
#[test]
fn a_fault_in_a_path_naming_stdin_is_worded_as_jq_words_it_7698() -> Result<()> {
    for (path, shared) in spellings() {
        check(path, shared, 0, true)?;
        if shared {
            continue;
        }
        let bytes = text(false, true);
        let lines = bytes.iter().filter(|&&b| b == b'\n').count();
        let out = run(&bytes, 0, &["-c", ".id", "D"], path, false)?;
        let want: String = (0..lines - 1).map(|id| format!("{id}\n")).collect();
        assert_eq!(
            (out.stdout, out.stderr, out.status),
            (
                want,
                format!("jq: parse error: Unmatched '}}' at line {lines}, column 8\n"),
                5
            ),
            "{path}"
        );
    }
    Ok(())
}

/// The file's own name, with the same file on standard input, opens it anew on every platform.
#[test]
fn the_files_own_name_with_it_on_stdin_is_windowed_7698() -> Result<()> {
    check("F", false, 0, false)?;
    check("F", false, 9, true)
}
