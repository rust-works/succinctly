//! #7200: `--seq` over a named source that is not a regular file (`/dev/stdin` on a pipe, a FIFO
//! path) keeps the bytes it read to decide the stream was not plain, so a byte-order mark, invalid
//! UTF-8 or a slurp is read from them and not from a second open of a stream that has gone. Each
//! row is the stream named `-`, which takes another route and was always right, and jq 1.7.1's own
//! output for the same bytes.

#![cfg(unix)]

use std::process::Command;
use std::time::Duration;

use anyhow::Result;

use crate::fifo_feed::{self, Feed};
use crate::{spawn_jq, succinctly_bin};

const STREAMS: [(&[u8], &str); 3] = [
    (b"\x1e\"\xff\"\n\x1e2\n", "invalid UTF-8"),
    (b"\xef\xbb\xbf\x1e1\n", "a byte-order mark"),
    (b"\x1e1\n\x1e[2\n", "a record cut short"),
];

#[test]
fn a_pipe_named_as_a_file_reads_as_the_stream_named_dash_does_7200() -> Result<()> {
    for flags in [&["-c"][..], &["-c", "-s"][..]] {
        for (bytes, what) in STREAMS {
            let mut named = vec!["--seq"];
            named.extend_from_slice(flags);
            named.push(".");
            let mut dash = named.clone();
            named.push("/dev/stdin");
            dash.push("-");
            let (by_name, name_status) = spawn_jq(&named, Some(bytes))?;
            let (by_dash, dash_status) = spawn_jq(&dash, Some(bytes))?;
            assert_eq!(
                by_name.stdout, by_dash.stdout,
                "{flags:?} over {what}: stdout"
            );
            assert_eq!(
                name_status, dash_status,
                "{flags:?} over {what}: exit status"
            );
            let err = String::from_utf8_lossy(&by_name.stderr).replace("/dev/stdin", "<stdin>");
            let dash_err = String::from_utf8_lossy(&by_dash.stderr);
            assert_eq!(err, dash_err, "{flags:?} over {what}: stderr");
        }
    }
    Ok(())
}

#[test]
fn a_pipe_named_as_a_file_keeps_its_values_7200() -> Result<()> {
    let (out, _) = spawn_jq(
        &["--seq", "-c", ".", "/dev/stdin"],
        Some(b"\xef\xbb\xbf\x1e1\n"),
    )?;
    assert_eq!(out.stdout, b"\x1e1\n");
    let (out, _) = spawn_jq(
        &["--seq", "-c", "-s", ".", "/dev/stdin"],
        Some(b"\x1e\"\xff\"\n\x1e2\n"),
    )?;
    assert_eq!(out.stdout, "\x1e[\"\u{fffd}\",2]\n".as_bytes());
    Ok(())
}

#[test]
fn a_fifo_path_with_a_byte_order_mark_does_not_hang_7200() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("succinctly-7200-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let fifo = dir.join("f");
    // One writer, which writes the bytes once: a second open of the path by the reader used to wait
    // forever, and with this writer reads an end of file, so the bytes are missing from the output.
    let feed = Feed::new(fifo.clone(), b"\xef\xbb\xbf\x1e1\n".to_vec());
    let out = fifo_feed::output_within(
        Command::new(succinctly_bin())
            .args(["jq", "--seq", "-c", "."])
            .arg(&fifo),
        std::slice::from_ref(&feed),
        Duration::from_secs(20),
    );
    drop(feed);
    let _ = std::fs::remove_dir_all(&dir);
    let out = out?;
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.stdout, b"\x1e1\n");
    Ok(())
}

#[test]
fn a_pipe_among_several_named_files_is_read_once_7200() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("succinctly-7200-multi-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("a.json");
    std::fs::write(&file, b"\x1e3\n")?;
    let file = file.to_string_lossy().into_owned();
    for flags in [&["-c"][..], &["-c", "-s"][..]] {
        for order in [["/dev/stdin", file.as_str()], [file.as_str(), "/dev/stdin"]] {
            let mut args = vec!["--seq"];
            args.extend_from_slice(flags);
            args.push(".");
            args.extend_from_slice(&order);
            let (out, status) = spawn_jq(&args, Some(b"\xef\xbb\xbf\x1e1\n"))?;
            let err = String::from_utf8_lossy(&out.stderr);
            assert_eq!(status, 0, "{args:?}: {err}");
            let want: &[u8] = if flags.contains(&"-s") {
                if order[0] == "/dev/stdin" {
                    b"\x1e[1,3]\n"
                } else {
                    b"\x1e[3,1]\n"
                }
            } else if order[0] == "/dev/stdin" {
                b"\x1e1\n\x1e3\n"
            } else {
                b"\x1e3\n\x1e1\n"
            };
            assert_eq!(out.stdout, want, "{args:?}");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
