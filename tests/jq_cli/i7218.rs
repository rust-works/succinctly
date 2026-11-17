//! #7218: a `--seq` run over several regular files, 4 MiB or more between them, is read a window at a
//! time when every file is plain `RS value LF` records ending in an LF, and whole otherwise. Every row
//! is what the same files give when the route declines them (they are handed over as named pipes):
//! stdout, stderr and the exit status, each file's name made one.

#![cfg(unix)]

use std::io::Write;
use std::process::Command;

use anyhow::Result;

use crate::fifo_feed::{self, Feed};

type Run = (String, String, i32);

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

fn same(args: &[&str], files: &[Vec<u8>]) -> Result<Run> {
    let (windowed, whole) = both(args, files)?;
    assert_eq!(windowed, whole, "{args:?}");
    Ok(windowed)
}

/// 4.5 MiB of records in three files, framed.
fn framed() -> Vec<Vec<u8>> {
    let each = 16_000;
    (0..3).map(|n| records(n * each, each)).collect()
}

#[test]
fn several_framed_seq_files_give_what_the_whole_route_gives_7218() -> Result<()> {
    let files = framed();
    for args in [
        &["--seq", "-c", ".id"][..],
        &["--seq", "-c", "[input_filename, input_line_number, .id]"],
        &["--seq", "-c", "$__loc__.line"],
        &["--seq", "-n", "-c", "[inputs | .id] | length"],
    ] {
        let (out, _, code) = same(args, &files)?;
        assert_eq!(code, 0, "{args:?}");
        assert_ne!(out, "");
    }
    Ok(())
}

#[test]
fn a_file_that_is_not_plain_records_sends_the_run_whole_7218() -> Result<()> {
    // No final LF in the middle of the list, a record cut short at the end of one file (the next file's
    // first bytes then continue it), an empty file, and a file with no RS to start.
    let mut no_lf = framed();
    no_lf[1].pop();
    let mut torn = framed();
    let keep = torn[1].len() - 40;
    torn[1].truncate(keep);
    let mut empty = framed();
    empty.insert(1, Vec::new());
    let mut unframed = framed();
    unframed[2].remove(0);
    for (name, files) in [
        ("no_lf", no_lf),
        ("torn", torn),
        ("empty", empty),
        ("unframed", unframed),
    ] {
        for args in [
            &["--seq", "-c", ".id"][..],
            &["--seq", "-c", "[input_filename, input_line_number, .id]"],
        ] {
            same(args, &files).map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
        }
    }
    Ok(())
}
