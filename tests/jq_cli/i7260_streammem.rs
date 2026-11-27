//! #7260: the memory the streaming routes hold above the fixed baseline.
//!
//! - A lone `--seq` file whose stream does not open with an RS is scanned to its end without being
//!   held, and answered with jq's one warning when no RS jq sees is in it; one an RS or a malformed
//!   BOM turns up in is handed on as before. Every expectation here is jq 1.7.1's, captured from the
//!   pinned binary on fixtures of the same shape.
//! - A pipe's probe is read in blocks, and the certifying thread reads each window's own buffer.

use std::process::{Command, Stdio};

use anyhow::Result;

use crate::small_window;

/// `succinctly jq ARGS FILE` and `succinctly jq ARGS < FILE`, with the small window: stdout,
/// stderr (with `<stdin>` named as the file) and exit code of each.
fn run_both(args: &[&str], bytes: &[u8]) -> Result<[(String, String, i32); 2]> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    std::fs::write(&path, bytes)?;
    let named = small_window::apply(Command::new(env!("CARGO_BIN_EXE_succinctly")).arg("jq"))
        .args(args)
        .arg(&path)
        .output()?;
    let redirected = small_window::apply(Command::new(env!("CARGO_BIN_EXE_succinctly")).arg("jq"))
        .args(args)
        .stdin(std::fs::File::open(&path)?)
        .output()?;
    let name = path.to_string_lossy().into_owned();
    let text = |output: &std::process::Output| {
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).replace("<stdin>", &name),
            output.status.code().unwrap_or(-1),
        )
    };
    Ok([text(&named), text(&redirected)])
}

fn abandoned(line: usize, column: usize) -> String {
    format!(
        "jq: ignoring parse error: Unfinished abandoned text at EOF at line {line}, column {column}\n"
    )
}

/// One line of a document, with no line break and no RS anywhere: jq waits for an RS to its end and
/// warns at the column of the last byte.
#[test]
fn a_seq_file_of_one_line_with_no_rs_warns_at_its_end_7260() -> Result<()> {
    let mut bytes = b"{\"users\":[".to_vec();
    let mut id = 0;
    while bytes.len() < small_window::MIN_BYTES * 3 / 2 {
        bytes.extend(format!("{{\"id\":{id},\"name\":\"User{id}\"}},").as_bytes());
        id += 1;
    }
    bytes.extend(b"{\"id\":0}]}");
    let want = (String::new(), abandoned(1, bytes.len()), 0);
    for got in run_both(&["--seq", "-c", ".users[]|.id"], &bytes)? {
        assert_eq!(got, want);
    }
    Ok(())
}

/// Lines after a BOM, ending in a line break: the BOM is not counted, and the warning is at the line
/// after the last.
#[test]
fn a_seq_file_of_lines_after_a_bom_warns_on_the_line_after_them_7260() -> Result<()> {
    let line = b"{\"a\":1}\n";
    let lines = small_window::MIN_BYTES / line.len() + 1;
    let bytes = [b"\xEF\xBB\xBF".as_slice(), &line.repeat(lines)].concat();
    let want = (String::new(), abandoned(lines + 1, 0), 0);
    for got in run_both(&["--seq", "-c", "."], &bytes)? {
        assert_eq!(got, want);
    }
    Ok(())
}

/// A NUL in a 4095-byte read of jq's that holds no line break is counted with the rest of that
/// read (#7966): jq 1.7.1 ended the second read of one long line at the NUL at byte 5000 and did not
/// count the 3190 bytes after it (column 278954 here); jq 1.8.2 counts them (column 282144), and
/// so does `succinctly jq`, which does not reproduce 1.7.1's cut. A NUL before the first RS is never
/// scanned, so it is no fault.
#[test]
fn a_nul_in_a_long_line_is_counted_with_the_rest_of_its_read_7966() -> Result<()> {
    let mut bytes = vec![b'a'; small_window::MIN_BYTES + 20_000];
    bytes[5_000] = 0;
    let want = (String::new(), abandoned(1, bytes.len()), 0);
    for got in run_both(&["--seq", "-c", "."], &bytes)? {
        assert_eq!(got, want);
    }
    Ok(())
}

/// An RS far into the stream: the stream is the whole-stream route's as before, and the records after
/// it are run, with nothing said of the text before it.
#[test]
fn a_seq_file_with_an_rs_far_in_reads_the_records_after_it_7260() -> Result<()> {
    let line = b"{\"a\":1}\n";
    let mut bytes = line.repeat(small_window::MIN_BYTES / line.len() + 1);
    bytes.extend(b"\x1e{\"b\":2}\n\x1e{\"b\":3}\n");
    let want = (
        "\u{1e}{\"b\":2}\n\u{1e}{\"b\":3}\n".to_string(),
        String::new(),
        0,
    );
    for got in run_both(&["--seq", "-c", "."], &bytes)? {
        assert_eq!(got, want);
    }
    Ok(())
}

