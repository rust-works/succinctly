//! #7425: a document of 4 GiB or more, which no `JsonIndex` can hold, is read without one.
//!
//! jq 1.7.1 reads such a document; `succinctly jq` panicked with exit 101 on the index's
//! `u32::MAX` assertion. It now reads the document into a tree with the one-pass reader of plain
//! JSON and runs the program on the tree, for a named file, a pipe and a `-s` slurp of a stream that
//! long. The outputs below were captured from `/usr/bin/jq` 1.7.1 on the same fixtures.
//!
//! Ignored by default: the fixtures are 8 GiB on disk and a run holds about 9 GB. Run with
//! `cargo test --features cli --test jq_cli_tests i7425 -- --ignored`. The routing decision and the
//! reader are covered without such a file by the unit tests beside them in `jq_runner.rs`.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;

/// 4097 strings of `A_COUNT` letters each and a last `"x"`: 4,296,011,781 bytes as one array,
/// 2^32 + 1,044,485, and two bytes fewer one per line.
const STRINGS: usize = 4097;
const A_COUNT: usize = (1 << 20) - 4;

/// One array of the strings (`array`), or the strings one per line (`!array`).
fn write_fixture(path: &Path, array: bool) -> Result<()> {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut item = Vec::with_capacity(A_COUNT + 3);
    item.push(b'"');
    item.resize(A_COUNT + 1, b'a');
    item.push(b'"');
    item.push(if array { b',' } else { b'\n' });
    if array {
        file.write_all(b"[")?;
    }
    for _ in 0..STRINGS {
        file.write_all(&item)?;
    }
    file.write_all(if array { b"\"x\"]\n" } else { b"\"x\"\n" })?;
    file.flush()?;
    assert_eq!(
        std::fs::metadata(path)?.len(),
        if array { 4_296_011_781 } else { 4_296_011_779 }
    );
    Ok(())
}

/// `succinctly jq <args>`, with `stdin` from the file when given: `(stdout, stderr, status)`.
fn run(args: &[&str], stdin: Option<&Path>) -> Result<(String, String, Option<i32>)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command.arg("jq").args(args);
    command.stdin(match stdin {
        Some(path) => Stdio::from(std::fs::File::open(path)?),
        None => Stdio::null(),
    });
    let output = command.output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    ))
}

#[test]
#[ignore = "writes 8 GiB of fixtures and needs about 9 GB of memory"]
fn a_document_of_4_gib_or_more_is_read_as_jq_reads_it_7425() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let array = dir.path().join("array.json");
    let lines = dir.path().join("lines.json");
    write_fixture(&array, true)?;
    write_fixture(&lines, false)?;
    let array_arg = array.to_str().expect("utf-8 path");
    let lines_arg = lines.to_str().expect("utf-8 path");
    // (arguments, stdin, stdout as jq 1.7.1 prints it).
    let rows: [(&[&str], Option<&Path>, &str); 5] = [
        (&["length", array_arg], None, "4098\n"),
        (
            &["-c", ".[-1], (.[0] | length)", array_arg],
            None,
            "\"x\"\n1048572\n",
        ),
        (&["length"], Some(&array), "4098\n"),
        (&["-s", "length"], Some(&lines), "4098\n"),
        (&["-s", ".[4097]", lines_arg], None, "\"x\"\n"),
    ];
    for (args, stdin, stdout) in rows {
        let (out, err, status) = run(args, stdin)?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (stdout, "", Some(0)),
            "{args:?}"
        );
    }
    Ok(())
}
