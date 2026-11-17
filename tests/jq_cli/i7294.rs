//! #7294: the last window of a pipe on standard input is certified only when a fault is asked for, over
//! its buffer, which it shares with the certifier. A pipe that holds one big document (one line, 6 MB
//! here) is that one window, so a run over it reads no fault, and a run that does is worded from the
//! certificate made then: the stream's first fault, which can lie in a part of the document the filter
//! never read, before the one it met. Each row is jq 1.7.1's answer over the same bytes (captured from
//! `/usr/bin/jq`), and the same bytes read whole (the pipe named `/dev/stdin`, which the windowed routes
//! decline) give the same stdout, stderr and status.

#![cfg(unix)]

use anyhow::Result;

use crate::fifo_feed::spawn_jq_deadline;

/// One line: `{"users":[...],"tail":TAIL}` with `mid` put among the records at the middle, and `end`
/// in place of the closing brace.
fn document(mid: &[u8], tail: &[u8], end: &[u8]) -> Vec<u8> {
    let count = 60_000;
    let mut records: Vec<Vec<u8>> = (0..count)
        .map(|i| {
            format!(
                "{{\"id\":{i},\"name\":\"user {i}\",\"pad\":\"{}\"}}",
                "x".repeat(60)
            )
            .into_bytes()
        })
        .collect();
    if !mid.is_empty() {
        records.insert(count / 2, mid.to_vec());
    }
    let mut bytes = b"{\"users\":[".to_vec();
    bytes.extend(records.join(&b","[..]));
    bytes.extend_from_slice(b"],\"tail\":");
    bytes.extend_from_slice(tail);
    bytes.extend_from_slice(end);
    bytes
}

/// The run over `bytes` on a pipe, checked against the same bytes read whole: stdout, stderr, status.
fn piped(args: &[&str], bytes: &[u8]) -> Result<(String, String, i32)> {
    assert!(bytes.len() > 4 << 20, "the pipe must be windowed");
    let (windowed, status) = spawn_jq_deadline(args, Some(bytes))?;
    let mut whole_args = args.to_vec();
    whole_args.push("/dev/stdin");
    let (whole, whole_status) = spawn_jq_deadline(&whole_args, Some(bytes))?;
    let out = String::from_utf8_lossy(&windowed.stdout).into_owned();
    let err = String::from_utf8_lossy(&windowed.stderr).into_owned();
    assert_eq!(
        out,
        String::from_utf8_lossy(&whole.stdout),
        "{args:?}: stdout"
    );
    assert_eq!(
        err,
        String::from_utf8_lossy(&whole.stderr).replace("/dev/stdin", "<stdin>"),
        "{args:?}: stderr"
    );
    assert_eq!(status, whole_status, "{args:?}: status");
    Ok((out, err, status))
}

#[test]
fn one_big_document_on_a_pipe_runs_as_jq_reads_it_7294() -> Result<()> {
    let bytes = document(b"", b"1", b"}");
    let first = format!(
        "{{\"id\":0,\"name\":\"user 0\",\"pad\":\"{}\"}}\n",
        "x".repeat(60)
    );
    assert_eq!(
        piped(&["-c", ".users[0]"], &bytes)?,
        (first, String::new(), 0)
    );
    assert_eq!(
        piped(&["-c", ".tail"], &bytes)?,
        ("1\n".into(), String::new(), 0)
    );
    assert_eq!(
        piped(&["-n", "-c", "input|.tail"], &bytes)?,
        ("1\n".into(), String::new(), 0)
    );
    assert_eq!(
        piped(&["-c", "[.users[]|.id]|length"], &bytes)?,
        ("60000\n".into(), String::new(), 0)
    );
    Ok(())
}

/// The document is cut short, so the run asks for the fault at its end, and jq's is the first of the
/// stream: in the middle, in records `.tail` and `.users[0]` never read.
#[test]
fn a_cut_short_document_words_the_fault_the_filter_did_not_read_7294() -> Result<()> {
    let bytes = document(b"[1 2]", b"1", b"");
    let fault = "jq: parse error: Expected separator between values at line 1, column 3037795\n";
    for args in [&["-c", ".tail"][..], &["-c", ".users[0]"]] {
        assert_eq!(
            piped(args, &bytes)?,
            (String::new(), fault.into(), 5),
            "{args:?}"
        );
    }
    // Bytes that are not UTF-8 before it count as read, not as their substitution.
    let bytes = document(b"\"\xff\xfe\" , [1 2]", b"1", b"");
    assert_eq!(
        piped(&["-c", ".tail"], &bytes)?,
        (
            String::new(),
            "jq: parse error: Expected separator between values at line 1, column 3037802\n".into(),
            5
        )
    );
    Ok(())
}

/// A fault the filter meets while it runs over the window asks for the certificate then.
#[test]
fn a_fault_met_while_the_document_runs_is_jqs_7294() -> Result<()> {
    let bytes = document(b"{\"id\":-1,\"x\":[1 2]}", b"1", b"}");
    let fault = "jq: parse error: Expected separator between values at line 1, column 3037808\n";
    for args in [
        &["-c", ".users[]|select(.id<0)"][..],
        &["-c", "."],
        &["-c", "[.users[]|.x]|length"],
    ] {
        assert_eq!(
            piped(args, &bytes)?,
            (String::new(), fault.into(), 5),
            "{args:?}"
        );
    }
    Ok(())
}

/// Trailing garbage after the document: its result is printed first, then jq's fault at the end.
#[test]
fn garbage_after_one_big_document_follows_its_output_7294() -> Result<()> {
    let mut bytes = document(b"", b"1", b"}");
    bytes.extend_from_slice(b" x");
    assert_eq!(
        piped(&["-c", ".tail"], &bytes)?,
        (
            "1\n".into(),
            "jq: parse error: Invalid numeric literal at EOF at line 1, column 6097802\n".into(),
            5
        )
    );
    Ok(())
}