/// A malformed BOM leaves jq's parser reading: the stream is the reader's, which warns three times on
/// the first line and then reads its documents.
#[test]
fn a_seq_file_with_a_malformed_bom_is_read_by_the_reader_7260() -> Result<()> {
    let line = b"{\"a\":1}\n";
    let lines = small_window::MIN_BYTES / line.len() + 1;
    let bytes = [b"\xEF\xBBx".as_slice(), &line.repeat(lines)].concat();
    let stdout = format!("\u{1e}\"a\"\n{}", "\u{1e}{\"a\":1}\n".repeat(lines - 1));
    let stderr = "jq: ignoring parse error: Invalid numeric literal at line 1, column 2 (need RS to resync)\n\
                  jq: ignoring parse error: Expected string key before ':' at line 1, column 6 (need RS to resync)\n\
                  jq: ignoring parse error: Unmatched '}' at line 1, column 8 (need RS to resync)\n";
    for got in run_both(&["--seq", "-c", "."], &bytes)? {
        assert_eq!(got, (stdout.clone(), stderr.to_string(), 0));
    }
    Ok(())
}

/// A pipe of many windows with a fault in its probe's last block that the filter does not read (#2103),
/// invalid UTF-8, and a stray `}` far past the probe that ends the run: the documents, and the fault
/// reported (the stream's first, which the thread certifying the windows found), are the ones the
/// file route reads.
#[cfg(unix)]
#[test]
fn a_pipe_read_in_blocks_reads_as_its_file_does_7260() -> Result<()> {
    use std::io::Write;
    let line = b"{\"name\":\"n\",\"v\":[1,2,3]}\n";
    let lines = small_window::MIN_BYTES * 3 / line.len();
    let mut bytes = Vec::new();
    for i in 0..lines {
        if i == lines / 4 {
            bytes.extend(b"{\"bad\":[1 2]}\n");
        }
        if i == lines / 2 {
            bytes.extend(b"{\"s\":\"\xff\"}\n");
        }
        if i == lines * 3 / 4 {
            bytes.extend(b"}\n");
        }
        bytes.extend(line);
    }
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    std::fs::write(&path, &bytes)?;
    let named = small_window::apply(Command::new(env!("CARGO_BIN_EXE_succinctly")).arg("jq"))
        .args(["-c", ".name?"])
        .arg(&path)
        .output()?;
    let mut child = small_window::apply(Command::new(env!("CARGO_BIN_EXE_succinctly")).arg("jq"))
        .args(["-c", ".name?"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("piped");
    let writer = std::thread::spawn(move || stdin.write_all(&bytes));
    let piped = child.wait_with_output()?;
    // The run stops at the first fault, so the rest of the pipe may not be read.
    let _ = writer.join();
    let name = path.to_string_lossy().into_owned();
    assert_eq!(
        String::from_utf8_lossy(&piped.stdout),
        String::from_utf8_lossy(&named.stdout)
    );
    assert_eq!(
        String::from_utf8_lossy(&piped.stderr).replace("<stdin>", &name),
        String::from_utf8_lossy(&named.stderr)
    );
    assert_eq!(piped.status.code(), named.status.code());
    // jq 1.7.1 stops at the first: `jq: parse error: Expected separator between values at line L, ...`.
    assert_eq!(named.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&named.stderr)
        .starts_with("jq: parse error: Expected separator between values at line "));
    Ok(())
}

/// The rest of a lone `--seq` file after a record its windows cannot carry on past (a stretch that is
/// not plain records to the end of the stream) is handed to the route that reads a stream whole behind
/// one line break, with the lines before it counted: the warnings and the runtime error name the lines
/// jq names (jq 1.7.1, captured on the same shape).
#[test]
fn a_seq_tail_handed_on_names_the_lines_of_the_whole_stream_7260() -> Result<()> {
    let record = |id: usize| format!("\x1e{{\"id\":{id}}}\n");
    let records = small_window::MIN_BYTES / record(1000).len() + 1;
    let mut bytes: Vec<u8> = (0..records)
        .flat_map(|i| record(1000 + i).into_bytes())
        .collect();
    bytes.extend(b"\x1e{\"cut\n\x1e{\"id\":99}\nzzz");
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.seq");
    std::fs::write(&path, &bytes)?;
    let program = "if .id == 99 then error(\"x\") else empty end";
    let named = small_window::apply(Command::new(env!("CARGO_BIN_EXE_succinctly")).arg("jq"))
        .args(["--seq", "-c", program])
        .arg(&path)
        .output()?;
    let name = path.to_string_lossy();
    assert_eq!(String::from_utf8_lossy(&named.stdout), "");
    assert_eq!(
        String::from_utf8_lossy(&named.stderr),
        format!(
            "jq: ignoring parse error: Truncated value at line {}, column 1\n\
             jq: error (at {name}:{}): x\n\
             jq: ignoring parse error: Invalid numeric literal at EOF at line {}, column 3\n",
            records + 2,
            records + 2,
            records + 3
        )
    );
    assert_eq!(named.status.code(), Some(5));
    Ok(())
}
