//! #7295: `--stream` over a pipe named by path (`/dev/stdin`, `/dev/fd/0`) failed with `Illegal seek` and
//! printed nothing, valid input included. The stream reader probed each file for a BOM, closed it, and
//! opened it again at the byte after the BOM, which a pipe cannot do (and what the probe read is gone from
//! it). A source that is not a regular file is now kept open and read on from the probe. Each row is jq
//! 1.7.1's answer over the same bytes (captured from `/usr/bin/jq`).

#![cfg(unix)]

use anyhow::Result;

use crate::fifo_feed::spawn_jq_deadline;

/// `--stream` over `bytes` on a pipe named `source`: stdout, stderr and the exit status.
fn stream(source: &str, bytes: &[u8]) -> Result<(String, String, i32)> {
    let (output, status) = spawn_jq_deadline(&["-c", "--stream", ".", source], Some(bytes))?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        status,
    ))
}

const NAMES: [&str; 2] = ["/dev/stdin", "/dev/fd/0"];

#[test]
fn a_stream_over_a_pipe_named_by_path_is_read_7295() -> Result<()> {
    for name in NAMES {
        let (out, err, status) = stream(name, b"[1,2]")?;
        assert_eq!(out, "[[0],1]\n[[1],2]\n[[1]]\n", "{name}");
        assert_eq!(err, "", "{name}");
        assert_eq!(status, 0, "{name}");
        let (out, _, status) = stream(name, br#"{"a":[1,{"b":2}]} 3"#)?;
        assert_eq!(
            out, "[[\"a\",0],1]\n[[\"a\",1,\"b\"],2]\n[[\"a\",1,\"b\"]]\n[[\"a\",1]]\n[[\"a\"]]\n[[],3]\n",
            "{name}"
        );
        assert_eq!(status, 0, "{name}");
    }
    Ok(())
}

#[test]
fn a_bom_at_the_front_of_a_named_pipe_is_skipped_7295() -> Result<()> {
    // The probe reads the BOM's three bytes, so they must not be read twice nor lost.
    for name in NAMES {
        let (out, _, status) = stream(name, b"\xef\xbb\xbf[1]")?;
        assert_eq!(out, "[[0],1]\n[[0]]\n", "{name}");
        assert_eq!(status, 0, "{name}");
    }
    Ok(())
}

#[test]
fn a_fault_in_a_named_pipe_follows_the_events_before_it_7295() -> Result<()> {
    for name in NAMES {
        let (out, err, status) = stream(name, b"[1,2")?;
        assert_eq!(out, "[[0],1]\n", "{name}");
        assert!(
            err.contains("Unfinished JSON term at EOF at line 1, column 4"),
            "{name}: {err}"
        );
        assert_eq!(status, 5, "{name}");
    }
    Ok(())
}

#[test]
fn an_empty_named_pipe_streams_nothing_7295() -> Result<()> {
    let (out, err, status) = stream("/dev/stdin", b"")?;
    assert_eq!((out.as_str(), err.as_str(), status), ("", "", 0));
    Ok(())
}

#[test]
fn a_regular_file_before_a_named_pipe_is_read_first_7295() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let first = dir.path().join("first.json");
    std::fs::write(&first, "[3]")?;
    let first = first.to_str().expect("utf-8 path");
    let (output, status) = spawn_jq_deadline(
        &["-c", "--stream", ".", first, "/dev/stdin"],
        Some(b"[1,2]"),
    )?;
    assert_eq!(
        String::from_utf8(output.stdout)?,
        "[[0],3]\n[[0]]\n[[0],1]\n[[1],2]\n[[1]]\n"
    );
    assert_eq!(status, 0);
    Ok(())
}

#[test]
fn a_named_pipe_past_the_probe_is_streamed_whole_7295() -> Result<()> {
    // Over 2 MB, so a reader that took only what the probe read would stop short.
    let mut bytes = b"[".to_vec();
    for i in 0..400_000 {
        if i > 0 {
            bytes.push(b',');
        }
        bytes.extend_from_slice(format!("{i}").as_bytes());
    }
    bytes.push(b']');
    assert!(bytes.len() > 2_000_000);
    let (output, status) = spawn_jq_deadline(
        &["-c", "--stream", "select(length == 2) | .[1]", "/dev/stdin"],
        Some(&bytes),
    )?;
    assert_eq!(status, 0);
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout.lines().count(), 400_000);
    assert_eq!(stdout.lines().last(), Some("399999"));
    Ok(())
}
