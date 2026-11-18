//! #7279: a `--seq` run over 2 to 32 regular files, which the run holds open from the start, is read a window
//! at a time up to the first file that is not plain `RS value LF` records ending in an LF (found by the prepass),
//! and that file and the ones after it are handed to the route that reads them whole, as for a longer list
//! (#7221). Every row is what the same files give when the route declines them (they are handed over as named
//! pipes): stdout, stderr and the exit status, each file's name made one.
//!
//! In a debug build the files are a sixteenth of the size and the binary's window is 64 KiB
//! (`small_window`, #7376); a release build runs the same rows over the 1 MiB window.

#![cfg(unix)]

use std::io::Write;
use std::process::Command;

use anyhow::Result;

use crate::fifo_feed::{self, Feed};
use crate::small_window::{self, scaled};

type Run = (String, String, i32);

/// Within the 32 files the run holds open from the start.
const FILES: usize = 8;

/// Records in each file: 8 files of about 660 KB on the shipped window, so 5.3 MB between them, over the
/// 4 MiB (four windows) the route needs. [`framed`] checks it at the window in use.
const EACH: usize = scaled(6_000);

fn records(from: usize, count: usize) -> Vec<u8> {
    (from..from + count)
        .flat_map(|i| format!("\x1e{{\"id\":{i},\"pad\":\"{}\"}}\n", "x".repeat(90)).into_bytes())
        .collect()
}

/// The files as paths (the windowed route), and as named pipes (the route that reads them whole).
fn both(args: &[&str], files: &[Vec<u8>]) -> Result<(Run, Run)> {
    let dir = tempfile::tempdir()?;
    let mut paths = Vec::new();
    let mut fifos = Vec::new();
    let mut feeds = Vec::new();
    for (n, bytes) in files.iter().enumerate() {
        let path = dir.path().join(format!("f{n:02}.seq"));
        std::fs::File::create(&path)?.write_all(bytes)?;
        let fifo = dir.path().join(format!("p{n:02}"));
        feeds.push(Feed::new(fifo.clone(), bytes.clone()));
        paths.push(path);
        fifos.push(fifo);
    }
    // The pipes, and the deadline the run is held to, are `fifo_feed`'s (#7304).
    let run = |operands: &[std::path::PathBuf], feeds: &[Feed]| -> Result<Run> {
        let output = fifo_feed::output(
            small_window::apply(
                Command::new(env!("CARGO_BIN_EXE_succinctly"))
                    .arg("jq")
                    .args(args)
                    .args(operands)
                    .env("LC_ALL", "C"),
            ),
            feeds,
        )?;
        let name = |bytes: &[u8]| {
            let mut text = String::from_utf8_lossy(bytes).into_owned();
            for (n, operand) in operands.iter().enumerate().rev() {
                text = text.replace(&operand.display().to_string(), &format!("<file {n}>"));
            }
            text
        };
        Ok((
            name(&output.stdout),
            name(&output.stderr),
            output.status.code().unwrap_or(-1),
        ))
    };
    let windowed = run(&paths, &[])?;
    let whole = run(&fifos, &feeds)?;
    Ok((windowed, whole))
}

fn same(name: &str, args: &[&str], files: &[Vec<u8>]) -> Result<Run> {
    let (windowed, whole) = both(args, files)?;
    assert_eq!(windowed, whole, "{name}: {args:?}");
    Ok(windowed)
}

/// Over four windows of records in 8 files (5.3 MB on the shipped window), framed.
fn framed() -> Vec<Vec<u8>> {
    let files: Vec<Vec<u8>> = (0..FILES).map(|n| records(n * EACH, EACH)).collect();
    assert!(
        files.iter().map(Vec::len).sum::<usize>() > small_window::MIN_BYTES + 4096,
        "the files must be windowed"
    );
    files
}

#[test]
fn several_framed_seq_files_give_what_the_whole_route_gives_7279() -> Result<()> {
    let files = framed();
    // Programs that leave the run on the route that reads in windows: `input_line_number` takes it to the
    // one that reads the files whole.
    for args in [
        &["--seq", "-c", ".id"][..],
        &["--seq", "-c", "[input_filename, .id]"],
    ] {
        let (out, err, code) = same("framed", args, &files)?;
        assert_eq!((err.as_str(), code), ("", 0), "{args:?}");
        assert_eq!(out.lines().count(), FILES * EACH, "{args:?}");
    }
    Ok(())
}

/// The files from the first that is not plain records are read whole, where jq's reader stands after the
/// files before: a parse warning names the line counted over every file, a runtime error the line in its own
/// file, a BOM there is not the stream's front, a bare `n` is read over the byte the records before left in
/// jq's token buffer, and a record cut short at a file's end runs on into the next file.
#[test]
fn the_files_from_one_that_is_not_plain_records_are_read_whole_7279() -> Result<()> {
    let at = 4;
    let with = |turn: &dyn Fn(&mut Vec<Vec<u8>>)| {
        let mut files = framed();
        turn(&mut files);
        files
    };
    let cases: Vec<(&str, Vec<Vec<u8>>)> = vec![
        (
            "torn",
            with(&|f| {
                let keep = f[at].len() - 40;
                f[at].truncate(keep);
            }),
        ),
        ("empty", with(&|f| f.insert(at, Vec::new()))),
        (
            "unframed",
            with(&|f| {
                f[at].remove(0);
            }),
        ),
        (
            "bom",
            with(&|f| f[at].splice(0..0, *b"\xef\xbb\xbf").for_each(drop)),
        ),
        (
            "runtime_error",
            with(&|f| {
                f[at].pop();
                f[at].splice(0..0, *b"\x1e7\n").for_each(drop);
            }),
        ),
        // A bare `n` is read by jq's literal check over the byte the last token left in its buffer, which
        // the string at the end of the file before leaves as `u`.
        (
            "token_byte",
            with(&|f| {
                f[at - 1].extend_from_slice(b"\x1e\"nu\"\n\x1e[]\n");
                f[at].splice(0..0, *b"\x1en\n").for_each(drop);
            }),
        ),
        (
            "cut_at_a_files_end",
            with(&|f| f[at].extend_from_slice(b"\x1e{\"id\":-1,\"name\":\"cut\n")),
        ),
        (
            "bad_in_the_middle_of_a_file",
            with(&|f| {
                let mid = f[at].len() / 2;
                let rs = f[at][mid..].iter().position(|&b| b == 0x1e).unwrap() + mid;
                f[at].splice(rs..rs, *b"\x1e[1,]\n").for_each(drop);
            }),
        ),
        (
            "first_file",
            with(&|f| {
                f[0].pop();
            }),
        ),
        (
            "last_file",
            with(&|f| {
                f[FILES - 1].pop();
            }),
        ),
    ];
    // A record of the file after the one that turns (30,000 of 48,000 on the shipped window).
    let boom = format!(
        "if .id == {} then error(\"boom\") else empty end",
        FILES * EACH * 5 / 8
    );
    for (name, files) in cases {
        same(name, &["--seq", "-c", "[input_filename, .id]"], &files)?;
        same(name, &["--seq", "-c", &boom], &files)?;
    }
    Ok(())
}
