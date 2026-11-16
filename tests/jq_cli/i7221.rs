//! #7221: a `--seq` run over more than 32 regular files, which the run opens by path as it reaches each, is read a
//! window at a time while each file is plain `RS value LF` records ending in an LF, judged when the run opens it;
//! the files from the first that is not are handed to the route that reads them whole, which reads them as jq's
//! reader does after the files before. Every row is what the same files give when the route declines them (they
//! are handed over as named pipes): stdout, stderr and the exit status, each file's name made one.

#![cfg(unix)]

use std::io::Write;
use std::process::Command;

use anyhow::Result;

use crate::fifo_feed::{self, Feed};

type Run = (String, String, i32);

/// More than the 32 files the run holds open from the start.
const FILES: usize = 40;

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
            Command::new(env!("CARGO_BIN_EXE_succinctly"))
                .arg("jq")
                .args(args)
                .args(operands)
                .env("LC_ALL", "C"),
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

/// 4.4 MiB of records in 40 files, framed.
fn framed() -> Vec<Vec<u8>> {
    let each = 1_000;
    (0..FILES).map(|n| records(n * each, each)).collect()
}

#[test]
fn many_framed_seq_files_give_what_the_whole_route_gives_7221() -> Result<()> {
    let files = framed();
    // Programs that leave the run on the route that reads in windows: `input_line_number` takes it to the
    // one that reads the files whole.
    for args in [
        &["--seq", "-c", ".id"][..],
        &["--seq", "-c", "[input_filename, .id]"],
    ] {
        let (out, err, code) = same("framed", args, &files)?;
        assert_eq!((err.as_str(), code), ("", 0), "{args:?}");
        assert_eq!(out.lines().count(), FILES * 1_000, "{args:?}");
    }
    Ok(())
}

/// The files from the first that is not plain records are read whole, where jq's reader stands after the
/// files before: a parse warning names the line counted over every file, a runtime error the line in its own
/// file, a BOM there is not the stream's front, a bare `n` is read over the byte the records before left in
/// jq's token buffer, and a record cut short at a file's end runs on into the next file.
#[test]
fn the_files_from_one_that_is_not_plain_records_are_read_whole_7221() -> Result<()> {
    let at = 20;
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
    for (name, files) in cases {
        same(name, &["--seq", "-c", "[input_filename, .id]"], &files)?;
    }
    Ok(())
}
