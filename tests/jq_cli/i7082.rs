//! #7082: `succinctly jq --seq` reads one big regular file of RFC 7464's own framing (every record
//! `RS value LF`) a window at a time, the RS bytes read as spaces, so its peak memory no longer grows
//! with the file. Every row is what the same bytes give on a pipe, which the whole-file route reads:
//! stdout, stderr (the file's name for stdin's) and exit status. A stream that is not that framing
//! keeps the route that walks it whole, and the rows for those pin that the gate does not vouch for
//! one it should not.
//!
//! The files are over 4 MiB (the smallest the route takes); `windowed_seq_file_takes_only_framed_streams_7082`
//! in `jq_runner.rs` pins which runs take it.

use anyhow::Result;

use crate::spawn_jq;

/// How many records the fixtures have: about 6.8 MiB, well over the 4 MiB the route takes.
const RECORDS: usize = 60_000;

fn record(i: usize) -> String {
    format!(
        "{{\"id\":{i},\"name\":\"user {i}\",\"email\":\"user{i}@example.com\",\"tags\":[\"a\",\"b\",\"c\"],\"score\":{}}}",
        i % 1000
    )
}

/// `RS value LF` for each of `records`.
fn framed(records: &[String]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for r in records {
        bytes.push(0x1e);
        bytes.extend_from_slice(r.as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

fn clean() -> Vec<u8> {
    framed(&(0..RECORDS).map(record).collect::<Vec<_>>())
}

/// The run over `bytes` as a file and as stdin, compared; the file's stdout, stderr and status.
fn same_as_stdin(bytes: &[u8], args: &[&str]) -> Result<(Vec<u8>, String, Option<i32>)> {
    assert!(
        bytes.len() > 4 * 1024 * 1024,
        "the file must be over the windowed route's minimum"
    );
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("big.seq");
    std::fs::write(&path, bytes)?;
    let name = path.to_string_lossy().into_owned();
    let mut with_file: Vec<&str> = args.to_vec();
    with_file.push(&name);
    let (file, _) = spawn_jq(&with_file, None)?;
    let (stdin, _) = spawn_jq(args, Some(bytes))?;
    let (file_status, stdin_status) = (file.status.code(), stdin.status.code());
    let file_err = String::from_utf8_lossy(&file.stderr).replace(&name, "<stdin>");
    let stdin_err = String::from_utf8_lossy(&stdin.stderr).into_owned();
    assert!(
        file.stdout == stdin.stdout,
        "{args:?}: stdout differs ({} and {} bytes)",
        file.stdout.len(),
        stdin.stdout.len()
    );
    assert_eq!(file_err, stdin_err, "{args:?}: stderr");
    assert_eq!(file_status, stdin_status, "{args:?}: exit status");
    // The same file redirected onto standard input is the stream too (#7024's shape), read from a
    // duplicate of descriptor 0 by position.
    let redirected = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::from(std::fs::File::open(&path)?))
        .output()?;
    assert!(
        redirected.stdout == stdin.stdout,
        "{args:?}: stdout differs on a redirected file"
    );
    assert_eq!(
        String::from_utf8_lossy(&redirected.stderr),
        stdin_err,
        "{args:?}: stderr on a redirected file"
    );
    assert_eq!(
        redirected.status.code(),
        stdin_status,
        "{args:?}: exit status on a redirected file"
    );
    Ok((file.stdout, file_err, file_status))
}

#[test]
fn a_framed_file_reads_as_its_bytes_on_stdin_do_7082() -> Result<()> {
    let bytes = clean();
    for args in [
        &["--seq", "-c", "select(.id % 7919 == 0) | .id"][..],
        &["--seq", "-r", "select(.id % 9973 == 0) | .name"],
        &["--seq", "-c", "select(.id > 59990)"],
        &["--seq", "-e", "select(.id == 59999) | .name"],
        &["--seq", "-c", ".tags | length", "--arg", "x", "1"],
        &["--seq", "."],
    ] {
        let (out, err, status) = same_as_stdin(&bytes, args)?;
        assert_eq!((err.as_str(), status), ("", Some(0)), "{args:?}");
        assert!(!out.is_empty(), "{args:?} selects something");
    }
    // The name of the file a record came from.
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("big.seq");
    std::fs::write(&path, &bytes)?;
    let name = path.to_string_lossy().into_owned();
    let (out, _) = spawn_jq(
        &[
            "--seq",
            "-c",
            "select(.id == 3) | [input_filename, .id]",
            &name,
        ],
        None,
    )?;
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("\x1e[{name:?},3]\n")
    );
    // `--seq` frames what it writes: every output starts with an RS.
    let (out, _, _) = same_as_stdin(&bytes, &["--seq", "-c", "select(.id == 5) | .id"])?;
    assert_eq!(out, b"\x1e5\n");
    Ok(())
}

#[test]
fn the_line_a_runtime_error_names_is_the_records_7082() -> Result<()> {
    let bytes = clean();
    let (out, err, status) = same_as_stdin(
        &bytes,
        &[
            "--seq",
            "-c",
            "if .id == 30000 then error(\"boom\") else empty end",
        ],
    )?;
    assert!(out.is_empty(), "the error draws no output");
    assert_eq!(err, "jq: error (at <stdin>:30001): boom\n");
    // (jq 1.7.1 exits 0 here too: the error is not the last thing the run saw.)
    assert_eq!(status, Some(0));
    let (out, _, _) = same_as_stdin(
        &bytes,
        &[
            "--seq",
            "-c",
            "select(.id % 20000 == 7) | input_line_number",
        ],
    )?;
    assert_eq!(out, b"\x1e8\n\x1e20008\n\x1e40008\n");
    Ok(())
}

/// A stream that is not RFC 7464's own framing is not this route's, whatever is wrong with it: the
/// text is walked whole as it always was, and says what it always said.
#[test]
fn a_stream_that_is_not_framed_keeps_the_route_that_walks_it_7082() -> Result<()> {
    let records: Vec<String> = (0..RECORDS).map(record).collect();
    let args = ["--seq", "-c", "select(.id % 20000 == 0) | .id"];
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();

    // A record the strict validator turns down, in the middle and at the end.
    for (name, at) in [("middle", RECORDS / 2), ("end", RECORDS - 1)] {
        let mut bad = records.clone();
        bad[at] = "{\"id\":}".to_string();
        cases.push((name, framed(&bad)));
    }
    // A truncated tail: the last record has no line feed, or stops inside its value.
    let mut no_lf = clean();
    no_lf.pop();
    cases.push(("no final LF", no_lf));
    let mut cut = clean();
    cut.truncate(cut.len() - 20);
    cases.push(("cut tail", cut));
    // Bytes before the first RS, and a stream that does not start on one.
    let mut leading = b"junk\n".to_vec();
    leading.extend(clean());
    cases.push(("bytes before the first RS", leading));
    // An RS straight after a value, so that a record is cut.
    let mut doubled = clean();
    let at = doubled.iter().position(|&b| b == b'\n').unwrap();
    doubled[at] = 0x1e;
    cases.push(("RS where a LF should be", doubled));
    // An empty record, and a blank line between records (which is still a framed stream).
    let mut empty = clean();
    empty.splice(0..0, [0x1e]);
    cases.push(("an empty record", empty));
    let mut blank = clean();
    let at = blank.iter().position(|&b| b == b'\n').unwrap();
    blank.insert(at + 1, b'\n');
    cases.push(("a blank line", blank));
    // A byte-order mark, a record that is not UTF-8, and a number the validator does not take.
    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend(clean());
    cases.push(("a BOM", bom));
    let mut invalid = clean();
    let at = invalid
        .windows(10)
        .position(|w| w == b"user 40000")
        .unwrap();
    invalid.insert(at, 0xff);
    cases.push(("invalid UTF-8", invalid));
    let mut number = records.clone();
    number[100] = "007".to_string();
    cases.push(("a leading zero", framed(&number)));
    // Records that are not framed at all: plain NDJSON under --seq is one abandoned text.
    let plain = records.join("\n").into_bytes();
    cases.push(("no RS at all", plain));

    for (name, bytes) in &cases {
        same_as_stdin(bytes, &args).map_err(|e| e.context(format!("case {name}")))?;
    }
    Ok(())
}

/// A record longer than a window, among small ones, is found whole.
#[test]
fn records_that_span_windows_are_found_whole_7082() -> Result<()> {
    // One record over a window (1 MiB) long, among small ones.
    let big = format!(
        "[{}]",
        (0..400_000)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut records: Vec<String> = (0..30_000).map(record).collect();
    records.insert(15_000, big);
    let bytes = framed(&records);
    for args in [
        &["--seq", "-c", "select(type == \"array\") | length"][..],
        &[
            "--seq",
            "-c",
            "select(type == \"object\" and .id % 10007 == 0) | .id",
        ],
    ] {
        let (out, err, status) = same_as_stdin(&bytes, args)?;
        assert_eq!((err.as_str(), status), ("", Some(0)), "{args:?}");
        assert!(!out.is_empty(), "{args:?} selects something");
    }
    Ok(())
}
