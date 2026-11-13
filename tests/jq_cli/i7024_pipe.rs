//! #7024: a pipe on standard input that holds 4 MiB or more is read a window at a time under the plain
//! filter, and its first fault is found as its windows are read, since a pipe cannot be read again for
//! it. Every row is checked against the same bytes read whole: the pipe named as `/dev/stdin`, which
//! the windowed routes decline (it names a descriptor the run already holds), so stdout, stderr (with
//! that name read as `<stdin>`) and the exit status are the whole-file route's.

#![cfg(unix)]

use anyhow::Result;

use crate::spawn_jq;

const LINES: usize = 60_000;

fn lines(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            format!(
                "{{\"id\":{i},\"name\":\"user {i}\",\"email\":\"user{i}@example.com\",\"tags\":[\"a\",\"b\",\"c\"],\"score\":{}}}",
                i % 1000
            )
        })
        .collect()
}

fn join(lines: &[String]) -> Vec<u8> {
    let mut bytes = lines.join("\n").into_bytes();
    bytes.push(b'\n');
    bytes
}

/// `succinctly jq ARGS` over `bytes` on a pipe, read the windowed way and the whole way: the stdout,
/// stderr and exit status of the windowed run, once they are checked to be the whole run's.
fn same(args: &[&str], bytes: &[u8]) -> Result<(String, String, i32)> {
    let (windowed, windowed_status) = spawn_jq(args, Some(bytes))?;
    let mut whole_args = args.to_vec();
    whole_args.push("/dev/stdin");
    let (whole, whole_status) = spawn_jq(&whole_args, Some(bytes))?;
    let whole_err = String::from_utf8_lossy(&whole.stderr).replace("/dev/stdin", "<stdin>");
    let err = String::from_utf8_lossy(&windowed.stderr).into_owned();
    assert!(
        windowed.stdout == whole.stdout,
        "{args:?}: stdout differs ({} and {} bytes)",
        windowed.stdout.len(),
        whole.stdout.len()
    );
    assert_eq!(err, whole_err, "{args:?}: stderr");
    assert_eq!(windowed_status, whole_status, "{args:?}: exit status");
    Ok((
        String::from_utf8_lossy(&windowed.stdout).into_owned(),
        err,
        windowed_status,
    ))
}

#[test]
fn a_pipe_reads_as_the_whole_stream_does_7024() -> Result<()> {
    let clean = join(&lines(LINES));
    assert!(clean.len() > 4 * 1024 * 1024 + 1024 * 1024);
    for args in [
        &["-c", "."][..],
        &["-r", "select(.id % 9973 == 0) | .name"],
        &["-c", "select(.score > 990) | .id"],
        &["-e", "select(.id == 59999) | .name"],
        &["-e", ".id == 0"],
    ] {
        same(args, &clean)?;
    }
    // The line a runtime error names.
    let (_, err, _) = same(
        &["-c", "if .id == 30000 then error(\"boom\") else empty end"],
        &clean,
    )?;
    assert_eq!(err, "jq: error (at <stdin>:30001): boom\n");
    // A BOM at the front.
    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend(&clean);
    same(&["-c", "select(.id == 0) | .name"], &bom)?;
    Ok(())
}

/// The stream's first fault is jq's, even where it is in a document the filter never read and the run
/// passed long before the end (a filter validates only what it reads, #2103): the window that held it
/// is gone by the time the fault is worded.
#[test]
fn a_pipe_words_the_first_fault_after_its_window_is_gone_7024() -> Result<()> {
    // A fault in a member `.name` never reads, then a stream that ends inside a document.
    let mut early = lines(LINES);
    early[1] = "{\"id\":1,\"name\":\"user 1\",\"tags\":[1 2]}".to_string();
    let mut early = join(&early);
    early.extend_from_slice(b"{\"id\":");
    let (out, err, status) = same(&["-r", "select(.id % 20000 == 0) | .name"], &early)?;
    assert_eq!(out, "user 0\nuser 20000\nuser 40000\n");
    assert_eq!(
        err,
        "jq: parse error: Expected separator between values at line 2, column 36\n"
    );
    assert_ne!(status, 0);
    // `-c .` reads it, and stops there.
    let (out, err, _) = same(&["-c", "."], &early)?;
    assert_eq!(out.lines().count(), 1);
    assert!(err.contains("at line 2, column 36"), "{err}");
    // A fault late in the stream, and one at its end only.
    let mut late = lines(LINES);
    late[50_000] = "{\"id\":50000,\"name\":\"x\",\"k\" 1}".to_string();
    same(&["-r", "select(.id % 20000 == 0) | .name"], &join(&late))?;
    let mut cut = join(&lines(LINES));
    cut.extend_from_slice(b"{\"id\":1,");
    let (_, err, _) = same(&["-c", "select(.id == 59999) | .id"], &cut)?;
    assert!(err.contains("at EOF"), "{err}");
    Ok(())
}

/// Columns are counted in the bytes as read, so a fault after bytes that are not UTF-8 is placed past
/// them, not past their substitution; and a document past jq's depth limit is refused where jq refuses it.
#[test]
fn a_pipe_counts_a_fault_in_the_bytes_as_read_7024() -> Result<()> {
    let mut bytes = join(&lines(LINES));
    let at = bytes.windows(10).position(|w| w == b"user 40000").unwrap();
    bytes.splice(at..at, *b"\xff\xfe\xfd");
    same(&["-c", "select(.id == 40000) | .name"], &bytes)?;
    let mut outside = lines(LINES);
    outside[45_000] = "{\"id\":45000,\"name\":\"n\",\"v\":\u{0}\u{1}}".to_string();
    let mut outside = join(&outside);
    let at = outside.windows(5).position(|w| w == b"\"v\":\0").unwrap() + 4;
    outside.splice(at..at + 2, *b"\xff\xff");
    let (_, err, _) = same(&["-c", "select(.id % 20000 == 0) | .id"], &outside)?;
    assert!(err.contains("line 45001"), "{err}");
    let mut deep = lines(LINES);
    deep[30_000] = format!(
        "{{\"id\":30000,\"d\":{}{}}}",
        "[".repeat(300),
        "]".repeat(300)
    );
    let (_, err, _) = same(&["-c", "select(.id % 20000 == 0) | .id"], &join(&deep))?;
    assert!(err.contains("Exceeds depth limit for parsing"), "{err}");
    Ok(())
}

/// A pipe under 4 MiB is read whole, as before: the probe hands what it read to that route. One of
/// exactly 4 MiB and one a byte over read the same.
#[test]
fn a_short_pipe_and_the_threshold_read_the_same_7024() -> Result<()> {
    const MIN: usize = 4 * 1024 * 1024;
    let clean = join(&lines(LINES));
    let line_end = |near: usize| clean[..near].iter().rposition(|&b| b == b'\n').unwrap() + 1;
    for len in [10_000, line_end(MIN), MIN - 1, MIN, MIN + 1] {
        let (_, _, status) = same(&["-c", "select(.id % 9000 == 0) | .id"], &clean[..len])?;
        // A cut that is not at a line end leaves a document open at the end of the stream.
        assert_eq!(status == 0, clean[len - 1] == b'\n', "length {len}");
    }
    Ok(())
}
